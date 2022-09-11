use crate::coin_balance::CoinBalanceReportOps;
use crate::hd_wallet::{AddressDerivingResult, HDAccountOps, HDWalletCoinOps, HDWalletOps};
use crate::my_tx_history_v2::{CoinWithTxHistoryV2, DisplayAddress, MyTxHistoryErrorV2, MyTxHistoryTarget,
                              TxDetailsBuilder, TxHistoryStorage};
use crate::tx_history_storage::{GetTxHistoryFilters, WalletId};
use crate::utxo::rpc_clients::{electrum_script_hash, ElectrumClient, NativeClient, UtxoRpcClientEnum};
use crate::utxo::utxo_common::{big_decimal_from_sat, HISTORY_TOO_LARGE_ERROR};
use crate::utxo::utxo_tx_history_v2::{UtxoTxDetailsParams, UtxoTxHistoryOps};
use crate::utxo::{output_script, RequestTxHistoryResult, UtxoCoinFields, UtxoCommonOps, UtxoHDAccount};
use crate::{big_decimal_from_sat_unsigned, compare_transactions, BalanceResult, CoinWithDerivationMethod,
            DerivationMethod, HDAddressId, MarketCoinOps, TransactionDetails, TxFeeDetails, TxIdHeight,
            UtxoFeeDetails, UtxoTx};
use common::jsonrpc_client::JsonRpcErrorType;
use crypto::Bip44Chain;
use futures::compat::Future01CompatExt;
use itertools::Itertools;
use keys::{Address, Type as ScriptType};
use mm2_err_handle::prelude::*;
use mm2_metrics::MetricsArc;
use mm2_number::BigDecimal;
use rpc::v1::types::{TransactionInputEnum, H256 as H256Json};
use serialization::deserialize;
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::iter;

fn pass_through<T>(t: T) -> T { t }

/// [`CoinWithTxHistoryV2::history_wallet_id`] implementation.
pub fn history_wallet_id(coin: &UtxoCoinFields) -> WalletId { WalletId::new(coin.conf.ticker.clone()) }

/// [`CoinWithTxHistoryV2::get_tx_history_filters`] implementation.
/// Returns `GetTxHistoryFilters` according to the derivation method.
pub async fn get_tx_history_filters<Coin>(
    coin: &Coin,
    target: MyTxHistoryTarget,
) -> MmResult<GetTxHistoryFilters, MyTxHistoryErrorV2>
where
    Coin: CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet> + HDWalletCoinOps + MarketCoinOps,
    <Coin as HDWalletCoinOps>::Address: DisplayAddress,
{
    match (coin.derivation_method(), target) {
        (DerivationMethod::Iguana(_), MyTxHistoryTarget::Iguana) => {
            let my_address = coin.my_address().map_to_mm(MyTxHistoryErrorV2::Internal)?;
            Ok(GetTxHistoryFilters::for_address(my_address))
        },
        (DerivationMethod::Iguana(_), target) => {
            MmError::err(MyTxHistoryErrorV2::with_expected_target(target, "Iguana"))
        },
        (DerivationMethod::HDWallet(hd_wallet), MyTxHistoryTarget::AccountId { account_id }) => {
            get_tx_history_filters_for_hd_account(coin, hd_wallet, account_id).await
        },
        (DerivationMethod::HDWallet(hd_wallet), MyTxHistoryTarget::AddressId(hd_address_id)) => {
            get_tx_history_filters_for_hd_address(coin, hd_wallet, hd_address_id).await
        },
        (DerivationMethod::HDWallet(hd_wallet), MyTxHistoryTarget::AddressDerivationPath(derivation_path)) => {
            let hd_address_id = HDAddressId::from(derivation_path);
            get_tx_history_filters_for_hd_address(coin, hd_wallet, hd_address_id).await
        },
        (DerivationMethod::HDWallet(_), target) => MmError::err(MyTxHistoryErrorV2::with_expected_target(
            target,
            "an HD account/address",
        )),
    }
}

/// `get_tx_history_filters` function's helper.
async fn get_tx_history_filters_for_hd_account<Coin>(
    coin: &Coin,
    hd_wallet: &Coin::HDWallet,
    account_id: u32,
) -> MmResult<GetTxHistoryFilters, MyTxHistoryErrorV2>
where
    Coin: HDWalletCoinOps,
    Coin::Address: DisplayAddress,
{
    let hd_account = hd_wallet
        .get_account(account_id)
        .await
        .or_mm_err(|| MyTxHistoryErrorV2::InvalidTarget(format!("No such account_id={account_id}")))?;

    let external_addresses = coin.derive_known_addresses(&hd_account, Bip44Chain::External)?;
    let internal_addresses = coin.derive_known_addresses(&hd_account, Bip44Chain::Internal)?;

    let addresses_iter = external_addresses
        .into_iter()
        .chain(internal_addresses)
        .map(|hd_address| DisplayAddress::display_address(&hd_address.address));
    Ok(GetTxHistoryFilters::for_addresses(addresses_iter))
}

/// `get_tx_history_filters` function's helper.
async fn get_tx_history_filters_for_hd_address<Coin>(
    coin: &Coin,
    hd_wallet: &Coin::HDWallet,
    hd_address_id: HDAddressId,
) -> MmResult<GetTxHistoryFilters, MyTxHistoryErrorV2>
where
    Coin: HDWalletCoinOps,
    Coin::Address: DisplayAddress,
{
    let hd_account = hd_wallet
        .get_account(hd_address_id.account_id)
        .await
        .or_mm_err(|| MyTxHistoryErrorV2::InvalidTarget(format!("No such account_id={}", hd_address_id.account_id)))?;

    let is_address_activated = hd_account.is_address_activated(hd_address_id.chain, hd_address_id.address_id)?;
    if !is_address_activated {
        let error = format!(
            "'{:?}:{}' address is not activated",
            hd_address_id.chain, hd_address_id.address_id
        );
        return MmError::err(MyTxHistoryErrorV2::InvalidTarget(error));
    }

    let hd_address = coin.derive_address(&hd_account, hd_address_id.chain, hd_address_id.address_id)?;
    Ok(GetTxHistoryFilters::for_address(hd_address.address.display_address()))
}
/// [`UtxoTxHistoryOps::my_addresses`] implementation.
pub async fn my_addresses<Coin>(coin: &Coin) -> Result<HashSet<Address>, String>
where
    Coin: HDWalletCoinOps<Address = Address, HDAccount = UtxoHDAccount> + UtxoCommonOps,
{
    // TODO return a corresponding error instead of `String`.
    Ok(try_s!(get_tx_addresses_mapped(coin, pass_through).await))
}

/// [`UtxoTxHistoryOps::tx_details_by_hash`] implementation.
pub async fn tx_details_by_hash<Coin, Storage>(
    coin: &Coin,
    params: UtxoTxDetailsParams<'_, Storage>,
) -> Result<Vec<TransactionDetails>, String>
where
    Coin: UtxoTxHistoryOps + UtxoCommonOps + MarketCoinOps,
    Storage: TxHistoryStorage,
{
    let ticker = coin.ticker();
    let decimals = coin.as_ref().decimals;

    let verbose_tx = try_s!(
        coin.as_ref()
            .rpc_client
            .get_verbose_transaction(params.hash)
            .compat()
            .await
    );
    let tx: UtxoTx = try_s!(deserialize(verbose_tx.hex.as_slice()).map_err(|e| ERRL!("{:?}", e)));

    let mut tx_builder = TxDetailsBuilder::new(
        ticker.to_string(),
        &tx,
        params.block_height_and_time,
        params.my_addresses.clone(),
    );

    let mut input_amount = 0;
    let mut output_amount = 0;

    for input in tx.inputs.iter() {
        // input transaction is zero if the tx is the coinbase transaction
        if input.previous_output.hash.is_zero() {
            continue;
        }

        let prev_tx_hash: H256Json = input.previous_output.hash.reversed().into();

        let prev_tx = try_s!(coin.tx_from_storage_or_rpc(&prev_tx_hash, params.storage).await);

        let prev_output_index = input.previous_output.index as usize;
        let prev_tx_value = prev_tx.outputs[prev_output_index].value;
        let prev_script = prev_tx.outputs[prev_output_index].script_pubkey.clone().into();

        input_amount += prev_tx_value;
        let amount = big_decimal_from_sat_unsigned(prev_tx_value, decimals);

        let from: Vec<Address> = try_s!(coin.addresses_from_script(&prev_script));
        for address in from {
            tx_builder.transferred_from(address, &amount);
        }
    }

    for output in tx.outputs.iter() {
        let to = try_s!(coin.addresses_from_script(&output.script_pubkey.clone().into()));
        if to.is_empty() {
            continue;
        }

        output_amount += output.value;
        let amount = big_decimal_from_sat_unsigned(output.value, decimals);
        for address in to {
            tx_builder.transferred_to(address, &amount);
        }
    }

    let fee = if input_amount == 0 {
        let fee = verbose_tx.vin.iter().fold(0., |cur, input| {
            let fee = match input {
                TransactionInputEnum::Lelantus(lelantus) => lelantus.n_fees,
                _ => 0.,
            };
            cur + fee
        });
        try_s!(BigDecimal::try_from(fee))
    } else {
        let fee = input_amount as i64 - output_amount as i64;
        big_decimal_from_sat(fee, decimals)
    };

    let fee_details = UtxoFeeDetails {
        coin: Some(ticker.to_string()),
        amount: fee,
    };

    tx_builder.set_tx_fee(Some(TxFeeDetails::from(fee_details)));
    Ok(vec![tx_builder.build()])
}

/// [`UtxoTxHistoryOps::tx_from_storage_or_rpc`] implementation.
pub async fn tx_from_storage_or_rpc<Coin, Storage>(
    coin: &Coin,
    tx_hash: &H256Json,
    storage: &Storage,
) -> Result<UtxoTx, String>
where
    Coin: CoinWithTxHistoryV2 + UtxoCommonOps,
    Storage: TxHistoryStorage,
{
    let tx_hash_str = format!("{:02x}", tx_hash);
    let wallet_id = coin.history_wallet_id();
    let tx_bytes = match storage
        .tx_bytes_from_cache(&wallet_id, &tx_hash_str)
        .await
        .map_err(|e| ERRL!("{:?}", e))?
    {
        Some(tx_bytes) => tx_bytes,
        None => {
            let tx_bytes = try_s!(coin.as_ref().rpc_client.get_transaction_bytes(tx_hash).compat().await);
            storage
                .add_tx_to_cache(&wallet_id, &tx_hash_str, &tx_bytes)
                .await
                .map_err(|e| ERRL!("{:?}", e))?;
            tx_bytes
        },
    };
    let tx = try_s!(deserialize(tx_bytes.0.as_slice()));
    Ok(tx)
}

/// [`UtxoTxHistoryOps::get_addresses_balances`] implementation.
/// Requests balances of all activated addresses.
pub async fn get_addresses_balances<Coin>(coin: &Coin) -> BalanceResult<HashMap<String, BigDecimal>>
where
    Coin: CoinBalanceReportOps,
{
    let coin_balance = coin.coin_balance_report().await?;
    Ok(coin_balance.to_addresses_total_balances())
}

/// [`UtxoTxHistoryOps::request_tx_history`] implementation.
/// Requests transaction history according to the `DerivationMethod` and `UtxoRpcClientEnum`.
pub async fn request_tx_history_with_der_method<Coin>(coin: &Coin, metrics: MetricsArc) -> RequestTxHistoryResult
where
    Coin: HDWalletCoinOps<Address = Address, HDAccount = UtxoHDAccount> + UtxoCommonOps + MarketCoinOps,
{
    match coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(ref native) => request_tx_history_with_native(coin, native, metrics).await,
        UtxoRpcClientEnum::Electrum(ref electrum) => request_tx_history_with_electrum(coin, electrum, metrics).await,
    }
}

/// `request_tx_history_with_der_method` function's helper.
async fn request_tx_history_with_native<Coin>(
    coin: &Coin,
    native: &NativeClient,
    metrics: MetricsArc,
) -> RequestTxHistoryResult
where
    Coin: HDWalletCoinOps<Address = Address, HDAccount = UtxoHDAccount> + UtxoCommonOps + MarketCoinOps,
{
    let addresses: HashSet<String> =
        match get_tx_addresses_mapped(coin, |addr| DisplayAddress::display_address(&addr)).await {
            Ok(addresses) => addresses,
            Err(e) => return RequestTxHistoryResult::CriticalError(e.to_string()),
        };

    let ticker = coin.ticker();

    let mut from = 0;
    let mut all_transactions = vec![];
    loop {
        mm_counter!(metrics, "tx.history.request.count", 1,
            "coin" => ticker, "client" => "native", "method" => "listtransactions");

        let transactions = match native.list_transactions(100, from).compat().await {
            Ok(value) => value,
            Err(e) => {
                return RequestTxHistoryResult::Retry {
                    error: ERRL!("Error {} on list transactions", e),
                };
            },
        };

        mm_counter!(metrics, "tx.history.response.count", 1,
            "coin" => ticker, "client" => "native", "method" => "listtransactions");

        if transactions.is_empty() {
            break;
        }
        from += 100;
        all_transactions.extend(transactions);
    }

    mm_counter!(metrics, "tx.history.response.total_length", all_transactions.len() as u64,
        "coin" => ticker, "client" => "native", "method" => "listtransactions");

    let all_transactions = all_transactions
        .into_iter()
        .filter_map(|item| {
            if addresses.contains(&item.address) {
                Some((item.txid, item.blockindex))
            } else {
                None
            }
        })
        .collect();

    RequestTxHistoryResult::Ok(all_transactions)
}

/// `request_tx_history_with_der_method` function's helper.
async fn request_tx_history_with_electrum<Coin>(
    coin: &Coin,
    electrum: &ElectrumClient,
    metrics: MetricsArc,
) -> RequestTxHistoryResult
where
    Coin: HDWalletCoinOps<Address = Address, HDAccount = UtxoHDAccount> + UtxoCommonOps + MarketCoinOps,
{
    fn addr_to_script_hash(addr: Address) -> String {
        let script = output_script(&addr, ScriptType::P2PKH);
        let script_hash = electrum_script_hash(&script);
        hex::encode(script_hash)
    }

    let script_hashes: Vec<String> = match get_tx_addresses_mapped(coin, addr_to_script_hash).await {
        Ok(hashes) => hashes,
        Err(e) => return RequestTxHistoryResult::CriticalError(e.to_string()),
    };
    let script_hashes_count = script_hashes.len() as u64;

    let ticker = coin.ticker();

    mm_counter!(metrics, "tx.history.request.count", script_hashes_count,
        "coin" => ticker, "client" => "electrum", "method" => "blockchain.scripthash.get_history");

    let hashes_history = match electrum.scripthash_get_history_batch(script_hashes).compat().await {
        Ok(hashes_history) => hashes_history,
        Err(e) => match &e.error {
            JsonRpcErrorType::InvalidRequest(e) | JsonRpcErrorType::Transport(e) | JsonRpcErrorType::Parse(_, e) => {
                return RequestTxHistoryResult::Retry {
                    error: ERRL!("Error {} on scripthash_get_history", e),
                };
            },
            JsonRpcErrorType::Response(_addr, err) => {
                if HISTORY_TOO_LARGE_ERROR.eq(err) {
                    return RequestTxHistoryResult::HistoryTooLarge;
                } else {
                    return RequestTxHistoryResult::Retry {
                        error: ERRL!("Error {:?} on scripthash_get_history", e),
                    };
                }
            },
        },
    };

    let ordered_history: Vec<_> = hashes_history
        .into_iter()
        .flatten()
        .map(|item| {
            let height = if item.height < 0 { 0 } else { item.height as u64 };
            (item.tx_hash, height)
        })
        // We need to order transactions by their height and TX hash.
        .sorted_by(|(tx_hash_left, height_left), (tx_hash_right, height_right)| {
            let left = TxIdHeight::new(*height_left, tx_hash_left);
            let right = TxIdHeight::new(*height_right, tx_hash_right);
            compare_transactions(left, right)
        })
        .collect();

    mm_counter!(metrics, "tx.history.response.count", script_hashes_count,
        "coin" => ticker, "client" => "electrum", "method" => "blockchain.scripthash.get_history");

    mm_counter!(metrics, "tx.history.response.total_length", ordered_history.len() as u64,
        "coin" => ticker, "client" => "electrum", "method" => "blockchain.scripthash.get_history");

    RequestTxHistoryResult::Ok(ordered_history)
}

/// `my_addresses`, `request_tx_history_with_native` and `request_tx_history_with_electrum` functions' helper.
/// This function is optimized to get all addresses mapped before they're collected into `R`.
/// - `R` is a result collection of the addresses.
/// - `F` is a function that maps `Address` into an `Item`.
/// - `Item` is a item type of the `R` result collection.
async fn get_tx_addresses_mapped<Coin, R, F, Item>(coin: &Coin, mut item_fn: F) -> AddressDerivingResult<R>
where
    Coin: HDWalletCoinOps<Address = Address, HDAccount = UtxoHDAccount> + UtxoCommonOps,
    R: Default + Extend<Item>,
    F: FnMut(Address) -> Item,
{
    let mut all_addresses = R::default();

    match coin.as_ref().derivation_method {
        DerivationMethod::Iguana(ref my_address) => all_addresses.extend(iter::once(item_fn(my_address.clone()))),
        DerivationMethod::HDWallet(ref hd_wallet) => {
            let hd_accounts = hd_wallet.get_accounts().await;

            for (_, hd_account) in hd_accounts {
                let external_addresses = coin.derive_known_addresses(&hd_account, Bip44Chain::External)?;
                let internal_addresses = coin.derive_known_addresses(&hd_account, Bip44Chain::Internal)?;

                let addresses_it = external_addresses
                    .into_iter()
                    .chain(internal_addresses)
                    .map(|hd_address| item_fn(hd_address.address));
                all_addresses.extend(addresses_it);
            }
        },
    }

    Ok(all_addresses)
}

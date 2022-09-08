use crate::hd_wallet::{HDAccountOps, HDWalletCoinOps, HDWalletOps};
use crate::my_tx_history_v2::{DisplayAddress, MyTxHistoryErrorV2, MyTxHistoryTarget};
use crate::tx_history_storage::{GetTxHistoryFilters, WalletId};
use crate::utxo::UtxoCoinFields;
use crate::{CoinWithDerivationMethod, DerivationMethod, HDAddressId, MarketCoinOps};
use crypto::Bip44Chain;
use mm2_err_handle::prelude::*;

pub fn history_wallet_id(coin: &UtxoCoinFields) -> WalletId { WalletId::new(coin.conf.ticker.clone()) }

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
    let internal_addresses = coin.derive_known_addresses(&hd_account, Bip44Chain::External)?;

    let addresses_iter = external_addresses
        .into_iter()
        .chain(internal_addresses)
        .map(|hd_address| DisplayAddress::display_address(&hd_address.address));
    Ok(GetTxHistoryFilters::for_addresses(addresses_iter))
}

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

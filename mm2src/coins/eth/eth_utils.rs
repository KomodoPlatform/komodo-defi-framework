use super::*;
use crate::{coin_conf, FeeApproxStage, NumConversError, NumConversResult, RpcClientType};
use bitcoin_hashes::hex::ToHex;
use ethabi::{Function, Token};
use ethereum_types::{Address, FromDecStrErr, U256};
use ethkey::{public_to_address, Public};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MapToMmResult;
use mm2_number::{BigDecimal, MmNumber};
use secp256k1::PublicKey;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;
use sha3::{Digest, Keccak256};
use std::collections::HashMap;

/// Coin config parameter name for the max supported eth transaction type
pub(super) const MAX_ETH_TX_TYPE_SUPPORTED: &str = "max_eth_tx_type";
/// Coin config parameter name for the eth gas price adjustment values
pub(super) const GAS_PRICE_ADJUST: &str = "gas_price_adjust";
/// Coin config parameter name for the eth estimate gas multiplier
pub(super) const ESTIMATE_GAS_MULT: &str = "estimate_gas_mult";
/// Coin config parameter name for the default eth swap gas fee policy
pub(super) const SWAP_GAS_FEE_POLICY: &str = "swap_gas_fee_policy";

pub(crate) mod nonce_sequencer {
    use super::*;

    type PerNetNonceLocksMap = Arc<AsyncMutex<HashMap<Address, Arc<AsyncMutex<()>>>>>;

    /// TODO: better to use ChainSpec instead of ticker
    type AllNetsNonceLocks = Mutex<HashMap<String, PerNetNonceLocks>>;

    // We can use a nonce lock shared between tokens using the same platform coin and the platform itself.
    // For example, ETH/USDT-ERC20 should use the same lock, but it will be different for BNB/USDT-BEP20.
    // This lock is used to ensure that only one transaction is sent at a time per address.
    lazy_static! {
        static ref ALL_NETS_NONCE_LOCKS: AllNetsNonceLocks = Mutex::new(HashMap::new());
    }

    #[derive(Clone)]
    pub(crate) struct PerNetNonceLocks {
        locks: PerNetNonceLocksMap,
    }

    impl PerNetNonceLocks {
        fn new_nonce_lock() -> PerNetNonceLocks {
            Self {
                locks: Arc::new(AsyncMutex::new(HashMap::new())),
            }
        }

        pub(crate) fn get_net_locks(platform_ticker: String) -> Self {
            let mut networks = ALL_NETS_NONCE_LOCKS.lock().unwrap();
            networks
                .entry(platform_ticker)
                .or_insert_with(Self::new_nonce_lock)
                .clone()
        }

        pub(crate) async fn get_adddress_lock(&self, address: Address) -> Arc<AsyncMutex<()>> {
            let mut locks = self.locks.lock().await;
            locks
                .entry(address)
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        }
    }
}

/// get tx type from pay_for_gas_option
/// currently only type2 and legacy supported
/// if for Eth Classic we also want support for type 1 then use a fn
#[macro_export]
macro_rules! tx_type_from_pay_for_gas_option {
    ($pay_for_gas_option: expr) => {
        if matches!($pay_for_gas_option, PayForGasOption::Eip1559 { .. }) {
            ethcore_transaction::TxType::Type2
        } else {
            ethcore_transaction::TxType::Legacy
        }
    };
}

pub(crate) fn get_function_input_data(decoded: &[Token], func: &Function, index: usize) -> Result<Token, String> {
    decoded.get(index).cloned().ok_or(format!(
        "Missing input in function {}: No input found at index {}",
        func.name.clone(),
        index
    ))
}

pub(crate) fn get_function_name(name: &str, watcher_reward: bool) -> String {
    if watcher_reward {
        format!("{}{}", name, "Reward")
    } else {
        name.to_owned()
    }
}

pub fn addr_from_raw_pubkey(pubkey: &[u8]) -> Result<Address, String> {
    let pubkey = try_s!(PublicKey::from_slice(pubkey).map_err(|e| ERRL!("{:?}", e)));
    let eth_public = Public::from_slice(&pubkey.serialize_uncompressed()[1..65]);
    Ok(public_to_address(&eth_public))
}

pub fn addr_from_pubkey_str(pubkey: &str) -> Result<String, String> {
    let pubkey_bytes = try_s!(hex::decode(pubkey));
    let addr = try_s!(addr_from_raw_pubkey(&pubkey_bytes));
    Ok(format!("{addr:#02x}"))
}

pub(crate) fn display_u256_with_decimal_point(number: U256, decimals: u8) -> String {
    let mut string = number.to_string();
    let decimals = decimals as usize;
    if string.len() <= decimals {
        string.insert_str(0, &"0".repeat(decimals - string.len() + 1));
    }

    string.insert(string.len() - decimals, '.');
    string.trim_end_matches('0').into()
}

/// Converts 'number' to value with decimal point and shifts it left by 'decimals' places
pub fn u256_to_big_decimal(number: U256, decimals: u8) -> NumConversResult<BigDecimal> {
    let string = display_u256_with_decimal_point(number, decimals);
    Ok(string.parse::<BigDecimal>()?)
}

/// Shifts 'number' with decimal point right by 'decimals' places and converts it to U256 value
pub fn u256_from_big_decimal(amount: &BigDecimal, decimals: u8) -> NumConversResult<U256> {
    let mut amount = amount.to_string();
    let dot = amount.find('.');
    let decimals = decimals as usize;
    if let Some(index) = dot {
        let mut fractional = amount.split_off(index);
        // remove the dot from fractional part
        fractional.remove(0);
        if fractional.len() < decimals {
            fractional.insert_str(fractional.len(), &"0".repeat(decimals - fractional.len()));
        }
        fractional.truncate(decimals);
        amount.push_str(&fractional);
    } else {
        amount.insert_str(amount.len(), &"0".repeat(decimals));
    }
    U256::from_dec_str(&amount).map_to_mm(|e| NumConversError::new(format!("{e:?}")))
}

/// Converts BigDecimal gwei value to wei value as U256
#[inline(always)]
pub fn wei_from_gwei_decimal(bigdec: &BigDecimal) -> NumConversResult<U256> {
    u256_from_big_decimal(bigdec, ETH_GWEI_DECIMALS)
}

/// Converts a U256 wei value to an gwei value as a BigDecimal
#[inline(always)]
pub fn wei_to_gwei_decimal(wei: U256) -> NumConversResult<BigDecimal> {
    u256_to_big_decimal(wei, ETH_GWEI_DECIMALS)
}

/// Converts a U256 wei value to an ETH value as a BigDecimal
/// TODO: use wei_to_eth_decimal instead of u256_to_big_decimal(gas_cost_wei, ETH_DECIMALS)
#[inline(always)]
pub fn wei_to_eth_decimal(wei: U256) -> NumConversResult<BigDecimal> {
    u256_to_big_decimal(wei, ETH_DECIMALS)
}

#[inline]
pub fn mm_number_to_u256(mm_number: &MmNumber) -> Result<U256, FromDecStrErr> {
    U256::from_dec_str(mm_number.to_ratio().to_integer().to_string().as_str())
}

#[inline]
pub fn mm_number_from_u256(u256: U256) -> MmNumber {
    MmNumber::from(u256.to_string().as_str())
}

#[inline]
pub fn wei_from_coins_mm_number(mm_number: &MmNumber, decimals: u8) -> NumConversResult<U256> {
    u256_from_big_decimal(&mm_number.to_decimal(), decimals)
}

#[inline]
#[allow(unused)]
pub fn wei_to_coins_mm_number(u256: U256, decimals: u8) -> NumConversResult<MmNumber> {
    Ok(MmNumber::from(u256_to_big_decimal(u256, decimals)?))
}

pub(super) fn get_conf_param_or_from_plaform_coin<T: DeserializeOwned>(
    ctx: &MmArc,
    conf: &Json,
    coin_type: &EthCoinType,
    param_name: &str,
) -> Result<Option<T>, String> {
    /// Get "max_eth_tx_type" param from a token conf, or from the platform coin conf
    fn read_conf_param_or_from_plaform(ctx: &MmArc, conf: &Json, param: &str, coin_type: &EthCoinType) -> Option<Json> {
        match &coin_type {
            EthCoinType::Eth => conf.get(param).cloned(),
            EthCoinType::Erc20 { platform, .. } | EthCoinType::Nft { platform } => conf
                .get(param)
                .cloned()
                .or(coin_conf(ctx, platform).get(param).cloned()),
        }
    }

    match read_conf_param_or_from_plaform(ctx, conf, param_name, coin_type) {
        Some(val) => {
            let param_val: T = serde_json::from_value(val).map_err(|_| format!("{param_name} in coins is invalid"))?;
            Ok(Some(param_val))
        },
        None => Ok(None),
    }
}

pub(crate) fn signed_tx_from_web3_tx(transaction: Web3Transaction) -> Result<SignedEthTx, String> {
    // Local function to map the access list
    fn map_access_list(web3_access_list: &Option<Vec<web3::types::AccessListItem>>) -> ethcore_transaction::AccessList {
        match web3_access_list {
            Some(list) => ethcore_transaction::AccessList(
                list.iter()
                    .map(|item| ethcore_transaction::AccessListItem {
                        address: item.address,
                        storage_keys: item.storage_keys.clone(),
                    })
                    .collect(),
            ),
            None => ethcore_transaction::AccessList(vec![]),
        }
    }

    // Define transaction types
    let type_0: ethereum_types::U64 = 0.into();
    let type_1: ethereum_types::U64 = 1.into();
    let type_2: ethereum_types::U64 = 2.into();

    // Determine the transaction type
    let tx_type = match transaction.transaction_type {
        None => TxType::Legacy,
        Some(t) if t == type_0 => TxType::Legacy,
        Some(t) if t == type_1 => TxType::Type1,
        Some(t) if t == type_2 => TxType::Type2,
        _ => return Err(ERRL!("'Transaction::transaction_type' unsupported")),
    };

    // Determine the action based on the presence of 'to' field
    let action = match transaction.to {
        Some(addr) => Action::Call(addr),
        None => Action::Create,
    };

    // Initialize the transaction builder
    let tx_builder = UnSignedEthTxBuilder::new(
        tx_type.clone(),
        transaction.nonce,
        transaction.gas,
        action,
        transaction.value,
        transaction.input.0,
    );

    // Modify the builder based on the transaction type
    let tx_builder = match tx_type {
        TxType::Legacy => {
            let gas_price = transaction
                .gas_price
                .ok_or_else(|| ERRL!("'Transaction::gas_price' is not set"))?;
            tx_builder.with_gas_price(gas_price)
        },
        TxType::Type1 => {
            let gas_price = transaction
                .gas_price
                .ok_or_else(|| ERRL!("'Transaction::gas_price' is not set"))?;
            let chain_id = transaction
                .chain_id
                .ok_or_else(|| ERRL!("'Transaction::chain_id' is not set"))?
                .to_string()
                .parse()
                .map_err(|e: std::num::ParseIntError| e.to_string())?;
            tx_builder
                .with_gas_price(gas_price)
                .with_chain_id(chain_id)
                .with_access_list(map_access_list(&transaction.access_list))
        },
        TxType::Type2 => {
            let max_fee_per_gas = transaction
                .max_fee_per_gas
                .ok_or_else(|| ERRL!("'Transaction::max_fee_per_gas' is not set"))?;
            let max_priority_fee_per_gas = transaction
                .max_priority_fee_per_gas
                .ok_or_else(|| ERRL!("'Transaction::max_priority_fee_per_gas' is not set"))?;
            let chain_id = transaction
                .chain_id
                .ok_or_else(|| ERRL!("'Transaction::chain_id' is not set"))?
                .to_string()
                .parse()
                .map_err(|e: std::num::ParseIntError| e.to_string())?;
            tx_builder
                .with_priority_fee_per_gas(max_fee_per_gas, max_priority_fee_per_gas)
                .with_chain_id(chain_id)
                .with_access_list(map_access_list(&transaction.access_list))
        },
        TxType::Invalid => return Err(ERRL!("Internal error: 'tx_type' invalid")),
    };

    // Build the unsigned transaction
    let unsigned = tx_builder.build().map_err(|err| err.to_string())?;

    // Extract signature components
    let r = transaction.r.ok_or_else(|| ERRL!("'Transaction::r' is not set"))?;
    let s = transaction.s.ok_or_else(|| ERRL!("'Transaction::s' is not set"))?;
    let v = transaction
        .v
        .ok_or_else(|| ERRL!("'Transaction::v' is not set"))?
        .as_u64();

    // Create the signed transaction
    let unverified = match unsigned {
        TransactionWrapper::Legacy(unsigned) => UnverifiedTransactionWrapper::Legacy(
            UnverifiedLegacyTransaction::new_with_network_v(unsigned, r, s, v, transaction.hash)
                .map_err(|err| ERRL!("'Transaction::new' error {}", err.to_string()))?,
        ),
        TransactionWrapper::Eip2930(unsigned) => UnverifiedTransactionWrapper::Eip2930(
            UnverifiedEip2930Transaction::new(unsigned, r, s, v, transaction.hash)
                .map_err(|err| ERRL!("'Transaction::new' error {}", err.to_string()))?,
        ),
        TransactionWrapper::Eip1559(unsigned) => UnverifiedTransactionWrapper::Eip1559(
            UnverifiedEip1559Transaction::new(unsigned, r, s, v, transaction.hash)
                .map_err(|err| ERRL!("'Transaction::new' error {}", err.to_string()))?,
        ),
    };

    // Return the signed transaction
    Ok(try_s!(SignedEthTx::new(unverified)))
}

pub fn valid_addr_from_str(addr_str: &str) -> Result<Address, String> {
    let addr = try_s!(addr_from_str(addr_str));
    if !is_valid_checksum_addr(addr_str) {
        return ERR!("Invalid address checksum");
    }
    Ok(addr)
}

pub fn addr_from_str(addr_str: &str) -> Result<Address, String> {
    if !addr_str.starts_with("0x") {
        return ERR!("Address must be prefixed with 0x");
    };

    Ok(try_s!(Address::from_str(&addr_str[2..])))
}

/// This function fixes a bug appeared on `ethabi` update:
/// 1. `ethabi(6.1.0)::Function::decode_input` had
/// ```rust
/// decode(&self.input_param_types(), &data[4..])
/// ```
///
/// 2. `ethabi(17.2.0)::Function::decode_input` has
/// ```rust
/// decode(&self.input_param_types(), data)
/// ```
pub fn decode_contract_call(function: &Function, contract_call_bytes: &[u8]) -> Result<Vec<Token>, ethabi::Error> {
    if contract_call_bytes.len() < 4 {
        return Err(ethabi::Error::Other(
            "Contract call should contain at least 4 bytes known as a function signature".into(),
        ));
    }

    let actual_signature = &contract_call_bytes[..4];
    let expected_signature = &function.short_signature();
    if actual_signature != expected_signature {
        let error =
            format!("Unexpected contract call signature: expected {expected_signature:?}, found {actual_signature:?}");
        return Err(ethabi::Error::Other(error.into()));
    }

    function.decode_input(&contract_call_bytes[4..])
}

pub(super) fn rpc_event_handlers_for_eth_transport(ctx: &MmArc, ticker: String) -> Vec<RpcTransportEventHandlerShared> {
    let metrics = ctx.metrics.weak();
    vec![CoinTransportMetrics::new(metrics, ticker, RpcClientType::Ethereum).into_shared()]
}

/// Displays the address in mixed-case checksum form
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md
pub fn checksum_address(addr: &str) -> String {
    let mut addr = addr.to_lowercase();
    if addr.starts_with("0x") {
        addr.replace_range(..2, "");
    }

    let mut hasher = Keccak256::default();
    hasher.update(&addr);
    let hash = hasher.finalize();
    let mut result: String = "0x".into();
    for (i, c) in addr.chars().enumerate() {
        if c.is_ascii_digit() {
            result.push(c);
        } else {
            // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md#specification
            // Convert the address to hex, but if the ith digit is a letter (ie. it's one of abcdef)
            // print it in uppercase if the 4*ith bit of the hash of the lowercase hexadecimal
            // address is 1 otherwise print it in lowercase.
            if hash[i / 2] & (1 << (7 - 4 * (i % 2))) != 0 {
                result.push(c.to_ascii_uppercase());
            } else {
                result.push(c.to_ascii_lowercase());
            }
        }
    }

    result
}

/// `eth_addr_to_hex` converts Address to hex format.
/// Note: the result will be in lowercase.
pub(super) fn eth_addr_to_hex(address: &Address) -> String {
    format!("{address:#x}")
}

/// Checks that input is valid mixed-case checksum form address
/// The input must be 0x prefixed hex string
pub(super) fn is_valid_checksum_addr(addr: &str) -> bool {
    addr == checksum_address(addr)
}

pub(super) fn increase_by_percent(num: U256, percent: u64) -> U256 {
    num + (num / U256::from(100)) * U256::from(percent)
}

pub(super) fn increase_gas_price_by_stage(
    pay_for_gas_option: PayForGasOption,
    level: &FeeApproxStage,
) -> PayForGasOption {
    fn increase_value_by_stage(value: U256, level: &FeeApproxStage) -> U256 {
        match level {
            FeeApproxStage::WithoutApprox => value,
            FeeApproxStage::StartSwap => increase_by_percent(value, GAS_PRICE_APPROXIMATION_PERCENT_ON_START_SWAP),
            FeeApproxStage::OrderIssue | FeeApproxStage::OrderIssueMax => {
                increase_by_percent(value, GAS_PRICE_APPROXIMATION_PERCENT_ON_ORDER_ISSUE)
            },
            FeeApproxStage::TradePreimage | FeeApproxStage::TradePreimageMax => {
                increase_by_percent(value, GAS_PRICE_APPROXIMATION_PERCENT_ON_TRADE_PREIMAGE)
            },
            FeeApproxStage::WatcherPreimage => {
                increase_by_percent(value, GAS_PRICE_APPROXIMATION_PERCENT_ON_WATCHER_PREIMAGE)
            },
        }
    }

    match pay_for_gas_option {
        PayForGasOption::Legacy { gas_price } => PayForGasOption::Legacy {
            gas_price: increase_value_by_stage(gas_price, level),
        },
        PayForGasOption::Eip1559 {
            max_fee_per_gas,
            max_priority_fee_per_gas,
        } => PayForGasOption::Eip1559 {
            max_fee_per_gas: increase_value_by_stage(max_fee_per_gas, level),
            max_priority_fee_per_gas,
        },
    }
}

pub fn signed_eth_tx_from_bytes(bytes: &[u8]) -> Result<SignedEthTx, String> {
    let tx: UnverifiedTransactionWrapper = try_s!(rlp::decode(bytes));
    let signed = try_s!(SignedEthTx::new(tx));
    Ok(signed)
}

// Todo: `get_eth_address` should be removed since NFT is now part of the coins ctx.
/// `get_eth_address` returns wallet address for coin with `ETH` protocol type.
/// Note: result address has mixed-case checksum form.
pub async fn get_eth_address(
    ctx: &MmArc,
    conf: &Json,
    ticker: &str,
    path_to_address: &HDPathAccountToAddressId,
) -> MmResult<MyWalletAddress, GetEthAddressError> {
    let crypto_ctx = CryptoCtx::from_ctx(ctx).map_mm_err()?;
    let priv_key_policy = if crypto_ctx.hw_ctx().is_some() {
        PrivKeyBuildPolicy::Trezor
    } else {
        PrivKeyBuildPolicy::detect_priv_key_policy(ctx).map_mm_err()?
    }
    .into();

    let (_, derivation_method) =
        build_address_and_priv_key_policy(ctx, ticker, conf, priv_key_policy, path_to_address, None, None)
            .await
            .map_mm_err()?;
    let my_address = derivation_method.single_addr_or_err().await.map_mm_err()?;

    Ok(MyWalletAddress {
        coin: ticker.to_owned(),
        wallet_address: my_address.display_address(),
    })
}

/// Validates Ethereum addresses for NFT withdrawal.
/// Returns a tuple of valid `to` address, `token` address, and `EthCoin` instance on success.
/// Errors if the coin doesn't support NFT withdrawal or if the addresses are invalid.
pub(super) fn get_valid_nft_addr_to_withdraw(
    coin_enum: MmCoinEnum,
    to: &str,
    token_add: &str,
) -> MmResult<(Address, Address, EthCoin), GetValidEthWithdrawAddError> {
    let eth_coin = match coin_enum {
        MmCoinEnum::EthCoin(eth_coin) => eth_coin,
        _ => {
            return MmError::err(GetValidEthWithdrawAddError::CoinDoesntSupportNftWithdraw {
                coin: coin_enum.ticker().to_owned(),
            })
        },
    };
    let to_addr = valid_addr_from_str(to).map_err(GetValidEthWithdrawAddError::InvalidAddress)?;
    let token_addr = addr_from_str(token_add).map_err(GetValidEthWithdrawAddError::InvalidAddress)?;
    Ok((to_addr, token_addr, eth_coin))
}

pub fn parse_fee_cap_error(message: &str) -> Option<(U256, U256)> {
    let re = Regex::new(r"gasfeecap: (\d+)\s+basefee: (\d+)").ok()?;
    let caps = re.captures(message)?;

    let user_cap_str = caps.get(1)?.as_str();
    let required_base_str = caps.get(2)?.as_str();

    let user_cap = U256::from_dec_str(user_cap_str).ok()?;
    let required_base = U256::from_dec_str(required_base_str).ok()?;

    Some((user_cap, required_base))
}

pub(super) async fn get_eth_gas_details_from_withdraw_fee(
    eth_coin: &EthCoin,
    fee: Option<WithdrawFee>,
    eth_value: U256,
    data: Bytes,
    sender_address: Address,
    call_addr: Address,
    fungible_max: bool,
) -> MmResult<GasDetails, EthGasDetailsErr> {
    let pay_for_gas_option = match fee {
        Some(WithdrawFee::EthGas { gas_price, gas }) => {
            let gas_price = u256_from_big_decimal(&gas_price, ETH_GWEI_DECIMALS).map_mm_err()?;
            return Ok((gas.into(), PayForGasOption::Legacy { gas_price }));
        },
        Some(WithdrawFee::EthGasEip1559 {
            max_fee_per_gas,
            max_priority_fee_per_gas,
            gas_option: gas_limit,
        }) => {
            let max_fee_per_gas = u256_from_big_decimal(&max_fee_per_gas, ETH_GWEI_DECIMALS).map_mm_err()?;
            let max_priority_fee_per_gas =
                u256_from_big_decimal(&max_priority_fee_per_gas, ETH_GWEI_DECIMALS).map_mm_err()?;
            match gas_limit {
                EthGasLimitOption::Set(gas) => {
                    return Ok((
                        gas.into(),
                        PayForGasOption::Eip1559 {
                            max_fee_per_gas,
                            max_priority_fee_per_gas,
                        },
                    ))
                },
                EthGasLimitOption::Calc =>
                // go to gas estimate code
                {
                    PayForGasOption::Eip1559 {
                        max_fee_per_gas,
                        max_priority_fee_per_gas,
                    }
                },
            }
        },
        Some(fee_policy) => {
            let error = format!("Expected 'EthGas' fee type, found {fee_policy:?}");
            return MmError::err(EthGasDetailsErr::InvalidFeePolicy(error));
        },
        None => {
            // If WithdrawFee not set use legacy gas price (?)
            let gas_price = eth_coin.get_gas_price().await.map_mm_err()?;
            // go to gas estimate code
            PayForGasOption::Legacy { gas_price }
        },
    };

    // covering edge case by deducting the standard transfer fee when we want to max withdraw ETH
    let eth_value_for_estimate = if fungible_max && eth_coin.coin_type == EthCoinType::Eth {
        let estimated_fee =
            calc_total_fee(U256::from(eth_coin.gas_limit.eth_send_coins), &pay_for_gas_option).map_mm_err()?;
        // Defaulting to zero is safe; if the balance is indeed too low, the `estimate_gas` call below
        // will fail, and we will catch and handle that error gracefully.
        eth_value.checked_sub(estimated_fee).unwrap_or_default()
    } else {
        eth_value
    };

    let gas_price = pay_for_gas_option.get_gas_price();
    let (max_fee_per_gas, max_priority_fee_per_gas) = pay_for_gas_option.get_fee_per_gas();
    let estimate_gas_req = CallRequest {
        value: Some(eth_value_for_estimate),
        data: Some(data),
        from: Some(sender_address),
        to: Some(call_addr),
        gas: None,
        // gas price must be supplied because some smart contracts base their
        // logic on gas price, e.g. TUSD: https://github.com/KomodoPlatform/atomicDEX-API/issues/643
        gas_price,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        ..CallRequest::default()
    };
    let gas_limit = match eth_coin.estimate_gas_wrapper(estimate_gas_req).compat().await {
        Ok(gas_limit) => gas_limit,
        Err(e) => {
            let error_str = e.to_string().to_lowercase();
            if error_str.contains("insufficient funds") || error_str.contains("exceeds allowance") {
                let standard_tx_fee =
                    calc_total_fee(U256::from(eth_coin.gas_limit.eth_send_coins), &pay_for_gas_option).map_mm_err()?;
                let threshold = u256_to_big_decimal(standard_tx_fee, eth_coin.decimals).map_mm_err()?;
                let amount = u256_to_big_decimal(eth_value, eth_coin.decimals).map_mm_err()?;

                return MmError::err(EthGasDetailsErr::AmountTooLow { amount, threshold });
            } else if error_str.contains("fee cap less than block base fee")
                || error_str.contains("max fee per gas less than block base fee")
            {
                if let Some((user_cap, required_base)) = parse_fee_cap_error(&error_str) {
                    // The RPC error gives fee values in wei. Convert to Gwei (9 decimals) for the user.
                    let provided_fee_cap = u256_to_big_decimal(user_cap, ETH_GWEI_DECIMALS).map_mm_err()?;
                    let required_base_fee = u256_to_big_decimal(required_base, ETH_GWEI_DECIMALS).map_mm_err()?;
                    return MmError::err(EthGasDetailsErr::GasFeeCapTooLow {
                        provided_fee_cap,
                        required_base_fee,
                    });
                } else {
                    return MmError::err(EthGasDetailsErr::GasFeeCapBelowBaseFee);
                }
            }
            // This can be a transport error or a non-standard insufficient funds error.
            // In the latter case,
            // we can add to the above error handling of insufficient funds on a case-by-case basis.
            return MmError::err(EthGasDetailsErr::Transport(e.to_string()));
        },
    };

    Ok((gas_limit, pay_for_gas_option))
}

/// Calc estimated total gas fee or price
pub(super) fn calc_total_fee(gas: U256, pay_for_gas_option: &PayForGasOption) -> NumConversResult<U256> {
    match *pay_for_gas_option {
        PayForGasOption::Legacy { gas_price } => gas
            .checked_mul(gas_price)
            .or_mm_err(|| NumConversError("total fee overflow".into())),
        PayForGasOption::Eip1559 { max_fee_per_gas, .. } => gas
            .checked_mul(max_fee_per_gas)
            .or_mm_err(|| NumConversError("total fee overflow".into())),
    }
}

// Todo: Tron have a different concept from gas (Energy, Bandwidth and Free Transaction), it should be added as a different function
// and this should be part of a trait abstracted over both types
#[allow(clippy::result_large_err)]
pub(super) fn tx_builder_with_pay_for_gas_option(
    eth_coin: &EthCoin,
    tx_builder: UnSignedEthTxBuilder,
    pay_for_gas_option: &PayForGasOption,
) -> MmResult<UnSignedEthTxBuilder, WithdrawError> {
    let tx_builder = match *pay_for_gas_option {
        PayForGasOption::Legacy { gas_price } => tx_builder.with_gas_price(gas_price),
        PayForGasOption::Eip1559 {
            max_priority_fee_per_gas,
            max_fee_per_gas,
        } => {
            let chain_id = eth_coin
                .chain_id()
                .ok_or_else(|| WithdrawError::InternalError("chain_id should be set for an EVM coin".to_string()))?;
            tx_builder
                .with_priority_fee_per_gas(max_fee_per_gas, max_priority_fee_per_gas)
                .with_chain_id(chain_id)
        },
    };
    Ok(tx_builder)
}

/// convert fee policy for gas estimate requests
pub(super) fn get_swap_fee_policy_for_estimate(swap_fee_policy: SwapGasFeePolicy) -> SwapGasFeePolicy {
    match swap_fee_policy {
        SwapGasFeePolicy::Legacy => SwapGasFeePolicy::Legacy,
        // always use 'high' for estimate to avoid max_fee_per_gas less than base_fee errors:
        SwapGasFeePolicy::Low | SwapGasFeePolicy::Medium | SwapGasFeePolicy::High => SwapGasFeePolicy::High,
    }
}

pub(super) fn call_request_with_pay_for_gas_option(
    call_request: CallRequest,
    pay_for_gas_option: PayForGasOption,
) -> CallRequest {
    match pay_for_gas_option {
        PayForGasOption::Legacy { gas_price } => CallRequest {
            gas_price: Some(gas_price),
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            ..call_request
        },
        PayForGasOption::Eip1559 {
            max_fee_per_gas,
            max_priority_fee_per_gas,
        } => CallRequest {
            gas_price: None,
            max_fee_per_gas: Some(max_fee_per_gas),
            max_priority_fee_per_gas: Some(max_priority_fee_per_gas),
            ..call_request
        },
    }
}

pub(super) fn validate_fee_impl(coin: EthCoin, validate_fee_args: EthValidateFeeArgs<'_>) -> ValidatePaymentFut<()> {
    let fee_tx_hash = validate_fee_args.fee_tx_hash.to_owned();
    let sender_addr = try_f!(
        addr_from_raw_pubkey(validate_fee_args.expected_sender).map_to_mm(ValidatePaymentError::InvalidParameter)
    );
    let fee_addr = try_f!(addr_from_raw_pubkey(coin.dex_pubkey()).map_to_mm(ValidatePaymentError::InvalidParameter));
    let amount = validate_fee_args.amount.clone();
    let min_block_number = validate_fee_args.min_block_number;

    let fut = async move {
        let expected_value = u256_from_big_decimal(&amount, coin.decimals).map_mm_err()?;
        let tx_from_rpc = coin.transaction(TransactionId::Hash(fee_tx_hash)).await?;

        let tx_from_rpc = tx_from_rpc.as_ref().ok_or_else(|| {
            ValidatePaymentError::TxDoesNotExist(format!("Didn't find provided tx {fee_tx_hash:?} on ETH node"))
        })?;

        if tx_from_rpc.from != Some(sender_addr) {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                "{INVALID_SENDER_ERR_LOG}: Fee tx {tx_from_rpc:?} was sent from wrong address, expected {sender_addr:?}"
            )));
        }

        if let Some(block_number) = tx_from_rpc.block_number {
            if block_number <= min_block_number.into() {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "{EARLY_CONFIRMATION_ERR_LOG}: Fee tx {tx_from_rpc:?} confirmed before min_block {min_block_number}"
                )));
            }
        }
        match &coin.coin_type {
            EthCoinType::Eth => {
                if tx_from_rpc.to != Some(fee_addr) {
                    return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                        "{INVALID_RECEIVER_ERR_LOG}: Fee tx {tx_from_rpc:?} was sent to wrong address, expected {fee_addr:?}"
                    )));
                }

                if tx_from_rpc.value < expected_value {
                    return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                        "Fee tx {tx_from_rpc:?} value is less than expected {expected_value:?}"
                    )));
                }
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                if tx_from_rpc.to != Some(*token_addr) {
                    return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                        "{INVALID_CONTRACT_ADDRESS_ERR_LOG}: ERC20 Fee tx {tx_from_rpc:?} called wrong smart contract, expected {token_addr:?}"
                    )));
                }

                let function = ERC20_CONTRACT
                    .function("transfer")
                    .map_to_mm(|e| ValidatePaymentError::InternalError(e.to_string()))?;
                let decoded_input = decode_contract_call(function, &tx_from_rpc.input.0)
                    .map_to_mm(|e| ValidatePaymentError::TxDeserializationError(e.to_string()))?;
                let address_input = get_function_input_data(&decoded_input, function, 0)
                    .map_to_mm(ValidatePaymentError::TxDeserializationError)?;

                if address_input != Token::Address(fee_addr) {
                    return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                        "{INVALID_RECEIVER_ERR_LOG}: ERC20 Fee tx was sent to wrong address {address_input:?}, expected {fee_addr:?}"
                    )));
                }

                let value_input = get_function_input_data(&decoded_input, function, 1)
                    .map_to_mm(ValidatePaymentError::TxDeserializationError)?;

                match value_input {
                    Token::Uint(value) => {
                        if value < expected_value {
                            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                                "ERC20 Fee tx value {value} is less than expected {expected_value}"
                            )));
                        }
                    },
                    _ => {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Should have got uint token but got {value_input:?}"
                        )))
                    },
                }
            },
            EthCoinType::Nft { .. } => {
                return MmError::err(ValidatePaymentError::ProtocolNotSupported(
                    "Nft protocol is not supported".to_string(),
                ))
            },
        }

        Ok(())
    };
    Box::new(fut.boxed().compat())
}

pub(super) async fn get_raw_transaction_impl(coin: EthCoin, req: RawTransactionRequest) -> RawTransactionResult {
    let tx = match req.tx_hash.strip_prefix("0x") {
        Some(tx) => tx,
        None => &req.tx_hash,
    };
    let hash = H256::from_str(tx).map_to_mm(|e| RawTransactionError::InvalidHashError(e.to_string()))?;
    get_tx_hex_by_hash_impl(coin, hash).await
}

pub(super) async fn get_tx_hex_by_hash_impl(coin: EthCoin, tx_hash: H256) -> RawTransactionResult {
    let web3_tx = coin
        .transaction(TransactionId::Hash(tx_hash))
        .await?
        .or_mm_err(|| RawTransactionError::HashNotExist(tx_hash.to_string()))?;
    let raw = signed_tx_from_web3_tx(web3_tx).map_to_mm(RawTransactionError::InternalError)?;
    Ok(RawTransactionRes {
        tx_hex: BytesJson(rlp::encode(&raw).to_vec()),
    })
}

/// Converts and extended public key derived using BIP32 to an Ethereum public key.
pub fn pubkey_from_extended(extended_pubkey: &Secp256k1ExtendedPublicKey) -> Public {
    let serialized = extended_pubkey.public_key().serialize_uncompressed();
    let mut pubkey_uncompressed = Public::default();
    pubkey_uncompressed.as_mut().copy_from_slice(&serialized[1..]);
    pubkey_uncompressed
}

/// Signs an Eth transaction using `key_pair`.
///
/// This method polls for the latest nonce from the RPC nodes and uses it for the transaction to be signed.
/// A `nonce_lock` is returned so that the caller doesn't release it until the transaction is sent and the
/// address nonce is updated on RPC nodes.
#[allow(clippy::too_many_arguments)]
async fn sign_transaction_with_keypair(
    coin: &EthCoin,
    key_pair: &KeyPair,
    value: U256,
    action: Action,
    data: Vec<u8>,
    gas: U256,
    pay_for_gas_option: &PayForGasOption,
    from_address: Address,
) -> Result<(SignedEthTx, Vec<Web3Instance>), TransactionErr> {
    info!(target: "sign", "get_addr_nonce…");
    let (nonce, web3_instances_with_latest_nonce) = try_tx_s!(coin.clone().get_addr_nonce(from_address).compat().await);
    let tx_type = tx_type_from_pay_for_gas_option!(pay_for_gas_option);
    if !coin.is_tx_type_supported(&tx_type) {
        return Err(TransactionErr::Plain("Eth transaction type not supported".into()));
    }

    let tx_builder = UnSignedEthTxBuilder::new(tx_type, nonce, gas, action, value, data);
    let tx_builder = tx_builder_with_pay_for_gas_option(coin, tx_builder, pay_for_gas_option)
        .map_err(|e| TransactionErr::Plain(e.get_inner().to_string()))?;
    let tx = tx_builder.build()?;
    let chain_id = match coin.chain_spec {
        ChainSpec::Evm { chain_id } => chain_id,
        // Todo: Add Tron signing logic
        ChainSpec::Tron { .. } => {
            return Err(TransactionErr::Plain(
                "Tron is not supported for sign_transaction_with_keypair yet".into(),
            ))
        },
    };
    let signed_tx = tx.sign(key_pair.secret(), Some(chain_id))?;
    Ok((signed_tx, web3_instances_with_latest_nonce))
}

/// Sign and send eth transaction with provided keypair,
/// This fn is primarily for swap transactions so it uses swap tx fee policy
pub(super) async fn sign_and_send_transaction_with_keypair(
    coin: &EthCoin,
    key_pair: &KeyPair,
    address: Address,
    value: U256,
    action: Action,
    data: Vec<u8>,
    gas: U256,
) -> Result<SignedEthTx, TransactionErr> {
    info!(target: "sign-and-send", "get_gas_price…");
    let pay_for_gas_policy = try_tx_s!(coin.get_swap_gas_fee_policy().await);
    let pay_for_gas_option = try_tx_s!(coin.get_swap_pay_for_gas_option(pay_for_gas_policy).await);
    let address_lock = coin.get_address_lock(address).await;
    let _nonce_lock = address_lock.lock().await;
    let (signed, web3_instances_with_latest_nonce) =
        sign_transaction_with_keypair(coin, key_pair, value, action, data, gas, &pay_for_gas_option, address).await?;
    let bytes = Bytes(rlp::encode(&signed).to_vec());
    info!(target: "sign-and-send", "send_raw_transaction… txid={}", signed.tx_hash().to_hex());

    let futures = web3_instances_with_latest_nonce
        .into_iter()
        .map(|web3_instance| web3_instance.as_ref().eth().send_raw_transaction(bytes.clone()));
    try_tx_s!(select_ok(futures).await.map_err(|e| ERRL!("{}", e)), signed);

    info!(target: "sign-and-send", "wait_for_tx_appears_on_rpc…");
    coin.wait_for_addr_nonce_increase(address, signed.unsigned().nonce())
        .await;
    Ok(signed)
}

/// Sign and send eth transaction with metamask API,
/// This fn is primarily for swap transactions so it uses swap tx fee policy
#[cfg(target_arch = "wasm32")]
pub(super) async fn sign_and_send_transaction_with_metamask(
    coin: EthCoin,
    value: U256,
    action: Action,
    data: Vec<u8>,
    gas: U256,
) -> Result<SignedEthTx, TransactionErr> {
    let to = match action {
        Action::Create => None,
        Action::Call(to) => Some(to),
    };

    let pay_for_gas_option = try_tx_s!(
        coin.get_swap_pay_for_gas_option(try_tx_s!(coin.get_swap_gas_fee_policy().await))
            .await
    );
    let my_address = try_tx_s!(coin.derivation_method.single_addr_or_err().await);
    let gas_price = pay_for_gas_option.get_gas_price();
    let (max_fee_per_gas, max_priority_fee_per_gas) = pay_for_gas_option.get_fee_per_gas();
    let tx_to_send = TransactionRequest {
        from: my_address,
        to,
        gas: Some(gas),
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        value: Some(value),
        data: Some(data.clone().into()),
        nonce: None,
        ..TransactionRequest::default()
    };

    // It's important to return the transaction hex for the swap,
    // so wait up to 60 seconds for the transaction to appear on the RPC node.
    let wait_rpc_timeout = 60;
    let check_every = 1.;

    // Please note that this method may take a long time
    // due to `wallet_switchEthereumChain` and `eth_sendTransaction` requests.
    let tx_hash = try_tx_s!(coin.send_transaction(tx_to_send).await);

    let maybe_signed_tx = try_tx_s!(
        coin.wait_for_tx_appears_on_rpc(tx_hash, wait_rpc_timeout, check_every)
            .await
    );
    match maybe_signed_tx {
        Some(signed_tx) => Ok(signed_tx),
        None => TX_PLAIN_ERR!(
            "Waited too long until the transaction {:?} appear on the RPC node",
            tx_hash
        ),
    }
}

/// Sign eth transaction
pub(super) async fn sign_raw_eth_tx(coin: &EthCoin, args: &SignEthTransactionParams) -> RawTransactionResult {
    let value =
        u256_from_big_decimal(args.value.as_ref().unwrap_or(&BigDecimal::from(0)), coin.decimals).map_mm_err()?;
    let action = if let Some(to) = &args.to {
        Call(Address::from_str(to).map_to_mm(|err| RawTransactionError::InvalidParam(err.to_string()))?)
    } else {
        Create
    };
    let data = hex::decode(args.data.as_ref().unwrap_or(&String::from("")))?;
    match coin.priv_key_policy {
        // TODO: use zeroise for privkey
        EthPrivKeyPolicy::Iguana(ref key_pair)
        | EthPrivKeyPolicy::HDWallet {
            activated_key: ref key_pair,
            ..
        } => {
            let my_address = coin
                .derivation_method
                .single_addr_or_err()
                .await
                .mm_err(|e| RawTransactionError::InternalError(e.to_string()))?;
            let address_lock = coin.get_address_lock(my_address).await;
            let _nonce_lock = address_lock.lock().await;
            info!(target: "sign-and-send", "get_gas_price…");
            let pay_for_gas_option = coin
                .get_swap_pay_for_gas_option_from_rpc(&args.pay_for_gas)
                .await
                .map_mm_err()?;
            info!("sign_raw_eth_tx pay_for_gas_option {:?}", pay_for_gas_option);
            sign_transaction_with_keypair(
                coin,
                key_pair,
                value,
                action,
                data,
                args.gas_limit,
                &pay_for_gas_option,
                my_address,
            )
            .await
            .map(|(signed_tx, _)| RawTransactionRes {
                tx_hex: signed_tx.tx_hex().into(),
            })
            .map_to_mm(|err| RawTransactionError::TransactionError(err.get_plain_text_format()))
        },
        EthPrivKeyPolicy::WalletConnect { .. } => {
            // NOTE: doesn't work with wallets that doesn't support `eth_signTransaction`. e.g TrustWallet
            let wc = {
                let ctx = MmArc::from_weak(&coin.ctx).expect("No context");
                WalletConnectCtx::from_ctx(&ctx)
                    .expect("TODO: handle error when enable kdf initialization without key.")
            };
            // Todo: Tron will have to be set with `ChainSpec::Evm` to work with walletconnect.
            // This means setting the protocol as `ETH` in coin config and having a different coin for this mode.
            let chain_id = coin.chain_spec.chain_id().ok_or(RawTransactionError::InvalidParam(
                "WalletConnect needs chain_id to be set".to_owned(),
            ))?;
            let my_address = coin
                .derivation_method
                .single_addr_or_err()
                .await
                .mm_err(|e| RawTransactionError::InternalError(e.to_string()))?;
            let address_lock = coin.get_address_lock(my_address).await;
            let _nonce_lock = address_lock.lock().await;
            let pay_for_gas_option = coin
                .get_swap_pay_for_gas_option_from_rpc(&args.pay_for_gas)
                .await
                .map_mm_err()?;
            let (nonce, _) = coin
                .clone()
                .get_addr_nonce(my_address)
                .compat()
                .await
                .map_to_mm(RawTransactionError::InvalidParam)?;
            let (max_fee_per_gas, max_priority_fee_per_gas) = pay_for_gas_option.get_fee_per_gas();

            info!(target: "sign-and-send", "WalletConnect signing and sending tx…");
            let (signed_tx, _) = coin
                .wc_sign_tx(
                    &wc,
                    WcEthTxParams {
                        my_address,
                        gas_price: pay_for_gas_option.get_gas_price(),
                        action,
                        value,
                        gas: args.gas_limit,
                        data: &data,
                        nonce,
                        chain_id,
                        max_fee_per_gas,
                        max_priority_fee_per_gas,
                    },
                )
                .await
                .mm_err(|err| RawTransactionError::TransactionError(err.to_string()))?;

            Ok(RawTransactionRes {
                tx_hex: signed_tx.tx_hex().into(),
            })
        },
        EthPrivKeyPolicy::Trezor => MmError::err(RawTransactionError::InvalidParam(
            "sign raw eth tx not implemented for Trezor".into(),
        )),
        #[cfg(target_arch = "wasm32")]
        EthPrivKeyPolicy::Metamask(_) => MmError::err(RawTransactionError::InvalidParam(
            "sign raw eth tx not implemented for Metamask".into(),
        )),
    }
}

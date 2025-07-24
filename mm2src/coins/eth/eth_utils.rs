use super::{ETH_DECIMALS, ETH_GWEI_DECIMALS};
use crate::{NumConversError, NumConversResult};
use ethabi::{Function, Token};
use ethereum_types::{Address, FromDecStrErr, U256};
use ethkey::{public_to_address, Public};
use mm2_err_handle::prelude::MapToMmResult;
use mm2_number::{BigDecimal, MmNumber};
use secp256k1::PublicKey;

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
    Ok(format!("{:#02x}", addr))
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
pub fn wei_from_big_decimal(amount: &BigDecimal, decimals: u8) -> NumConversResult<U256> {
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
    U256::from_dec_str(&amount).map_to_mm(|e| NumConversError::new(format!("{:?}", e)))
}

/// Converts BigDecimal gwei value to wei value as U256
#[inline(always)]
pub fn wei_from_gwei_decimal(bigdec: &BigDecimal) -> NumConversResult<U256> {
    wei_from_big_decimal(bigdec, ETH_GWEI_DECIMALS)
}

/// Converts a U256 wei value to an gwei value as a BigDecimal
#[inline(always)]
pub fn wei_to_gwei_decimal(wei: U256) -> NumConversResult<BigDecimal> { u256_to_big_decimal(wei, ETH_GWEI_DECIMALS) }

/// Converts a U256 wei value to an ETH value as a BigDecimal
/// TODO: use wei_to_eth_decimal instead of u256_to_big_decimal(gas_cost_wei, ETH_DECIMALS)
#[inline(always)]
pub fn wei_to_eth_decimal(wei: U256) -> NumConversResult<BigDecimal> { u256_to_big_decimal(wei, ETH_DECIMALS) }

#[inline]
pub fn mm_number_to_u256(mm_number: &MmNumber) -> Result<U256, FromDecStrErr> {
    U256::from_dec_str(mm_number.to_ratio().to_integer().to_string().as_str())
}

#[inline]
pub fn mm_number_from_u256(u256: U256) -> MmNumber { MmNumber::from(u256.to_string().as_str()) }

#[inline]
pub fn wei_from_coins_mm_number(mm_number: &MmNumber, decimals: u8) -> NumConversResult<U256> {
    wei_from_big_decimal(&mm_number.to_decimal(), decimals)
}

#[inline]
#[allow(unused)]
pub fn wei_to_coins_mm_number(u256: U256, decimals: u8) -> NumConversResult<MmNumber> {
    Ok(MmNumber::from(u256_to_big_decimal(u256, decimals)?))
}

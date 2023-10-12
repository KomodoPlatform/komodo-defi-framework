use std::collections::{HashMap, HashSet};

use coins::utxo::UtxoActivationParams;
#[cfg(not(target_arch = "wasm32"))]
use coins::z_coin::ZcoinActivationParams;
use coins::{coin_conf, CoinBalance, CoinProtocol, MmCoinEnum};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use serde_derive::Serialize;
use serde_json::{self as json, Value as Json};

pub trait CurrentBlock {
    fn current_block(&self) -> u64;
}

pub trait TxHistory {
    fn tx_history(&self) -> bool;
}

impl TxHistory for UtxoActivationParams {
    fn tx_history(&self) -> bool { self.tx_history }
}

#[cfg(not(target_arch = "wasm32"))]
impl TxHistory for ZcoinActivationParams {
    fn tx_history(&self) -> bool { false }
}

pub trait GetAddressesBalances {
    fn get_addresses_balances(&self) -> HashMap<String, BigDecimal>;
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum DerivationMethod {
    /// Legacy iguana's privkey derivation, used by default
    Iguana,
    /// HD wallet derivation path, String is temporary here
    #[allow(dead_code)]
    HDWallet(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct CoinAddressInfo<Balance> {
    pub(crate) derivation_method: DerivationMethod,
    pub(crate) pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) balances: Option<Balance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tickers: Option<HashSet<String>>,
}

pub type TokenBalances = HashMap<String, CoinBalance>;

pub trait TryPlatformCoinFromMmCoinEnum {
    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized;
}

pub trait TryFromCoinProtocol {
    #[allow(clippy::result_large_err)]
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized;
}

#[derive(Debug)]
pub enum CoinConfWithProtocolError {
    ConfigIsNotFound(String),
    CoinProtocolParseError { ticker: String, err: json::Error },
    UnexpectedProtocol { ticker: String, protocol: CoinProtocol },
}

#[allow(clippy::result_large_err)]
pub fn coin_conf_with_protocol<T: TryFromCoinProtocol>(
    ctx: &MmArc,
    coin: &str,
) -> Result<(Json, T), MmError<CoinConfWithProtocolError>> {
    let conf = coin_conf(ctx, coin);
    if conf.is_null() {
        return MmError::err(CoinConfWithProtocolError::ConfigIsNotFound(coin.into()));
    }

    let coin_protocol: CoinProtocol = json::from_value(conf["protocol"].clone()).map_to_mm(|err| {
        CoinConfWithProtocolError::CoinProtocolParseError {
            ticker: coin.into(),
            err,
        }
    })?;

    let coin_protocol =
        T::try_from_coin_protocol(coin_protocol).mm_err(|protocol| CoinConfWithProtocolError::UnexpectedProtocol {
            ticker: coin.into(),
            protocol,
        })?;
    Ok((conf, coin_protocol))
}

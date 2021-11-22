use coins::{coin_conf, CoinBalance, CoinProtocol};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use serde_derive::Serialize;
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum DerivationMethod {
    /// Legacy iguana's privkey derivation, used by default
    Iguana,
    /// HD wallet derivation path, String is temporary here
    #[allow(dead_code)]
    HDWallet(String),
}

#[derive(Debug, Serialize)]
pub struct CoinAddressInfo<Balance> {
    pub(crate) derivation_method: DerivationMethod,
    pub(crate) pubkey: String,
    pub(crate) balances: Balance,
}

pub type TokenBalances = HashMap<String, CoinBalance>;

pub trait TryFromCoinProtocol {
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

pub fn coin_conf_with_protocol<T: TryFromCoinProtocol>(
    ctx: &MmArc,
    coin: &str,
) -> Result<(Json, T), MmError<CoinConfWithProtocolError>> {
    let conf = coin_conf(&ctx, coin);
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

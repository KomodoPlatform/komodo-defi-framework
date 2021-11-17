use coins::{coin_conf, CoinProtocol};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use serde_json::{self as json, Value as Json};

pub trait TryFromCoinProtocol {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized;
}

#[derive(Debug)]
pub enum CoinConfWithProtocolError {
    ConfigIsNotFound(String),
    CoinProtocolParseError(json::Error),
    UnexpectedProtocol(CoinProtocol),
}

pub fn coin_conf_with_protocol<T: TryFromCoinProtocol>(
    ctx: &MmArc,
    coin: &str,
) -> Result<(Json, T), MmError<CoinConfWithProtocolError>> {
    let conf = coin_conf(&ctx, coin);
    if conf.is_null() {
        return MmError::err(CoinConfWithProtocolError::ConfigIsNotFound(coin.into()));
    }

    let coin_protocol: CoinProtocol =
        json::from_value(conf["protocol"].clone()).map_to_mm(CoinConfWithProtocolError::CoinProtocolParseError)?;

    let coin_protocol =
        T::try_from_coin_protocol(coin_protocol).mm_err(CoinConfWithProtocolError::UnexpectedProtocol)?;
    Ok((conf, coin_protocol))
}

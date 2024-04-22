use coins::nft::nft_structs::{Chain, ConvertChain};
use coins::utxo::UtxoActivationParams;
use coins::z_coin::ZcoinActivationParams;
use coins::{coin_conf, CoinBalance, CoinProtocol, DerivationMethodResponse, MmCoinEnum};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use serde_derive::Serialize;
use serde_json::{self as json, Value as Json};
use std::collections::{HashMap, HashSet};

pub trait CurrentBlock {
    fn current_block(&self) -> u64;
}

pub trait TxHistory {
    fn tx_history(&self) -> bool;
}

impl TxHistory for UtxoActivationParams {
    fn tx_history(&self) -> bool { self.tx_history }
}

impl TxHistory for ZcoinActivationParams {
    fn tx_history(&self) -> bool { false }
}

pub trait GetAddressesBalances {
    fn get_addresses_balances(&self) -> HashMap<String, BigDecimal>;
}

#[derive(Clone, Debug, Serialize)]
pub struct CoinAddressInfo<Balance> {
    pub(crate) derivation_method: DerivationMethodResponse,
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

/// Determines the coin configuration and protocol information for a given coin or NFT ticker.
/// In the case of NFT ticker, it's platform coin config will be returned.
#[allow(clippy::result_large_err)]
pub fn coin_conf_with_protocol<T: TryFromCoinProtocol>(
    ctx: &MmArc,
    coin: &str,
) -> Result<(Json, T), MmError<CoinConfWithProtocolError>> {
    let (conf, coin_protocol) = match Chain::from_nft_ticker(coin) {
        Ok(chain) => {
            let platform = chain.to_ticker();
            let platform_conf = coin_conf(ctx, platform);
            let nft_protocol = CoinProtocol::NFT {
                platform: platform.to_string(),
            };
            (platform_conf, nft_protocol)
        },
        Err(_) => {
            let conf = coin_conf(ctx, coin);
            let coin_protocol: CoinProtocol = json::from_value(conf["protocol"].clone()).map_to_mm(|err| {
                CoinConfWithProtocolError::CoinProtocolParseError {
                    ticker: coin.into(),
                    err,
                }
            })?;
            (conf, coin_protocol)
        },
    };

    if conf.is_null() {
        return MmError::err(CoinConfWithProtocolError::ConfigIsNotFound(coin.into()));
    }

    let protocol =
        T::try_from_coin_protocol(coin_protocol).mm_err(|protocol| CoinConfWithProtocolError::UnexpectedProtocol {
            ticker: coin.into(),
            protocol,
        })?;
    Ok((conf, protocol))
}

/// A trait to be implemented for coin activation requests to determine some information about the request.
pub trait ActivationRequestInfo {
    /// Checks if the activation request is for a hardware wallet.
    fn is_hw_policy(&self) -> bool;
}

impl ActivationRequestInfo for UtxoActivationParams {
    fn is_hw_policy(&self) -> bool { self.priv_key_policy.is_hw_policy() }
}

#[cfg(not(target_arch = "wasm32"))]
impl ActivationRequestInfo for ZcoinActivationParams {
    fn is_hw_policy(&self) -> bool { false } // TODO: fix when device policy is added
}

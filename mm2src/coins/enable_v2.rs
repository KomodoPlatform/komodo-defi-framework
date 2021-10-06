use super::{coin_conf, lp_coinfind, CoinProtocol};
use crate::utxo::rpc_clients::ElectrumRpcRequest;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use derive_more::Display;
use ethereum_types::Address;
use serde_json::{self as json, Value as Json};

#[derive(Debug, Deserialize, Serialize)]
pub enum UtxoActivationMode {
    Native,
    Electrum { servers: Vec<ElectrumRpcRequest> },
}

#[derive(Debug, Deserialize, Serialize)]
pub enum ZcoinActivationMode {
    Native,
    // TODO extend in the future
    Zlite,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum EnableProtocolParams {
    Bch {
        #[serde(default)]
        allow_slp_unsafe_conf: bool,
        bchd_urls: Vec<String>,
        mode: UtxoActivationMode,
        with_tokens: Vec<String>,
    },
    SlpToken,
    Eth {
        urls: Vec<String>,
        swap_contract_address: Address,
        fallback_swap_contract: Option<Address>,
        with_tokens: Vec<String>,
    },
    Erc20,
    Utxo {
        mode: UtxoActivationMode,
    },
    Qrc20,
    Zcoin {
        mode: ZcoinActivationMode,
    },
}

#[derive(Debug, Deserialize)]
pub struct EnableRpcRequest {
    coin: String,
    tx_history: bool,
    required_confirmations: u64,
    requires_notarization: bool,
    protocol_params: EnableProtocolParams,
}

pub struct EnableResult {}

#[derive(Debug, Display)]
pub enum EnableError {
    /// With ticker
    CoinIsAlreadyActivated(String),
    /// With ticker
    CoinConfigNotFound(String),
    RequestDeserializationErr(serde_json::Error),
    InvalidCoinProtocolConf(serde_json::Error),
    #[display(
        fmt = "Request {:?} is not compatible with protocol {:?}",
        request,
        protocol_from_conf
    )]
    RequestNotCompatibleWithProtocol {
        request: EnableProtocolParams,
        protocol_from_conf: CoinProtocol,
    },
}

#[allow(unused_variables)]
pub async fn enable_v2(ctx: MmArc, req: Json) -> Result<EnableResult, MmError<EnableError>> {
    let req: EnableRpcRequest = json::from_value(req).map_to_mm(EnableError::RequestDeserializationErr)?;

    if let Ok(Some(_)) = lp_coinfind(&ctx, &req.coin).await {
        return MmError::err(EnableError::CoinIsAlreadyActivated(req.coin));
    }

    let conf = coin_conf(&ctx, &req.coin);
    if conf.is_null() {
        return MmError::err(EnableError::CoinConfigNotFound(req.coin));
    }

    let protocol: CoinProtocol =
        json::from_value(conf["protocol"].clone()).map_to_mm(EnableError::InvalidCoinProtocolConf)?;

    let coin = match (&req.protocol_params, &protocol) {
        (
            EnableProtocolParams::Bch {
                allow_slp_unsafe_conf,
                bchd_urls,
                mode,
                with_tokens,
            },
            CoinProtocol::BCH { slp_prefix },
        ) => {
            // BCH
        },
        (
            EnableProtocolParams::SlpToken,
            CoinProtocol::SLPTOKEN {
                platform,
                token_id,
                decimals,
                required_confirmations,
            },
        ) => {
            // SLP
        },
        (
            EnableProtocolParams::Eth {
                urls,
                swap_contract_address,
                fallback_swap_contract,
                with_tokens,
            },
            CoinProtocol::ETH,
        ) => {
            // ETH
        },
        (
            EnableProtocolParams::Erc20,
            CoinProtocol::ERC20 {
                platform,
                contract_address,
            },
        ) => {
            // Erc20
        },
        (EnableProtocolParams::Utxo { mode }, CoinProtocol::UTXO) => {
            // UTXO
        },
        (
            EnableProtocolParams::Qrc20,
            CoinProtocol::QRC20 {
                platform,
                contract_address,
            },
        ) => {
            // Qrc20
        },
        #[cfg(feature = "zhtlc")]
        (EnableProtocolParams::Zcoin { mode }, CoinProtocol::ZHTLC) => {
            // Qrc20
        },
        _ => {
            return MmError::err(EnableError::RequestNotCompatibleWithProtocol {
                request: req.protocol_params,
                protocol_from_conf: protocol,
            })
        },
    };
    unimplemented!()
}

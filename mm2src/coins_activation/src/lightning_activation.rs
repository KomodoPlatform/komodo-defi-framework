use crate::l2::{EnableL2Error, L2ActivationOps, L2ProtocolParams};
use crate::prelude::*;
use async_trait::async_trait;
use coins::lightning::ln_errors::EnableLightningError;
use coins::lightning::ln_utils::{start_lightning, LightningParams};
use coins::lightning::{LightningCoin, LightningProtocolConf};
use coins::utxo::utxo_standard::UtxoStandardCoin;
use coins::utxo::UtxoCommonOps;
use coins::{CoinProtocol, MarketCoinOps, MmCoinEnum};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use derive_more::Display;
use ser_error_derive::SerializeErrorType;
use serde_derive::{Deserialize, Serialize};

const DEFAULT_LISTENING_PORT: u16 = 9735;

impl TryPlatformCoinFromMmCoinEnum for UtxoStandardCoin {
    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized,
    {
        match coin {
            MmCoinEnum::UtxoCoin(coin) => Some(coin),
            _ => None,
        }
    }
}

impl TryFromCoinProtocol for LightningProtocolConf {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::LIGHTNING { platform, network } => Ok(LightningProtocolConf {
                platform_coin_ticker: platform,
                network,
            }),
            proto => MmError::err(proto),
        }
    }
}

impl L2ProtocolParams for LightningProtocolConf {
    fn platform_coin_ticker(&self) -> &str { &self.platform_coin_ticker }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LightningActivationParams {
    // The listening port for the p2p LN node
    pub listening_port: Option<u16>,
    // Printable human-readable string to describe this node to other users.
    pub name: String,
    // Node's HEX color. This is used for showing the node in a network graph with the desired color.
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum LightningValidationErr {
    #[display(fmt = "Platform coin {} activated in {} mode", _0, _1)]
    UnexpectedMethod(String, String),
    #[display(fmt = "{} is only supported in {} mode", _0, _1)]
    UnsupportedMode(String, String),
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
}

#[derive(Debug, Serialize)]
pub struct LightningInitResult {
    platform_coin: String,
}

#[derive(Debug)]
pub enum LightningInitError {
    EnableLightningError(EnableLightningError),
    LightningValidationErr(LightningValidationErr),
}

impl From<LightningInitError> for EnableL2Error {
    fn from(err: LightningInitError) -> Self {
        match err {
            LightningInitError::EnableLightningError(enable_err) => match enable_err {
                EnableLightningError::RpcError(rpc_err) => EnableL2Error::Transport(rpc_err),
                enable_error => EnableL2Error::Internal(enable_error.to_string()),
            },
            LightningInitError::LightningValidationErr(req_err) => EnableL2Error::Internal(req_err.to_string()),
        }
    }
}

impl From<EnableLightningError> for LightningInitError {
    fn from(err: EnableLightningError) -> Self { LightningInitError::EnableLightningError(err) }
}

impl From<LightningValidationErr> for LightningInitError {
    fn from(err: LightningValidationErr) -> Self { LightningInitError::LightningValidationErr(err) }
}

#[async_trait]
impl L2ActivationOps for LightningCoin {
    type PlatformCoin = UtxoStandardCoin;
    type ActivationParams = LightningActivationParams;
    type ProtocolInfo = LightningProtocolConf;
    type ValidatedParams = LightningParams;
    type ActivationResult = LightningInitResult;
    type ActivationError = LightningInitError;

    fn validate_platform_configuration(
        platform_coin: &Self::PlatformCoin,
    ) -> Result<(), MmError<Self::ActivationError>> {
        // Channel funding transactions need to spend segwit outputs
        // and while the witness script can be generated from pubkey and be used
        // it's better for the coin to be enabled in segwit to check if balance is enough for funding transaction, etc...
        if !platform_coin.addr_format().is_segwit() {
            return MmError::err(
                LightningValidationErr::UnsupportedMode("Lightning network".into(), "segwit".into()).into(),
            );
        }
        Ok(())
    }

    fn validate_activation_params(
        activation_params: Self::ActivationParams,
    ) -> Result<Self::ValidatedParams, MmError<Self::ActivationError>> {
        if activation_params.name.len() > 32 {
            return MmError::err(
                LightningValidationErr::InvalidRequest("Node name length can't be more than 32 characters".into())
                    .into(),
            );
        }
        let mut node_name = [b' '; 32];
        node_name[0..activation_params.name.len()].copy_from_slice(activation_params.name.as_bytes());

        let mut node_color = [0u8; 3];
        hex::decode_to_slice(
            activation_params.color.unwrap_or_else(|| "000000".into()),
            &mut node_color as &mut [u8],
        )
        .map_to_mm(|_| LightningValidationErr::InvalidRequest("Invalid Hex Color".into()))?;

        let listening_port = activation_params.listening_port.unwrap_or(DEFAULT_LISTENING_PORT);

        Ok(LightningParams {
            listening_port,
            node_name,
            node_color,
        })
    }

    async fn enable_l2(
        ctx: &MmArc,
        ticker: String,
        platform_coin: Self::PlatformCoin,
        validated_params: Self::ValidatedParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>> {
        let lightning_coin = start_lightning(
            ctx,
            platform_coin.clone(),
            ticker,
            validated_params,
            protocol_conf.network.into(),
        )
        .await?;
        let init_result = LightningInitResult {
            platform_coin: platform_coin.ticker().into(),
        };
        Ok((lightning_coin, init_result))
    }
}

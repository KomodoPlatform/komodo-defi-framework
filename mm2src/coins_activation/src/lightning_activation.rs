use crate::l2::{EnableL2Error, L2ActivationOps, L2ProtocolParams};
use crate::prelude::*;
use async_trait::async_trait;
use coins::coin_errors::MyAddressError;
use coins::lightning::ln_conf::{LightningCoinConf, LightningProtocolConf};
use coins::lightning::ln_errors::EnableLightningError;
use coins::lightning::{start_lightning, LightningCoin, LightningParams};
use coins::utxo::utxo_standard::UtxoStandardCoin;
use coins::utxo::UtxoCommonOps;
use coins::{BalanceError, CoinBalance, CoinProtocol, MarketCoinOps, MmCoinEnum};
use derive_more::Display;
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use ser_error_derive::SerializeErrorType;
use serde_derive::{Deserialize, Serialize};
use serde_json::{self as json, Value as Json};

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
            CoinProtocol::LIGHTNING {
                platform,
                network,
                confirmation_targets,
            } => Ok(LightningProtocolConf {
                platform_coin_ticker: platform,
                network,
                confirmation_targets,
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
    // The number of payment retries that should be done before considering a payment failed or partially failed.
    pub payment_retries: Option<usize>,
    // Node's backup path for channels and other data that requires backup.
    pub backup_path: Option<String>,
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
    address: String,
    balance: CoinBalance,
}

#[derive(Debug)]
pub enum LightningInitError {
    InvalidConfiguration(String),
    EnableLightningError(EnableLightningError),
    LightningValidationErr(LightningValidationErr),
    MyBalanceError(BalanceError),
    MyAddressError(String),
}

impl From<MyAddressError> for LightningInitError {
    fn from(err: MyAddressError) -> Self { Self::MyAddressError(err.to_string()) }
}

impl From<LightningInitError> for EnableL2Error {
    fn from(err: LightningInitError) -> Self {
        match err {
            LightningInitError::InvalidConfiguration(err) => EnableL2Error::L2ConfigParseError(err),
            LightningInitError::EnableLightningError(enable_err) => match enable_err {
                EnableLightningError::RpcError(rpc_err) => EnableL2Error::Transport(rpc_err),
                enable_error => EnableL2Error::Internal(enable_error.to_string()),
            },
            LightningInitError::LightningValidationErr(req_err) => EnableL2Error::Internal(req_err.to_string()),
            LightningInitError::MyBalanceError(balance_err) => match balance_err {
                BalanceError::Transport(e) => EnableL2Error::Transport(e),
                balance_error => EnableL2Error::Internal(balance_error.to_string()),
            },
            LightningInitError::MyAddressError(e) => EnableL2Error::Internal(e),
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
    type CoinConf = LightningCoinConf;
    type ActivationResult = LightningInitResult;
    type ActivationError = LightningInitError;

    fn coin_conf_from_json(json: Json) -> Result<Self::CoinConf, MmError<Self::ActivationError>> {
        json::from_value::<LightningCoinConf>(json)
            .map_to_mm(|e| LightningInitError::InvalidConfiguration(e.to_string()))
    }

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
            payment_retries: activation_params.payment_retries,
            backup_path: activation_params.backup_path,
        })
    }

    async fn enable_l2(
        ctx: &MmArc,
        platform_coin: Self::PlatformCoin,
        validated_params: Self::ValidatedParams,
        protocol_conf: Self::ProtocolInfo,
        coin_conf: Self::CoinConf,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>> {
        let lightning_coin =
            start_lightning(ctx, platform_coin.clone(), protocol_conf, coin_conf, validated_params).await?;
        let address = lightning_coin.my_address()?;
        let balance = lightning_coin
            .my_balance()
            .compat()
            .await
            .mm_err(LightningInitError::MyBalanceError)?;
        let init_result = LightningInitResult {
            platform_coin: platform_coin.ticker().into(),
            address,
            balance,
        };
        Ok((lightning_coin, init_result))
    }
}

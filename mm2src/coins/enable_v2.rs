use super::{coin_conf, lp_coinfind, CoinProtocol};
use crate::qrc20::Qrc20ActivationParams;
use crate::utxo::bch::{activate_bch_with_tokens, BchActivationParams};
use crate::utxo::UtxoActivationParams;
use crate::{BalanceError, CoinActivationOps, CoinActivationParamsOps, CoinBalance, CoinsContext, TokenCreationError};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::NotSame;
use derive_more::Display;
use ethereum_types::Address;
use futures::compat::Future01CompatExt;
use serde_json::{self as json};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize)]
pub enum ZcoinActivationMode {
    Native,
    // TODO extend in the future
    Zlite,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "protocol_name", content = "protocol_data")]
pub enum EnableProtocolParams {
    Bch(BchActivationParams),
    SlpToken,
    Eth {
        urls: Vec<String>,
        swap_contract_address: Address,
        fallback_swap_contract: Option<Address>,
        with_tokens: Vec<String>,
    },
    Erc20,
    Utxo(UtxoActivationParams),
    Qrc20(Qrc20ActivationParams),
    Zcoin {
        mode: ZcoinActivationMode,
    },
}

#[derive(Debug, Deserialize)]
pub struct EnableRpcRequest {
    coin: String,
    protocol_params: EnableProtocolParams,
}

#[derive(Debug, Serialize)]
pub struct EnableResult<T> {
    current_block: u64,
    platform_coin_balances: HashMap<String, CoinBalance>,
    token_balances: HashMap<String, HashMap<String, CoinBalance>>,
    resulting_coin_config: T,
}

#[derive(Debug, Display)]
#[allow(clippy::large_enum_variant)]
pub enum EnableError {
    /// With ticker
    CoinIsAlreadyActivated(String),
    /// With ticker
    CoinConfigNotFound(String),
    /// With ticker
    TokenConfigNotFound(String),
    TokenCreationError(TokenCreationError),
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
    GetCurrentBlockError(String),
    GetBalanceError(BalanceError),
}

impl From<TokenCreationError> for EnableError {
    fn from(err: TokenCreationError) -> Self { EnableError::TokenCreationError(err) }
}

impl From<BalanceError> for EnableError {
    fn from(err: BalanceError) -> Self { EnableError::GetBalanceError(err) }
}

#[allow(unused_variables)]
pub async fn enable_v2(ctx: MmArc, req: EnableRpcRequest) -> Result<EnableResult<()>, MmError<EnableError>> {
    if let Ok(Some(_)) = lp_coinfind(&ctx, &req.coin).await {
        return MmError::err(EnableError::CoinIsAlreadyActivated(req.coin));
    }

    let conf = coin_conf(&ctx, &req.coin);
    if conf.is_null() {
        return MmError::err(EnableError::CoinConfigNotFound(req.coin));
    }

    let protocol: CoinProtocol =
        json::from_value(conf["protocol"].clone()).map_to_mm(EnableError::InvalidCoinProtocolConf)?;
    let priv_key = &*ctx.secp256k1_key_pair().private().secret;

    let coin = match (req.protocol_params, protocol) {
        (EnableProtocolParams::Bch(params), CoinProtocol::BCH { slp_prefix }) => {
            // TODO it's unclear now how to not overcomplicate the error in this case
            let bch_activation_result = activate_bch_with_tokens(&ctx, &req.coin, &conf, params, &slp_prefix, priv_key)
                .await
                .unwrap();
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
        (EnableProtocolParams::Utxo(_), CoinProtocol::UTXO) => {
            // UTXO
        },
        (
            EnableProtocolParams::Qrc20(_),
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
        (request, protocol_from_conf) => {
            return MmError::err(EnableError::RequestNotCompatibleWithProtocol {
                request,
                protocol_from_conf,
            })
        },
    };
    unimplemented!()
}

pub async fn enable_coin<T>(
    ctx: &MmArc,
    ticker: &str,
    params: T::ActivationParams,
) -> Result<EnableResult<()>, MmError<EnableError>>
where
    T: CoinActivationOps,
    EnableError: From<T::ActivationError>,
    (T::ActivationError, EnableError): NotSame,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, ticker).await {
        return MmError::err(EnableError::CoinIsAlreadyActivated(ticker.to_owned()));
    }

    let conf = coin_conf(&ctx, ticker);
    if conf.is_null() {
        return MmError::err(EnableError::CoinConfigNotFound(ticker.to_owned()));
    }

    let token_tickers = params.activate_with_tokens();
    let platform_coin = T::activate(ctx, ticker, &conf, params).await?;
    let tokens: Vec<_> = token_tickers
        .into_iter()
        .map(|token| {
            let conf = coin_conf(&ctx, &token);
            if conf.is_null() {
                return MmError::err(EnableError::TokenConfigNotFound(token));
            }
            let token_as_coin = platform_coin.activate_token(&token, &conf)?;
            Ok(token_as_coin)
        })
        .collect::<Result<_, _>>()?;
    let coins_ctx = CoinsContext::from_ctx(ctx).unwrap();
    let mut coins = coins_ctx.coins.lock().await;

    let platform_coin = platform_coin.into();

    let current_block = platform_coin
        .current_block()
        .compat()
        .await
        .map_to_mm(EnableError::GetCurrentBlockError)?;

    let balances = platform_coin.get_balances_with_tokens().compat().await?;

    if coins.contains_key(ticker) {
        return MmError::err(EnableError::CoinIsAlreadyActivated(ticker.to_owned()));
    }

    coins.insert(ticker.to_owned(), platform_coin);
    for token in tokens {
        coins.insert(token.ticker().to_owned(), token);
    }

    Ok(EnableResult {
        current_block,
        platform_coin_balances: balances.platform_coin_balances,
        token_balances: balances.token_balances,
        resulting_coin_config: (),
    })
}

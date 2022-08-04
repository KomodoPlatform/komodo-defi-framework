use crate::{platform_coin_with_tokens::{EnablePlatformCoinWithTokensError, GetPlatformBalance,
                                        InitTokensAsMmCoinsError, PlatformWithTokensActivationOps, RegisterTokenInfo,
                                        TokenActivationParams, TokenActivationRequest, TokenAsMmCoinInitializer,
                                        TokenInitializer, TokenOf},
            prelude::*};
use async_trait::async_trait;
use coins::{eth::{v2_activation::{eth_coin_from_conf_and_request_v2, Erc20Protocol, Erc20TokenActivationError,
                                  Erc20TokenActivationRequest, EthActivationV2Error, EthActivationV2Request},
                  Erc20TokenInfo, EthCoin, EthCoinType},
            my_tx_history_v2::TxHistoryStorage,
            CoinBalance, CoinProtocol, MarketCoinOps, MmCoin};
use common::{mm_metrics::MetricsArc, Future01CompatExt};
use futures::future::AbortHandle;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;

impl From<EthActivationV2Error> for EnablePlatformCoinWithTokensError {
    fn from(err: EthActivationV2Error) -> Self {
        match err {
            EthActivationV2Error::InvalidPayload(e)
            | EthActivationV2Error::InvalidSwapContractAddr(e)
            | EthActivationV2Error::InvalidFallbackSwapContract(e) => {
                EnablePlatformCoinWithTokensError::InvalidPayload(e)
            },
            EthActivationV2Error::ActivationFailed { ticker, error } => {
                EnablePlatformCoinWithTokensError::PlatformCoinCreationError { ticker, error }
            },
            EthActivationV2Error::AtLeastOneNodeRequired => EnablePlatformCoinWithTokensError::AtLeastOneNodeRequired(
                "Enable request for ETH coin must have at least 1 node".to_string(),
            ),
            EthActivationV2Error::CouldNotFetchBalance(e) | EthActivationV2Error::UnreachableNodes(e) => {
                EnablePlatformCoinWithTokensError::Transport(e)
            },
            EthActivationV2Error::InternalError(e) => EnablePlatformCoinWithTokensError::Internal(e),
        }
    }
}

impl TryFromCoinProtocol for EthCoinType {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::ETH => Ok(EthCoinType::Eth),
            protocol => MmError::err(protocol),
        }
    }
}

pub struct Erc20Initializer {
    platform_coin: EthCoin,
}

impl From<Erc20TokenActivationError> for InitTokensAsMmCoinsError {
    fn from(error: Erc20TokenActivationError) -> Self {
        match error {
            Erc20TokenActivationError::InternalError(e) => InitTokensAsMmCoinsError::Internal(e),
            Erc20TokenActivationError::CouldNotFetchBalance(e) => InitTokensAsMmCoinsError::CouldNotFetchBalance(e),
        }
    }
}

#[async_trait]
impl TokenInitializer for Erc20Initializer {
    type Token = EthCoin;
    type TokenActivationRequest = Erc20TokenActivationRequest;
    type TokenProtocol = Erc20Protocol;
    type InitTokensError = Erc20TokenActivationError;

    fn tokens_requests_from_platform_request(
        platform_params: &EthWithTokensActivationRequest,
    ) -> Vec<TokenActivationRequest<Self::TokenActivationRequest>> {
        platform_params.erc20_tokens_requests.clone()
    }

    async fn enable_tokens(
        &self,
        activation_params: Vec<TokenActivationParams<Erc20TokenActivationRequest, Erc20Protocol>>,
    ) -> Result<Vec<EthCoin>, MmError<Erc20TokenActivationError>> {
        let mut tokens = vec![];
        for param in activation_params {
            let token: EthCoin = self
                .platform_coin
                .initialize_erc20_token(param.activation_request, param.protocol, param.ticker)
                .await?;
            tokens.push(token);
        }

        Ok(tokens)
    }

    fn platform_coin(&self) -> &EthCoin { &self.platform_coin }
}

#[derive(Clone, Deserialize)]
pub struct EthWithTokensActivationRequest {
    #[serde(flatten)]
    platform_request: EthActivationV2Request,
    erc20_tokens_requests: Vec<TokenActivationRequest<Erc20TokenActivationRequest>>,
}

impl TxHistory for EthWithTokensActivationRequest {
    fn tx_history(&self) -> bool { false }
}

impl TokenOf for EthCoin {
    type PlatformCoin = EthCoin;
}

impl RegisterTokenInfo<EthCoin> for EthCoin {
    fn register_token_info(&self, token: &EthCoin) {
        self.add_erc_token_info(token.ticker().to_string(), Erc20TokenInfo {
            token_address: token.erc20_token_address().unwrap(),
            decimals: token.decimals(),
        });
    }
}

#[derive(Serialize)]
pub struct EthWithTokensActivationResult {
    current_block: u64,
    eth_addresses_infos: HashMap<String, CoinAddressInfo<CoinBalance>>,
    erc20_addresses_infos: HashMap<String, CoinAddressInfo<TokenBalances>>,
}

impl GetPlatformBalance for EthWithTokensActivationResult {
    fn get_platform_balance(&self) -> BigDecimal {
        self.eth_addresses_infos
            .iter()
            .fold(BigDecimal::from(0), |total, (_, addr_info)| {
                &total + &addr_info.balances.get_total()
            })
    }
}

impl CurrentBlock for EthWithTokensActivationResult {
    fn current_block(&self) -> u64 { self.current_block }
}

#[async_trait]
impl PlatformWithTokensActivationOps for EthCoin {
    type ActivationRequest = EthWithTokensActivationRequest;
    type PlatformProtocolInfo = EthCoinType;
    type ActivationResult = EthWithTokensActivationResult;
    type ActivationError = EthActivationV2Error;

    async fn enable_platform_coin(
        ctx: MmArc,
        ticker: String,
        platform_conf: Json,
        activation_request: Self::ActivationRequest,
        _protocol: Self::PlatformProtocolInfo,
        priv_key: &[u8],
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let platform_coin = eth_coin_from_conf_and_request_v2(
            &ctx,
            &ticker,
            &platform_conf,
            activation_request.platform_request,
            priv_key,
        )
        .await?;

        Ok(platform_coin)
    }

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>> {
        vec![Box::new(Erc20Initializer {
            platform_coin: self.clone(),
        })]
    }

    async fn get_activation_result(&self) -> Result<EthWithTokensActivationResult, MmError<EthActivationV2Error>> {
        let my_address = self.my_address().map_err(EthActivationV2Error::InternalError)?;
        let pubkey = self
            .get_public_key()
            .map_err(|e| EthActivationV2Error::InternalError(e.to_string()))?;

        let current_block = self
            .current_block()
            .compat()
            .await
            .map_err(EthActivationV2Error::InternalError)?;

        let eth_balance = self
            .my_balance()
            .compat()
            .await
            .map_err(|e| EthActivationV2Error::CouldNotFetchBalance(e.to_string()))?;
        let token_balances = self
            .get_tokens_balance_list()
            .await
            .map_err(|e| EthActivationV2Error::CouldNotFetchBalance(e.to_string()))?;

        let mut result = EthWithTokensActivationResult {
            current_block,
            eth_addresses_infos: HashMap::new(),
            erc20_addresses_infos: HashMap::new(),
        };

        result
            .eth_addresses_infos
            .insert(my_address.to_string(), CoinAddressInfo {
                derivation_method: DerivationMethod::Iguana,
                pubkey: pubkey.clone(),
                balances: eth_balance,
            });

        result
            .erc20_addresses_infos
            .insert(my_address.to_string(), CoinAddressInfo {
                derivation_method: DerivationMethod::Iguana,
                pubkey,
                balances: token_balances,
            });

        Ok(result)
    }

    fn start_history_background_fetching(
        &self,
        _metrics: MetricsArc,
        _storage: impl TxHistoryStorage + Send + 'static,
        _initial_balance: BigDecimal,
    ) -> AbortHandle {
        unimplemented!()
    }
}

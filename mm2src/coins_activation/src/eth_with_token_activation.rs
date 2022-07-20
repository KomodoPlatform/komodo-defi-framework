use crate::{platform_coin_with_tokens::{EnablePlatformCoinWithTokensError, GetPlatformBalance,
                                        PlatformWithTokensActivationOps, RegisterTokenInfo, TokenActivationParams,
                                        TokenActivationRequest, TokenAsMmCoinInitializer, TokenInitializer, TokenOf},
            prelude::*};
use async_trait::async_trait;
use coins::{coin_conf,
            eth::{eth_coin_from_conf_and_request_v2, Erc20TokenInfo, EthActivationRequest, EthActivationV2Error,
                  EthCoin},
            lp_register_coin,
            my_tx_history_v2::TxHistoryStorage,
            CoinBalance, CoinProtocol, MarketCoinOps, MmCoin, MmCoinEnum, RegisterCoinParams};
use common::{block_on, mm_metrics::MetricsArc, Future01CompatExt};
use crypto::CryptoCtx;
use futures::future::AbortHandle;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;
use tokio::task::block_in_place;

impl From<EthActivationV2Error> for EnablePlatformCoinWithTokensError {
    fn from(err: EthActivationV2Error) -> Self {
        match err {
            EthActivationV2Error::InvalidPayload(e) => EnablePlatformCoinWithTokensError::InvalidPayload(e),
            EthActivationV2Error::ActivationFailed { ticker, error } => {
                EnablePlatformCoinWithTokensError::PlatformCoinCreationError { ticker, error }
            },
            EthActivationV2Error::CouldNotFetchBalance(e)
            | EthActivationV2Error::UnreachableNodes(e)
            | EthActivationV2Error::AtLeastOneNodeRequired(e) => EnablePlatformCoinWithTokensError::Transport(e),
            EthActivationV2Error::InternalError(e) => EnablePlatformCoinWithTokensError::Internal(e),
        }
    }
}

impl TryFromCoinProtocol for CoinProtocol {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        Ok(proto)
    }
}

pub struct Erc20Initializer {
    platform_coin: EthCoin,
}

#[async_trait]
impl TokenInitializer for Erc20Initializer {
    type Token = EthCoin;
    type TokenActivationRequest = EthActivationRequest;
    type TokenProtocol = CoinProtocol;
    type InitTokensError = std::convert::Infallible;

    fn tokens_requests_from_platform_request(
        platform_params: &EthWithTokensActivationRequest,
    ) -> Vec<TokenActivationRequest<Self::TokenActivationRequest>> {
        platform_params.erc20_tokens_requests.clone()
    }

    async fn enable_tokens(
        &self,
        activation_params: Vec<TokenActivationParams<EthActivationRequest, CoinProtocol>>,
    ) -> Result<Vec<EthCoin>, MmError<std::convert::Infallible>> {
        let ctx = MmArc::from_weak(&self.platform_coin.ctx).ok_or("No context").unwrap();

        let secret = CryptoCtx::from_ctx(&ctx)
            .unwrap()
            .iguana_ctx()
            .secp256k1_privkey_bytes()
            .to_vec();

        let mut tokens = vec![];
        for param in activation_params {
            let coins_en = coin_conf(&ctx, &param.ticker);
            let coin: EthCoin = eth_coin_from_conf_and_request_v2(
                &ctx,
                &param.ticker,
                &coins_en,
                param.activation_request.clone(),
                &secret,
                param.protocol,
            )
            .await
            .unwrap();

            let register_params = RegisterCoinParams {
                ticker: param.ticker,
                tx_history: param.activation_request.tx_history.unwrap_or(false),
            };

            lp_register_coin(&ctx, MmCoinEnum::EthCoin(coin.clone()), register_params)
                .await
                .unwrap();

            tokens.push(coin);
        }

        Ok(tokens)
    }

    fn platform_coin(&self) -> &EthCoin { &self.platform_coin }
}

#[derive(Clone, Debug, Deserialize)]
pub struct EthWithTokensActivationRequest {
    #[serde(flatten)]
    platform_request: EthActivationRequest,
    erc20_tokens_requests: Vec<TokenActivationRequest<EthActivationRequest>>,
}

impl TxHistory for EthWithTokensActivationRequest {
    fn tx_history(&self) -> bool { self.platform_request.tx_history.unwrap_or(false) }
}

impl TokenOf for EthCoin {
    type PlatformCoin = EthCoin;
}

impl RegisterTokenInfo<EthCoin> for EthCoin {
    fn register_token_info(&self, token: &EthCoin) {
        block_in_place(|| {
            let fut = self.add_erc_token_info(token.ticker().to_string(), Erc20TokenInfo {
                token_address: token.swap_contract_address,
                decimals: token.decimals(),
            });
            block_on(fut);
        });
    }
}

#[derive(Debug, Serialize)]
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
    type PlatformProtocolInfo = CoinProtocol;
    type ActivationResult = EthWithTokensActivationResult;
    type ActivationError = EthActivationV2Error;

    async fn enable_platform_coin(
        ctx: MmArc,
        ticker: String,
        platform_conf: Json,
        activation_request: Self::ActivationRequest,
        protocol: Self::PlatformProtocolInfo,
        priv_key: &[u8],
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let platform_coin = eth_coin_from_conf_and_request_v2(
            &ctx,
            &ticker,
            &platform_conf,
            activation_request.platform_request,
            priv_key,
            protocol,
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
                pubkey: my_address.clone(),
                balances: eth_balance,
            });

        result
            .erc20_addresses_infos
            .insert(my_address.to_string(), CoinAddressInfo {
                derivation_method: DerivationMethod::Iguana,
                pubkey: my_address,
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
        todo!()
    }
}

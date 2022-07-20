use crate::{platform_coin_with_tokens::{EnablePlatformCoinWithTokensError, GetPlatformBalance,
                                        PlatformWithTokensActivationOps, RegisterTokenInfo, TokenActivationParams,
                                        TokenActivationRequest, TokenAsMmCoinInitializer, TokenInitializer, TokenOf},
            prelude::*};
use async_trait::async_trait;
use coins::{coin_conf,
            eth::{eth_coin_from_conf_and_request_v2, valid_addr_from_str, Erc20TokenInfo, EthActivationRequest,
                  EthActivationV2Error, EthCoin, EthCoinType},
            lp_register_coin,
            my_tx_history_v2::TxHistoryStorage,
            CoinBalance, CoinProtocol, MarketCoinOps, MmCoin, MmCoinEnum, RegisterCoinParams};
use common::{mm_metrics::MetricsArc, Future01CompatExt};
use crypto::CryptoCtx;
use futures::future::AbortHandle;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;

pub struct Erc20Initializer {
    platform_coin: EthCoin,
}

pub struct EthProtocolInfo {
    coin_type: EthCoinType,
    decimals: u8,
}
impl TokenOf for EthCoin {
    type PlatformCoin = EthCoin;
}

impl RegisterTokenInfo<EthCoin> for EthCoin {
    fn register_token_info(&self, _token: &EthCoin) {
        // self.add_erc_token_info(Erc20TokenInfo {
        //     token_address: token.my_address,
        //     decimals: token.decimals(),
        // })
        // .await;
    }
}

#[async_trait]
impl TokenInitializer for Erc20Initializer {
    type Token = EthCoin;
    type TokenActivationRequest = EthActivationRequest;
    type TokenProtocol = EthProtocolInfo;
    type InitTokensError = std::convert::Infallible;

    fn tokens_requests_from_platform_request(
        platform_params: &EthWithTokensActivationRequest,
    ) -> Vec<TokenActivationRequest<Self::TokenActivationRequest>> {
        platform_params.erc20_tokens_requests.clone()
    }

    async fn enable_tokens(
        &self,
        activation_params: Vec<TokenActivationParams<EthActivationRequest, EthProtocolInfo>>,
    ) -> Result<Vec<EthCoin>, MmError<std::convert::Infallible>> {
        let ctx = MmArc::from_weak(&self.platform_coin.ctx)
            .clone()
            .ok_or("No context")
            .unwrap();

        let secret = CryptoCtx::from_ctx(&ctx)
            .unwrap()
            .iguana_ctx()
            .secp256k1_privkey_bytes()
            .to_vec();

        let mut tokens = vec![];
        for param in activation_params {
            let coins_en = coin_conf(&ctx, &param.activation_request.coin);
            let protocol: CoinProtocol = serde_json::from_value(coins_en["protocol"].clone()).unwrap();
            let coin: EthCoin = eth_coin_from_conf_and_request_v2(
                &ctx,
                &param.activation_request.coin,
                &coins_en,
                param.activation_request.clone(),
                &secret,
                protocol,
            )
            .await
            .unwrap()
            .into();

            self.platform_coin()
                .add_erc_token_info(coin.ticker().to_string(), Erc20TokenInfo {
                    token_address: coin.swap_contract_address,
                    decimals: coin.decimals(),
                })
                .await;

            let register_params = RegisterCoinParams {
                ticker: param.activation_request.coin,
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

impl TryFromCoinProtocol for EthProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::ETH => Ok(EthProtocolInfo {
                coin_type: EthCoinType::Eth,
                decimals: 18,
            }),
            CoinProtocol::ERC20 {
                platform,
                contract_address,
            } => {
                let token_addr = valid_addr_from_str(&contract_address).unwrap();
                Ok(EthProtocolInfo {
                    coin_type: EthCoinType::Erc20 { platform, token_addr },
                    decimals: 6,
                })
            },
            protocol => MmError::err(protocol),
        }
    }
}

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

#[derive(Clone, Debug, Deserialize)]
pub struct EthWithTokensActivationRequest {
    #[serde(flatten)]
    platform_request: EthActivationRequest,
    erc20_tokens_requests: Vec<TokenActivationRequest<EthActivationRequest>>,
}

impl TxHistory for EthWithTokensActivationRequest {
    fn tx_history(&self) -> bool { self.platform_request.tx_history.unwrap_or(false) }
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
    type PlatformProtocolInfo = EthProtocolInfo;
    type ActivationResult = EthWithTokensActivationResult;
    type ActivationError = EthActivationV2Error;

    async fn enable_platform_coin(
        ctx: MmArc,
        ticker: String,
        platform_conf: Json,
        activation_request: Self::ActivationRequest,
        _protocol_conf: Self::PlatformProtocolInfo,
        priv_key: &[u8],
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let coins_en = coin_conf(&ctx, &ticker);

        let protocol: CoinProtocol = serde_json::from_value(coins_en["protocol"].clone()).map_err(|e| {
            Self::ActivationError::ActivationFailed {
                ticker: ticker.clone(),
                error: e.to_string(),
            }
        })?;

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

        // let bch_unspents = self.bch_unspents_for_display(my_address).await?;
        // let bch_balance = bch_unspents.platform_balance(self.decimals());

        let eth_balance = self.my_balance().compat().await.unwrap();

        let token_balances = self.get_tokens_balance_list().await;

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

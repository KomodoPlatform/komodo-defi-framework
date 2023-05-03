use crate::{platform_coin_with_tokens::{EnablePlatformCoinWithTokensError, GetPlatformBalance,
                                        InitTokensAsMmCoinsError, PlatformWithTokensActivationOps, RegisterTokenInfo,
                                        TokenActivationParams, TokenActivationRequest, TokenAsMmCoinInitializer,
                                        TokenInitializer, TokenOf},
            prelude::*};
use async_trait::async_trait;
use coins::eth::EthPrivKeyBuildPolicy;
use coins::{eth::v2_activation::EthPrivKeyActivationPolicy, MmCoinEnum};
use coins::{eth::{v2_activation::{eth_coin_from_conf_and_request_v2, Erc20Protocol, Erc20TokenActivationError,
                                  Erc20TokenActivationRequest, EthActivationV2Error, EthActivationV2Request},
                  Erc20TokenInfo, EthCoin, EthCoinType},
            my_tx_history_v2::TxHistoryStorage,
            CoinBalance, CoinProtocol, MarketCoinOps, MmCoin};
use common::Future01CompatExt;
use common::{drop_mutability, true_f};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
#[cfg(target_arch = "wasm32")]
use mm2_metamask::MetamaskRpcError;
use mm2_number::BigDecimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};

impl From<EthActivationV2Error> for EnablePlatformCoinWithTokensError {
    fn from(err: EthActivationV2Error) -> Self {
        match err {
            EthActivationV2Error::InvalidPayload(e)
            | EthActivationV2Error::InvalidSwapContractAddr(e)
            | EthActivationV2Error::InvalidFallbackSwapContract(e) => {
                EnablePlatformCoinWithTokensError::InvalidPayload(e)
            },
            #[cfg(target_arch = "wasm32")]
            EthActivationV2Error::ExpectedRpcChainId => {
                EnablePlatformCoinWithTokensError::InvalidPayload(err.to_string())
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
            EthActivationV2Error::DerivationPathIsNotSet => EnablePlatformCoinWithTokensError::InvalidPayload(
                "'derivation_path' field is not found in config".to_string(),
            ),
            EthActivationV2Error::ErrorDeserializingDerivationPath(e) => {
                EnablePlatformCoinWithTokensError::InvalidPayload(e)
            },
            EthActivationV2Error::PrivKeyPolicyNotAllowed(e) => {
                EnablePlatformCoinWithTokensError::PrivKeyPolicyNotAllowed(e)
            },
            #[cfg(target_arch = "wasm32")]
            EthActivationV2Error::MetamaskError(metamask) => {
                EnablePlatformCoinWithTokensError::Transport(metamask.to_string())
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
    #[serde(default = "true_f")]
    pub get_balances: bool,
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
    fn get_platform_balance(&self) -> Option<BigDecimal> {
        self.eth_addresses_infos
            .iter()
            .fold(Some(BigDecimal::from(0)), |total, (_, addr_info)| {
                total.and_then(|t| addr_info.balances.as_ref().map(|b| t + b.get_total()))
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
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let priv_key_policy = eth_priv_key_build_policy(&ctx, &activation_request.platform_request.priv_key_policy)?;

        let platform_coin = eth_coin_from_conf_and_request_v2(
            &ctx,
            &ticker,
            &platform_conf,
            activation_request.platform_request,
            priv_key_policy,
        )
        .await?;

        Ok(platform_coin)
    }

    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized,
    {
        match coin {
            MmCoinEnum::EthCoin(coin) => Some(coin),
            _ => None,
        }
    }

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>> {
        vec![Box::new(Erc20Initializer {
            platform_coin: self.clone(),
        })]
    }

    async fn get_activation_result(
        &self,
        activation_request: &Self::ActivationRequest,
    ) -> Result<EthWithTokensActivationResult, MmError<EthActivationV2Error>> {
        let current_block = self
            .current_block()
            .compat()
            .await
            .map_err(EthActivationV2Error::InternalError)?;

        let my_address = self.my_address()?;
        let pubkey = self.get_public_key()?;

        let mut eth_address_info = CoinAddressInfo {
            derivation_method: DerivationMethod::Iguana,
            pubkey: pubkey.clone(),
            balances: None,
            tickers: None,
        };

        let mut erc20_address_info = CoinAddressInfo {
            derivation_method: DerivationMethod::Iguana,
            pubkey,
            balances: None,
            tickers: None,
        };

        if !activation_request.get_balances {
            drop_mutability!(eth_address_info);
            let tickers: HashSet<_> = self.get_erc_tokens_infos().into_keys().collect();
            erc20_address_info.tickers = Some(tickers);
            drop_mutability!(erc20_address_info);

            return Ok(EthWithTokensActivationResult {
                current_block,
                eth_addresses_infos: HashMap::from([(my_address.clone(), eth_address_info)]),
                erc20_addresses_infos: HashMap::from([(my_address, erc20_address_info)]),
            });
        }

        let eth_balance = self
            .my_balance()
            .compat()
            .await
            .map_err(|e| EthActivationV2Error::CouldNotFetchBalance(e.to_string()))?;
        eth_address_info.balances = Some(eth_balance);
        drop_mutability!(eth_address_info);

        let token_balances = self
            .get_tokens_balance_list()
            .await
            .map_err(|e| EthActivationV2Error::CouldNotFetchBalance(e.to_string()))?;
        erc20_address_info.balances = Some(token_balances);
        drop_mutability!(erc20_address_info);

        Ok(EthWithTokensActivationResult {
            current_block,
            eth_addresses_infos: HashMap::from([(my_address.clone(), eth_address_info)]),
            erc20_addresses_infos: HashMap::from([(my_address, erc20_address_info)]),
        })
    }

    fn start_history_background_fetching(
        &self,
        _ctx: MmArc,
        _storage: impl TxHistoryStorage + Send + 'static,
        _initial_balance: Option<BigDecimal>,
    ) {
    }
}

fn eth_priv_key_build_policy(
    ctx: &MmArc,
    activation_policy: &EthPrivKeyActivationPolicy,
) -> MmResult<EthPrivKeyBuildPolicy, EthActivationV2Error> {
    match activation_policy {
        EthPrivKeyActivationPolicy::ContextPrivKey => Ok(EthPrivKeyBuildPolicy::detect_priv_key_policy(ctx)?),
        #[cfg(target_arch = "wasm32")]
        EthPrivKeyActivationPolicy::Metamask => {
            let metamask_ctx = crypto::CryptoCtx::from_ctx(ctx)?
                .metamask_ctx()
                .or_mm_err(|| EthActivationV2Error::MetamaskError(MetamaskRpcError::MetamaskCtxNotInitialized))?;
            Ok(EthPrivKeyBuildPolicy::Metamask(metamask_ctx))
        },
    }
}

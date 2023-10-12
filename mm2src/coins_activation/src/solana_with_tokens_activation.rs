use std::collections::HashMap;

use async_trait::async_trait;
use coins::coin_errors::MyAddressError;
use coins::my_tx_history_v2::TxHistoryStorage;
use coins::solana::solana_coin_with_policy;
use coins::solana::spl::{SplProtocolConf, SplTokenCreationError};
use coins::{BalanceError, CoinBalance, CoinProtocol, MarketCoinOps, MmCoinEnum, PrivKeyBuildPolicy,
            SolanaActivationParams, SolanaCoin, SplToken};
use common::Future01CompatExt;
use common::{drop_mutability, true_f};
use crypto::CryptoCtxError;
use futures::future::try_join_all;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::platform_coin_with_tokens::{EnablePlatformCoinWithTokensError, GetPlatformBalance,
                                       InitTokensAsMmCoinsError, PlatformWithTokensActivationOps, RegisterTokenInfo,
                                       TokenActivationParams, TokenActivationRequest, TokenAsMmCoinInitializer,
                                       TokenInitializer, TokenOf};
use crate::prelude::*;
use crate::prelude::{CoinAddressInfo, TokenBalances, TryFromCoinProtocol, TxHistory};
use crate::spl_token_activation::SplActivationRequest;

pub struct SplTokenInitializer {
    platform_coin: SolanaCoin,
}

impl TokenOf for SplToken {
    type PlatformCoin = SolanaCoin;
}

pub struct SplTokenInitializerErr {
    ticker: String,
    inner: SplTokenCreationError,
}

#[async_trait]
impl TokenInitializer for SplTokenInitializer {
    type Token = SplToken;
    type TokenActivationRequest = SplActivationRequest;
    type TokenProtocol = SplProtocolConf;
    type InitTokensError = SplTokenInitializerErr;

    fn tokens_requests_from_platform_request(
        platform_params: &SolanaWithTokensActivationRequest,
    ) -> Vec<TokenActivationRequest<Self::TokenActivationRequest>> {
        platform_params.spl_tokens_requests.clone()
    }

    async fn enable_tokens(
        &self,
        activation_params: Vec<TokenActivationParams<SplActivationRequest, SplProtocolConf>>,
    ) -> Result<Vec<SplToken>, MmError<Self::InitTokensError>> {
        let tokens = activation_params
            .into_iter()
            .map(|param| {
                let ticker = param.ticker.clone();
                SplToken::new(
                    param.protocol.decimals,
                    param.ticker,
                    param.protocol.token_contract_address,
                    self.platform_coin.clone(),
                )
                .mm_err(|inner| SplTokenInitializerErr { ticker, inner })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tokens)
    }

    fn platform_coin(&self) -> &SolanaCoin { &self.platform_coin }
}

impl RegisterTokenInfo<SplToken> for SolanaCoin {
    fn register_token_info(&self, token: &SplToken) { self.add_spl_token_info(token.ticker().into(), token.get_info()) }
}

#[derive(Clone, Debug, Deserialize)]
pub struct SolanaWithTokensActivationRequest {
    #[serde(flatten)]
    platform_request: SolanaActivationParams,
    spl_tokens_requests: Vec<TokenActivationRequest<SplActivationRequest>>,
    #[serde(default = "true_f")]
    pub get_balances: bool,
}

impl TxHistory for SolanaWithTokensActivationRequest {
    fn tx_history(&self) -> bool { false }
}

#[derive(Debug, Serialize)]
pub struct SolanaWithTokensActivationResult {
    current_block: u64,
    solana_addresses_infos: HashMap<String, CoinAddressInfo<CoinBalance>>,
    spl_addresses_infos: HashMap<String, CoinAddressInfo<TokenBalances>>,
}

impl GetPlatformBalance for SolanaWithTokensActivationResult {
    fn get_platform_balance(&self) -> Option<BigDecimal> {
        self.solana_addresses_infos
            .iter()
            .fold(Some(BigDecimal::from(0)), |total, (_, addr_info)| {
                total.and_then(|t| addr_info.balances.as_ref().map(|b| t + b.get_total()))
            })
    }
}

impl CurrentBlock for SolanaWithTokensActivationResult {
    fn current_block(&self) -> u64 { self.current_block }
}

#[derive(Debug)]
pub enum SolanaWithTokensActivationError {
    PlatformCoinCreationError { ticker: String, error: String },
    UnableToRetrieveMyAddress(String),
    GetBalanceError(BalanceError),
    Transport(String),
    Internal(String),
}

impl From<MyAddressError> for SolanaWithTokensActivationError {
    fn from(err: MyAddressError) -> Self { Self::UnableToRetrieveMyAddress(err.to_string()) }
}

impl From<SolanaWithTokensActivationError> for EnablePlatformCoinWithTokensError {
    fn from(e: SolanaWithTokensActivationError) -> Self {
        match e {
            SolanaWithTokensActivationError::PlatformCoinCreationError { ticker, error } => {
                EnablePlatformCoinWithTokensError::PlatformCoinCreationError { ticker, error }
            },
            SolanaWithTokensActivationError::UnableToRetrieveMyAddress(e) => {
                EnablePlatformCoinWithTokensError::Internal(e)
            },
            SolanaWithTokensActivationError::GetBalanceError(e) => {
                EnablePlatformCoinWithTokensError::Internal(format!("{:?}", e))
            },
            SolanaWithTokensActivationError::Transport(e) => EnablePlatformCoinWithTokensError::Transport(e),
            SolanaWithTokensActivationError::Internal(e) => EnablePlatformCoinWithTokensError::Internal(e),
        }
    }
}

impl From<BalanceError> for SolanaWithTokensActivationError {
    fn from(e: BalanceError) -> Self { SolanaWithTokensActivationError::GetBalanceError(e) }
}

impl From<CryptoCtxError> for SolanaWithTokensActivationError {
    fn from(e: CryptoCtxError) -> Self { SolanaWithTokensActivationError::Internal(e.to_string()) }
}

pub struct SolanaProtocolInfo {}

impl TryFromCoinProtocol for SolanaProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::SOLANA {} => Ok(SolanaProtocolInfo {}),
            protocol => MmError::err(protocol),
        }
    }
}

impl From<SplTokenInitializerErr> for InitTokensAsMmCoinsError {
    fn from(err: SplTokenInitializerErr) -> Self {
        match err.inner {
            SplTokenCreationError::InvalidPubkey(error) => InitTokensAsMmCoinsError::TokenProtocolParseError {
                ticker: err.ticker,
                error,
            },
            SplTokenCreationError::Internal(internal) => InitTokensAsMmCoinsError::Internal(internal),
        }
    }
}

#[async_trait]
impl PlatformWithTokensActivationOps for SolanaCoin {
    type ActivationRequest = SolanaWithTokensActivationRequest;
    type PlatformProtocolInfo = SolanaProtocolInfo;
    type ActivationResult = SolanaWithTokensActivationResult;
    type ActivationError = SolanaWithTokensActivationError;

    async fn enable_platform_coin(
        ctx: MmArc,
        ticker: String,
        platform_conf: Json,
        activation_request: Self::ActivationRequest,
        _protocol_conf: Self::PlatformProtocolInfo,
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let priv_key_policy = PrivKeyBuildPolicy::detect_priv_key_policy(&ctx)?;
        solana_coin_with_policy(
            &ctx,
            &ticker,
            &platform_conf,
            activation_request.platform_request,
            priv_key_policy,
        )
        .await
        .map_to_mm(|error| SolanaWithTokensActivationError::PlatformCoinCreationError { ticker, error })
    }

    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized,
    {
        match coin {
            MmCoinEnum::SolanaCoin(coin) => Some(coin),
            _ => None,
        }
    }

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>> {
        vec![Box::new(SplTokenInitializer {
            platform_coin: self.clone(),
        })]
    }

    async fn get_activation_result(
        &self,
        activation_request: &Self::ActivationRequest,
    ) -> Result<Self::ActivationResult, MmError<Self::ActivationError>> {
        let current_block = self
            .current_block()
            .compat()
            .await
            .map_to_mm(Self::ActivationError::Internal)?;

        let my_address = self.my_address()?;

        let mut solana_address_info = CoinAddressInfo {
            derivation_method: DerivationMethod::Iguana,
            pubkey: my_address.clone(),
            balances: None,
            tickers: None,
        };

        let mut spl_address_info = CoinAddressInfo {
            derivation_method: DerivationMethod::Iguana,
            pubkey: my_address.clone(),
            balances: None,
            tickers: None,
        };

        if !activation_request.get_balances {
            drop_mutability!(solana_address_info);
            let tickers = self.get_spl_tokens_infos().into_keys().collect();
            spl_address_info.tickers = Some(tickers);
            drop_mutability!(spl_address_info);

            return Ok(SolanaWithTokensActivationResult {
                current_block,
                solana_addresses_infos: HashMap::from([(my_address.clone(), solana_address_info)]),
                spl_addresses_infos: HashMap::from([(my_address, spl_address_info)]),
            });
        }

        let solana_balance = self
            .my_balance()
            .compat()
            .await
            .map_err(|e| Self::ActivationError::GetBalanceError(e.into_inner()))?;
        solana_address_info.balances = Some(solana_balance);
        drop_mutability!(solana_address_info);

        let (token_tickers, requests): (Vec<_>, Vec<_>) = self
            .get_spl_tokens_infos()
            .into_iter()
            .map(|(ticker, info)| (ticker, self.my_balance_spl(info)))
            .unzip();
        spl_address_info.balances = Some(token_tickers.into_iter().zip(try_join_all(requests).await?).collect());
        drop_mutability!(spl_address_info);

        Ok(SolanaWithTokensActivationResult {
            current_block,
            solana_addresses_infos: HashMap::from([(my_address.clone(), solana_address_info)]),
            spl_addresses_infos: HashMap::from([(my_address, spl_address_info)]),
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

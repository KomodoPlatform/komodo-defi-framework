use crate::platform_coin_with_tokens::{EnablePlatformCoinWithTokensError, GetPlatformBalance,
                                       InitTokensAsMmCoinsError, PlatformWithTokensActivationOps, RegisterTokenInfo,
                                       TokenActivationParams, TokenActivationRequest, TokenAsMmCoinInitializer,
                                       TokenInitializer, TokenOf};
use crate::prelude::*;
use crate::prelude::{CoinAddressInfo, TokenBalances, TryFromCoinProtocol, TxHistory};
use crate::spl_token_activation::SplActivationRequest;
use async_trait::async_trait;
use coins::coin_errors::MyAddressError;
use coins::my_tx_history_v2::TxHistoryStorage;
use coins::solana::solana_coin_with_policy;
use coins::solana::spl::{SplProtocolConf, SplTokenCreationError};
use coins::{BalanceError, CoinBalance, CoinProtocol, MarketCoinOps, PrivKeyBuildPolicy, SolanaActivationParams,
            SolanaCoin, SplToken};
use common::Future01CompatExt;
use crypto::CryptoCtxError;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;

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
    fn get_platform_balance(&self) -> BigDecimal {
        self.solana_addresses_infos
            .iter()
            .fold(BigDecimal::from(0), |total, (_, addr_info)| {
                &total + &addr_info.balances.get_total()
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

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>> {
        vec![Box::new(SplTokenInitializer {
            platform_coin: self.clone(),
        })]
    }

    async fn get_activation_result(&self) -> Result<Self::ActivationResult, MmError<Self::ActivationError>> {
        let my_address = self.my_address()?;
        let current_block = self
            .current_block()
            .compat()
            .await
            .map_to_mm(Self::ActivationError::Internal)?;
        let solana_balance = self
            .my_balance()
            .compat()
            .await
            .map_err(|e| Self::ActivationError::GetBalanceError(e.into_inner()))?;
        let mut token_balances = HashMap::new();
        let token_infos = self.get_spl_tokens_infos();
        for (token_ticker, info) in token_infos.into_iter() {
            let balance = self.my_balance_spl(&info.clone()).await?;
            token_balances.insert(token_ticker.to_owned(), balance);
        }
        let mut result = SolanaWithTokensActivationResult {
            current_block,
            solana_addresses_infos: HashMap::new(),
            spl_addresses_infos: HashMap::new(),
        };
        result
            .solana_addresses_infos
            .insert(my_address.clone(), CoinAddressInfo {
                derivation_method: DerivationMethod::Iguana,
                pubkey: my_address.clone(),
                balances: solana_balance,
            });

        result.spl_addresses_infos.insert(my_address.clone(), CoinAddressInfo {
            derivation_method: DerivationMethod::Iguana,
            pubkey: my_address,
            balances: token_balances,
        });
        Ok(result)
    }

    fn start_history_background_fetching(
        &self,
        _ctx: MmArc,
        _storage: impl TxHistoryStorage + Send + 'static,
        _initial_balance: BigDecimal,
    ) {
    }
}

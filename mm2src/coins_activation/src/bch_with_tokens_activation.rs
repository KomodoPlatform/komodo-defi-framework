use crate::platform_coin_with_tokens::*;
use crate::prelude::*;
use crate::slp_token_activation::SlpActivationRequest;
use async_trait::async_trait;
use coins::my_tx_history_v2::TxHistoryStorage;
use coins::utxo::bch::{bch_coin_with_policy, BchActivationRequest, BchCoin, CashAddrPrefix};
use coins::utxo::rpc_clients::UtxoRpcError;
use coins::utxo::slp::{EnableSlpError, SlpProtocolConf, SlpToken};
use coins::utxo::utxo_tx_history_v2::bch_and_slp_history_loop;
use coins::utxo::UtxoCommonOps;
use coins::{CoinBalance, CoinProtocol, MarketCoinOps, MmCoin, PrivKeyBuildPolicy, PrivKeyPolicyNotAllowed,
            UnexpectedDerivationMethod};
use common::executor::{AbortSettings, SpawnAbortable};
use common::Future01CompatExt;
use common::{drop_mutability, true_f};
use crypto::CryptoCtxError;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

impl From<EnableSlpError> for InitTokensAsMmCoinsError {
    fn from(e: EnableSlpError) -> Self {
        match e {
            EnableSlpError::GetBalanceError(balance_err) => {
                InitTokensAsMmCoinsError::CouldNotFetchBalance(balance_err.to_string())
            },
            EnableSlpError::UnexpectedDerivationMethod(internal) | EnableSlpError::Internal(internal) => {
                InitTokensAsMmCoinsError::Internal(internal)
            },
        }
    }
}

pub struct SlpTokenInitializer {
    platform_coin: BchCoin,
}

impl TokenOf for SlpToken {
    type PlatformCoin = BchCoin;
}

#[async_trait]
impl TokenInitializer for SlpTokenInitializer {
    type Token = SlpToken;
    type TokenActivationRequest = SlpActivationRequest;
    type TokenProtocol = SlpProtocolConf;
    type InitTokensError = EnableSlpError;

    fn tokens_requests_from_platform_request(
        platform_params: &BchWithTokensActivationRequest,
    ) -> Vec<TokenActivationRequest<Self::TokenActivationRequest>> {
        platform_params.slp_tokens_requests.clone()
    }

    async fn enable_tokens(
        &self,
        activation_params: Vec<TokenActivationParams<SlpActivationRequest, SlpProtocolConf>>,
    ) -> Result<Vec<SlpToken>, MmError<EnableSlpError>> {
        let tokens = activation_params
            .into_iter()
            .map(|params| {
                // confirmation settings from RPC request have the highest priority
                let required_confirmations = params.activation_request.required_confirmations.unwrap_or_else(|| {
                    params
                        .protocol
                        .required_confirmations
                        .unwrap_or_else(|| self.platform_coin.required_confirmations())
                });

                SlpToken::new(
                    params.protocol.decimals,
                    params.ticker,
                    params.protocol.token_id,
                    self.platform_coin.clone(),
                    required_confirmations,
                )
            })
            .collect::<MmResult<_, EnableSlpError>>()?;

        Ok(tokens)
    }

    fn platform_coin(&self) -> &BchCoin { &self.platform_coin }
}

impl RegisterTokenInfo<SlpToken> for BchCoin {
    fn register_token_info(&self, token: &SlpToken) { self.add_slp_token_info(token.ticker().into(), token.get_info()) }
}

impl From<BchWithTokensActivationError> for EnablePlatformCoinWithTokensError {
    fn from(err: BchWithTokensActivationError) -> Self {
        match err {
            BchWithTokensActivationError::PlatformCoinCreationError { ticker, error } => {
                EnablePlatformCoinWithTokensError::PlatformCoinCreationError { ticker, error }
            },
            BchWithTokensActivationError::InvalidSlpPrefix { ticker, prefix, error } => {
                EnablePlatformCoinWithTokensError::Internal(format!(
                    "Invalid slp prefix {} configured for {}. Error: {}",
                    prefix, ticker, error
                ))
            },
            BchWithTokensActivationError::PrivKeyPolicyNotAllowed(e) => {
                EnablePlatformCoinWithTokensError::PrivKeyPolicyNotAllowed(e)
            },
            BchWithTokensActivationError::UnexpectedDerivationMethod(e) => {
                EnablePlatformCoinWithTokensError::UnexpectedDerivationMethod(e)
            },
            BchWithTokensActivationError::Transport(e) => EnablePlatformCoinWithTokensError::Transport(e),
            BchWithTokensActivationError::Internal(e) => EnablePlatformCoinWithTokensError::Internal(e),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct BchWithTokensActivationRequest {
    #[serde(flatten)]
    platform_request: BchActivationRequest,
    slp_tokens_requests: Vec<TokenActivationRequest<SlpActivationRequest>>,
    #[serde(default = "true_f")]
    pub get_balances: bool,
}

impl TxHistory for BchWithTokensActivationRequest {
    fn tx_history(&self) -> bool { self.platform_request.utxo_params.tx_history }
}

pub struct BchProtocolInfo {
    slp_prefix: String,
}

impl TryFromCoinProtocol for BchProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::BCH { slp_prefix } => Ok(BchProtocolInfo { slp_prefix }),
            protocol => MmError::err(protocol),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BchWithTokensActivationResult {
    current_block: u64,
    bch_addresses_infos: HashMap<String, CoinAddressInfo<CoinBalance>>,
    slp_addresses_infos: HashMap<String, CoinAddressInfo<TokenBalances>>,
}

impl GetPlatformBalance for BchWithTokensActivationResult {
    fn get_platform_balance(&self) -> Option<BigDecimal> {
        self.bch_addresses_infos
            .iter()
            .fold(Some(BigDecimal::from(0)), |total, (_, addr_info)| {
                total.and_then(|t| addr_info.balances.as_ref().map(|b| t + b.get_total()))
            })
    }
}

impl CurrentBlock for BchWithTokensActivationResult {
    fn current_block(&self) -> u64 { self.current_block }
}

#[derive(Debug)]
pub enum BchWithTokensActivationError {
    PlatformCoinCreationError {
        ticker: String,
        error: String,
    },
    InvalidSlpPrefix {
        ticker: String,
        prefix: String,
        error: String,
    },
    PrivKeyPolicyNotAllowed(PrivKeyPolicyNotAllowed),
    UnexpectedDerivationMethod(String),
    Transport(String),
    Internal(String),
}

impl From<UtxoRpcError> for BchWithTokensActivationError {
    fn from(err: UtxoRpcError) -> Self { BchWithTokensActivationError::Transport(err.to_string()) }
}

impl From<UnexpectedDerivationMethod> for BchWithTokensActivationError {
    fn from(e: UnexpectedDerivationMethod) -> Self {
        BchWithTokensActivationError::UnexpectedDerivationMethod(e.to_string())
    }
}

impl From<PrivKeyPolicyNotAllowed> for BchWithTokensActivationError {
    fn from(e: PrivKeyPolicyNotAllowed) -> Self { BchWithTokensActivationError::PrivKeyPolicyNotAllowed(e) }
}

impl From<CryptoCtxError> for BchWithTokensActivationError {
    fn from(e: CryptoCtxError) -> Self { BchWithTokensActivationError::Internal(e.to_string()) }
}

#[async_trait]
impl PlatformWithTokensActivationOps for BchCoin {
    type ActivationRequest = BchWithTokensActivationRequest;
    type PlatformProtocolInfo = BchProtocolInfo;
    type ActivationResult = BchWithTokensActivationResult;
    type ActivationError = BchWithTokensActivationError;

    async fn enable_platform_coin(
        ctx: MmArc,
        ticker: String,
        platform_conf: Json,
        activation_request: Self::ActivationRequest,
        protocol_conf: Self::PlatformProtocolInfo,
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let priv_key_policy = PrivKeyBuildPolicy::detect_priv_key_policy(&ctx)?;

        let slp_prefix = CashAddrPrefix::from_str(&protocol_conf.slp_prefix).map_to_mm(|error| {
            BchWithTokensActivationError::InvalidSlpPrefix {
                ticker: ticker.clone(),
                prefix: protocol_conf.slp_prefix,
                error,
            }
        })?;

        let platform_coin = bch_coin_with_policy(
            &ctx,
            &ticker,
            &platform_conf,
            activation_request.platform_request,
            slp_prefix,
            priv_key_policy,
        )
        .await
        .map_to_mm(|error| BchWithTokensActivationError::PlatformCoinCreationError { ticker, error })?;
        Ok(platform_coin)
    }

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>> {
        vec![Box::new(SlpTokenInitializer {
            platform_coin: self.clone(),
        })]
    }

    async fn get_activation_result(
        &self,
        activation_request: &Self::ActivationRequest,
    ) -> Result<BchWithTokensActivationResult, MmError<BchWithTokensActivationError>> {
        let current_block = self.as_ref().rpc_client.get_block_count().compat().await?;

        let my_address = self.as_ref().derivation_method.single_addr_or_err()?;
        let my_slp_address = self
            .get_my_slp_address()
            .map_to_mm(BchWithTokensActivationError::Internal)?
            .encode()
            .map_to_mm(BchWithTokensActivationError::Internal)?;
        let pubkey = self.my_public_key()?.to_string();

        let mut bch_address_info = CoinAddressInfo {
            derivation_method: DerivationMethod::Iguana,
            pubkey: pubkey.clone(),
            balances: None,
            tickers: None,
        };

        let mut slp_address_info = CoinAddressInfo {
            derivation_method: DerivationMethod::Iguana,
            pubkey: pubkey.clone(),
            balances: None,
            tickers: None,
        };

        if !activation_request.get_balances {
            drop_mutability!(bch_address_info);
            let tickers: HashSet<_> = self.get_slp_tokens_infos().keys().cloned().collect();
            slp_address_info.tickers = Some(tickers);
            drop_mutability!(slp_address_info);

            return Ok(BchWithTokensActivationResult {
                current_block,
                bch_addresses_infos: HashMap::from([(my_address.to_string(), bch_address_info)]),
                slp_addresses_infos: HashMap::from([(my_slp_address, slp_address_info)]),
            });
        }

        let bch_unspents = self.bch_unspents_for_display(my_address).await?;
        bch_address_info.balances = Some(bch_unspents.platform_balance(self.decimals()));
        drop_mutability!(bch_address_info);

        let token_balances: HashMap<_, _> = self
            .get_slp_tokens_infos()
            .iter()
            .map(|(token_ticker, info)| {
                let token_balance = bch_unspents.slp_token_balance(&info.token_id, info.decimals);
                (token_ticker.clone(), token_balance)
            })
            .collect();
        slp_address_info.balances = Some(token_balances);
        drop_mutability!(slp_address_info);

        Ok(BchWithTokensActivationResult {
            current_block,
            bch_addresses_infos: HashMap::from([(my_address.to_string(), bch_address_info)]),
            slp_addresses_infos: HashMap::from([(my_slp_address, slp_address_info)]),
        })
    }

    fn start_history_background_fetching(
        &self,
        ctx: MmArc,
        storage: impl TxHistoryStorage + Send + 'static,
        initial_balance: Option<BigDecimal>,
    ) {
        let fut = bch_and_slp_history_loop(self.clone(), storage, ctx.metrics.clone(), initial_balance);

        let settings = AbortSettings::info_on_abort(format!("bch_and_slp_history_loop stopped for {}", self.ticker()));
        self.spawner().spawn_with_settings(fut, settings);
    }
}

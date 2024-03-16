use crate::context::CoinsActivationContext;
use crate::platform_coin_with_tokens::{EnablePlatformCoinWithTokensError, GetPlatformBalance,
                                       InitPlatformCoinWithTokensAwaitingStatus,
                                       InitPlatformCoinWithTokensInProgressStatus,
                                       InitPlatformCoinWithTokensTaskManagerShared,
                                       InitPlatformCoinWithTokensUserAction, InitTokensAsMmCoinsError,
                                       PlatformCoinWithTokensActivationOps, RegisterTokenInfo, TokenActivationParams,
                                       TokenActivationRequest, TokenAsMmCoinInitializer, TokenInitializer, TokenOf};
use crate::prelude::*;
use async_trait::async_trait;
use coins::coin_balance::{CoinBalanceReport, EnableCoinBalanceOps};
use coins::eth::v2_activation::{eth_coin_from_conf_and_request_v2, Erc20Protocol, Erc20TokenActivationRequest,
                                EthActivationV2Error, EthActivationV2Request, EthPrivKeyActivationPolicy};
use coins::eth::v2_activation::{EthTokenActivationError, NftActivationRequest, NftProviderEnum};
use coins::eth::{Erc20TokenInfo, EthCoin, EthCoinType, EthPrivKeyBuildPolicy};
use coins::hd_wallet::RpcTaskXPubExtractor;
use coins::my_tx_history_v2::TxHistoryStorage;
use coins::nft::nft_structs::NftInfo;
use coins::{CoinProtocol, MarketCoinOps, MmCoin, MmCoinEnum};

use crate::platform_coin_with_tokens::InitPlatformCoinWithTokensTask;
use common::true_f;
use common::Future01CompatExt;
use crypto::hw_rpc_task::HwConnectStatuses;
use crypto::HwRpcError;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_event_stream::EventStreamConfiguration;
#[cfg(target_arch = "wasm32")]
use mm2_metamask::MetamaskRpcError;
use mm2_number::BigDecimal;
use rpc_task::RpcTaskHandleShared;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;

impl From<EthActivationV2Error> for EnablePlatformCoinWithTokensError {
    fn from(err: EthActivationV2Error) -> Self {
        match err {
            EthActivationV2Error::InvalidPayload(e)
            | EthActivationV2Error::InvalidSwapContractAddr(e)
            | EthActivationV2Error::InvalidFallbackSwapContract(e)
            | EthActivationV2Error::ErrorDeserializingDerivationPath(e) => {
                EnablePlatformCoinWithTokensError::InvalidPayload(e)
            },
            EthActivationV2Error::InvalidPathToAddress(e) => EnablePlatformCoinWithTokensError::InvalidPayload(e),
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
            EthActivationV2Error::PrivKeyPolicyNotAllowed(e) => {
                EnablePlatformCoinWithTokensError::PrivKeyPolicyNotAllowed(e)
            },
            EthActivationV2Error::FailedSpawningBalanceEvents(e) => {
                EnablePlatformCoinWithTokensError::FailedSpawningBalanceEvents(e)
            },
            EthActivationV2Error::HDWalletStorageError(e) => EnablePlatformCoinWithTokensError::Internal(e),
            #[cfg(target_arch = "wasm32")]
            EthActivationV2Error::MetamaskError(metamask) => {
                EnablePlatformCoinWithTokensError::Transport(metamask.to_string())
            },
            EthActivationV2Error::InternalError(e) => EnablePlatformCoinWithTokensError::Internal(e),
            EthActivationV2Error::Transport(e) => EnablePlatformCoinWithTokensError::Transport(e),
            EthActivationV2Error::UnexpectedDerivationMethod(e) => {
                EnablePlatformCoinWithTokensError::UnexpectedDerivationMethod(e.to_string())
            },
            EthActivationV2Error::HwContextNotInitialized => {
                EnablePlatformCoinWithTokensError::Internal("Hardware wallet is not initalised".to_string())
            },
            EthActivationV2Error::CoinDoesntSupportTrezor => {
                EnablePlatformCoinWithTokensError::Internal("Coin does not support Trezor wallet".to_string())
            },
            EthActivationV2Error::TaskTimedOut { .. } => {
                EnablePlatformCoinWithTokensError::Internal("Coin activation timed out".to_string())
            },
            EthActivationV2Error::HwError(e) => EnablePlatformCoinWithTokensError::Internal(e.to_string()),
            EthActivationV2Error::InvalidHardwareWalletCall => EnablePlatformCoinWithTokensError::Internal(
                "Hardware wallet must be used within rpc task manager".to_string(),
            ),
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

impl From<EthTokenActivationError> for InitTokensAsMmCoinsError {
    fn from(error: EthTokenActivationError) -> Self {
        match error {
            EthTokenActivationError::InternalError(e) => InitTokensAsMmCoinsError::Internal(e),
            EthTokenActivationError::CouldNotFetchBalance(e) | EthTokenActivationError::ClientConnectionFailed(e) => {
                InitTokensAsMmCoinsError::CouldNotFetchBalance(e)
            },
            EthTokenActivationError::InvalidPayload(e) => InitTokensAsMmCoinsError::InvalidPayload(e),
            EthTokenActivationError::Transport(e) => InitTokensAsMmCoinsError::Transport(e),
            EthTokenActivationError::UnexpectedDerivationMethod(e) => {
                InitTokensAsMmCoinsError::UnexpectedDerivationMethod(e)
            },
        }
    }
}

#[async_trait]
impl TokenInitializer for Erc20Initializer {
    type Token = EthCoin;
    type TokenActivationRequest = Erc20TokenActivationRequest;
    type TokenProtocol = Erc20Protocol;
    type InitTokensError = EthTokenActivationError;

    fn tokens_requests_from_platform_request(
        platform_params: &EthWithTokensActivationRequest,
    ) -> Vec<TokenActivationRequest<Self::TokenActivationRequest>> {
        platform_params.erc20_tokens_requests.clone()
    }

    async fn enable_tokens(
        &self,
        activation_params: Vec<TokenActivationParams<Erc20TokenActivationRequest, Erc20Protocol>>,
    ) -> Result<Vec<EthCoin>, MmError<EthTokenActivationError>> {
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
    nft_req: Option<NftActivationRequest>,
}

impl TxHistory for EthWithTokensActivationRequest {
    fn tx_history(&self) -> bool { false }
}

impl ActivationRequestInfo for EthWithTokensActivationRequest {
    fn is_hw_policy(&self) -> bool { self.platform_request.priv_key_policy.is_hw_policy() }
}

impl TokenOf for EthCoin {
    type PlatformCoin = EthCoin;
}

impl RegisterTokenInfo<EthCoin> for EthCoin {
    fn register_token_info(&self, token: &EthCoin) {
        // Dont register Nft in platform coin.
        if matches!(token.coin_type, EthCoinType::Nft { .. }) {
            return;
        }

        self.add_erc_token_info(token.ticker().to_string(), Erc20TokenInfo {
            token_address: token.erc20_token_address().unwrap(),
            decimals: token.decimals(),
        });
    }
}

/// Represents the result of activating an Ethereum-based coin along with its associated tokens (ERC20 and NFTs).
///
/// This structure provides a snapshot of the relevant activation data, including the current blockchain block,
/// information about Ethereum addresses and their balances, ERC-20 token balances, and a summary of NFT ownership.
#[derive(Serialize, Clone)]
pub struct EthWithTokensActivationResult {
    current_block: u64,
    ticker: String,
    wallet_balance: CoinBalanceReport,
    // Todo: should be part of wallet_balance or not?
    nfts_infos: HashMap<String, NftInfo>,
}

impl GetPlatformBalance for EthWithTokensActivationResult {
    fn get_platform_balance(&self) -> Option<BigDecimal> {
        self.wallet_balance
            .to_addresses_total_balances(&self.ticker)
            .iter()
            .fold(None, |maybe_total, (_, maybe_balance)| {
                match (maybe_total, maybe_balance) {
                    (Some(total), Some(balance)) => Some(total + balance),
                    (None, Some(balance)) => Some(balance.clone()),
                    (total, None) => total,
                }
            })
    }
}

impl CurrentBlock for EthWithTokensActivationResult {
    fn current_block(&self) -> u64 { self.current_block }
}

#[async_trait]
impl PlatformCoinWithTokensActivationOps for EthCoin {
    type ActivationRequest = EthWithTokensActivationRequest;
    type PlatformProtocolInfo = EthCoinType;
    type ActivationResult = EthWithTokensActivationResult;
    type ActivationError = EthActivationV2Error;

    type InProgressStatus = InitPlatformCoinWithTokensInProgressStatus;
    type AwaitingStatus = InitPlatformCoinWithTokensAwaitingStatus;
    type UserAction = InitPlatformCoinWithTokensUserAction;

    async fn enable_platform_coin(
        ctx: MmArc,
        ticker: String,
        platform_conf: &Json,
        activation_request: Self::ActivationRequest,
        _protocol: Self::PlatformProtocolInfo,
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let priv_key_policy = eth_priv_key_build_policy(&ctx, &activation_request.platform_request.priv_key_policy)?;

        let platform_coin = eth_coin_from_conf_and_request_v2(
            &ctx,
            &ticker,
            platform_conf,
            activation_request.platform_request,
            priv_key_policy,
        )
        .await?;

        Ok(platform_coin)
    }

    async fn enable_global_nft(
        &self,
        activation_request: &Self::ActivationRequest,
    ) -> Result<Option<MmCoinEnum>, MmError<Self::ActivationError>> {
        let url = match &activation_request.nft_req {
            Some(nft_req) => match &nft_req.provider {
                NftProviderEnum::Moralis { url } => url,
            },
            None => return Ok(None),
        };
        let nft_global = self.global_nft_from_platform_coin(url).await?;
        Ok(Some(MmCoinEnum::EthCoin(nft_global)))
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
        task_handle: Option<RpcTaskHandleShared<InitPlatformCoinWithTokensTask<EthCoin>>>,
        activation_request: &Self::ActivationRequest,
        nft_global: &Option<MmCoinEnum>,
    ) -> Result<EthWithTokensActivationResult, MmError<EthActivationV2Error>> {
        let current_block = self
            .current_block()
            .compat()
            .await
            .map_err(EthActivationV2Error::InternalError)?;

        let xpub_extractor = if self.is_trezor() {
            let ctx = MmArc::from_weak(&self.ctx).ok_or(EthActivationV2Error::InvalidHardwareWalletCall)?;
            let task_handle = task_handle.ok_or_else(|| {
                EthActivationV2Error::InternalError("Hardware wallet must be accessed under task manager".to_string())
            })?;
            Some(
                RpcTaskXPubExtractor::new_trezor_extractor(
                    &ctx,
                    task_handle,
                    eth_xpub_extractor_rpc_statuses(),
                    CoinProtocol::ETH,
                )
                .map_err(|_| MmError::new(EthActivationV2Error::HwError(HwRpcError::NotInitialized)))?,
            )
        } else {
            None
        };

        let wallet_balance = self
            .enable_coin_balance(
                xpub_extractor,
                activation_request.platform_request.enable_params.clone(),
                &activation_request.platform_request.path_to_address,
            )
            .await
            .mm_err(|e| EthActivationV2Error::InternalError(e.to_string()))?;

        let nfts_map = if let Some(MmCoinEnum::EthCoin(nft_global)) = nft_global {
            nft_global.nfts_infos.lock().await.clone()
        } else {
            Default::default()
        };

        // Todo: fix get_balances
        // if !activation_request.get_balances {
        //     drop_mutability!(eth_address_info);
        //     let tickers: HashSet<_> = self.get_erc_tokens_infos().into_keys().collect();
        //     erc20_address_info.tickers = Some(tickers);
        //     drop_mutability!(erc20_address_info);
        //
        //     return Ok(EthWithTokensActivationResult {
        //         current_block,
        //         eth_addresses_infos: HashMap::from([(my_address.clone(), eth_address_info)]),
        //         erc20_addresses_infos: HashMap::from([(my_address, erc20_address_info)]),
        //         nfts_infos: nfts_map,
        //     });
        // }

        // let eth_balance = self
        //     .my_balance()
        //     .compat()
        //     .await
        //     .map_err(|e| EthActivationV2Error::CouldNotFetchBalance(e.to_string()))?;
        // eth_address_info.balances = Some(eth_balance);
        // drop_mutability!(eth_address_info);

        // Todo: recheck this
        // Todo: get_tokens_balance_list use get_token_balance_by_address that uses the enabled address
        // Todo: We should pass and address to this function so that we can get balances for all HD wallet enabled addresses on activation
        // let token_balances = self
        //     .get_tokens_balance_list()
        //     .await
        //     .map_err(|e| EthActivationV2Error::CouldNotFetchBalance(e.to_string()))?;
        // erc20_address_info.balances = Some(token_balances);
        // drop_mutability!(erc20_address_info);

        Ok(EthWithTokensActivationResult {
            current_block,
            ticker: self.ticker().to_string(),
            wallet_balance,
            nfts_infos: nfts_map,
        })
    }

    fn start_history_background_fetching(
        &self,
        _ctx: MmArc,
        _storage: impl TxHistoryStorage + Send + 'static,
        _initial_balance: Option<BigDecimal>,
    ) {
    }

    async fn handle_balance_streaming(
        &self,
        _config: &EventStreamConfiguration,
    ) -> Result<(), MmError<Self::ActivationError>> {
        Ok(())
    }

    fn rpc_task_manager(
        activation_ctx: &CoinsActivationContext,
    ) -> &InitPlatformCoinWithTokensTaskManagerShared<EthCoin> {
        &activation_ctx.init_eth_task_manager
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
        EthPrivKeyActivationPolicy::Trezor => Ok(EthPrivKeyBuildPolicy::Trezor),
    }
}

pub type EthTaskManagerShared = InitPlatformCoinWithTokensTaskManagerShared<EthCoin>;

pub(crate) fn eth_xpub_extractor_rpc_statuses(
) -> HwConnectStatuses<InitPlatformCoinWithTokensInProgressStatus, InitPlatformCoinWithTokensAwaitingStatus> {
    HwConnectStatuses {
        on_connect: InitPlatformCoinWithTokensInProgressStatus::WaitingForTrezorToConnect,
        on_connected: InitPlatformCoinWithTokensInProgressStatus::ActivatingCoin,
        on_connection_failed: InitPlatformCoinWithTokensInProgressStatus::Finishing,
        on_button_request: InitPlatformCoinWithTokensInProgressStatus::FollowHwDeviceInstructions,
        on_pin_request: InitPlatformCoinWithTokensAwaitingStatus::EnterTrezorPin,
        on_passphrase_request: InitPlatformCoinWithTokensAwaitingStatus::EnterTrezorPassphrase,
        on_ready: InitPlatformCoinWithTokensInProgressStatus::ActivatingCoin,
    }
}

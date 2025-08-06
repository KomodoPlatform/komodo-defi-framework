use async_trait::async_trait;
use coins::{my_tx_history_v2::TxHistoryStorage, solana::SolanaCoin, MmCoinEnum};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;
use mm2_number::BigDecimal;
use rpc_task::RpcTaskHandleShared;

use crate::{
    context::CoinsActivationContext,
    platform_coin_with_tokens::{
        InitPlatformCoinWithTokensTask, InitPlatformCoinWithTokensTaskManagerShared,
        PlatformCoinWithTokensActivationOps, TokenAsMmCoinInitializer,
    },
};

#[async_trait]
impl PlatformCoinWithTokensActivationOps for SolanaCoin {
    type ActivationRequest;
    type PlatformProtocolInfo;
    type ActivationResult;
    type ActivationError;

    type InProgressStatus;
    type AwaitingStatus;
    type UserAction;

    async fn enable_platform_coin(
        ctx: MmArc,
        ticker: String,
        coin_conf: &serde_json::Value,
        activation_request: Self::ActivationRequest,
        protocol_conf: Self::PlatformProtocolInfo,
    ) -> Result<Self, MmError<Self::ActivationError>> {
        todo!()
    }

    async fn enable_global_nft(
        &self,
        _activation_request: &Self::ActivationRequest,
    ) -> Result<Option<MmCoinEnum>, MmError<Self::ActivationError>> {
        todo!()
    }

    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized,
    {
        todo!()
    }

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>> {
        todo!()
    }

    async fn get_activation_result(
        &self,
        _task_handle: Option<RpcTaskHandleShared<InitPlatformCoinWithTokensTask<SolanaCoin>>>,
        activation_request: &Self::ActivationRequest,
        _nft_global: &Option<MmCoinEnum>,
    ) -> Result<Self::ActivationResult, MmError<Self::ActivationError>> {
        todo!()
    }

    fn start_history_background_fetching(
        &self,
        ctx: MmArc,
        storage: impl TxHistoryStorage,
        initial_balance: Option<BigDecimal>,
    ) {
        todo!()
    }

    fn rpc_task_manager(
        activation_ctx: &CoinsActivationContext,
    ) -> &InitPlatformCoinWithTokensTaskManagerShared<SolanaCoin> {
        todo!()
    }
}

use crate::mm2::lp_native_dex::init_context::MmInitContext;
use crate::mm2::lp_native_dex::{lp_init_continue, MmInitError, MmInitResult};
use async_trait::async_trait;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::SuccessResponse;
use crypto::hw_rpc_task::{HwConnectStatuses, HwRpcTaskAwaitingStatus, HwRpcTaskUserAction,
                          TrezorRpcTaskConnectProcessor};
use crypto::{CryptoCtx, HwWalletType};
use rpc_task::{RpcTask, RpcTaskHandle, RpcTaskManager, RpcTaskManagerShared, RpcTaskStatus, RpcTaskTypes};
use serde_json as json;
use std::time::Duration;

const MM_TREZOR_CONNECT_TIMEOUT: Duration = Duration::from_secs(300);
const MM_INIT_TREZOR_PIN_TIMEOUT: Duration = Duration::from_secs(600);

pub type MmInitAwaitingStatus = HwRpcTaskAwaitingStatus;
pub type MmInitUserAction = HwRpcTaskUserAction;

pub type MmInitTaskManagerShared = RpcTaskManagerShared<MmInitTask>;
pub type MmInitStatus = RpcTaskStatus<SuccessResponse, MmInitError, MmInitInProgressStatus, MmInitAwaitingStatus>;
type MmInitTaskHandle = RpcTaskHandle<MmInitTask>;

#[derive(Clone, Deserialize, Serialize)]
pub enum MmInitInProgressStatus {
    /// TODO replace with more specific statuses.
    Initializing,
    WaitingForTrezorToConnect,
    InitializingCryptoCtx,
    ReadPublicKeyFromTrezor,
}

pub struct MmInitTask {
    ctx: MmArc,
}

impl RpcTaskTypes for MmInitTask {
    type Item = SuccessResponse;
    type Error = MmInitError;
    type InProgressStatus = MmInitInProgressStatus;
    type AwaitingStatus = MmInitAwaitingStatus;
    type UserAction = MmInitUserAction;
}

#[async_trait]
impl RpcTask for MmInitTask {
    fn initial_status(&self) -> Self::InProgressStatus { MmInitInProgressStatus::InitializingCryptoCtx }

    async fn run(self, task_handle: &MmInitTaskHandle) -> Result<Self::Item, MmError<Self::Error>> {
        if self.ctx.conf["hw_wallet"].is_null() {
            return MmError::err(MmInitError::FieldNotFoundInConfig {
                field: "hw_wallet".to_owned(),
            });
        }
        let hw_wallet: HwWalletType = json::from_value(self.ctx.conf["hw_wallet"].clone()).map_to_mm(|e| {
            MmInitError::ErrorDeserializingConfig {
                field: "hw_wallet".to_owned(),
                error: e.to_string(),
            }
        })?;
        match hw_wallet {
            HwWalletType::Trezor => {
                let trezor_connect_processor = TrezorRpcTaskConnectProcessor::new(task_handle, HwConnectStatuses {
                    on_connect: MmInitInProgressStatus::WaitingForTrezorToConnect,
                    on_connected: MmInitInProgressStatus::Initializing,
                    on_connection_failed: MmInitInProgressStatus::Initializing,
                    on_button_request: MmInitInProgressStatus::ReadPublicKeyFromTrezor,
                    on_pin_request: MmInitAwaitingStatus::WaitForTrezorPin,
                    on_ready: MmInitInProgressStatus::Initializing,
                })
                .with_connect_timeout(MM_TREZOR_CONNECT_TIMEOUT)
                .with_pin_timeout(MM_INIT_TREZOR_PIN_TIMEOUT);

                CryptoCtx::init_with_trezor(self.ctx.weak(), &trezor_connect_processor).await?;
            },
        }

        lp_init_continue(self.ctx.clone()).await.map(|_| SuccessResponse::new())
    }
}

impl MmInitTask {
    pub fn new(ctx: MmArc) -> MmInitTask { MmInitTask { ctx } }

    /// # Panic
    ///
    /// Panic if the MarketMaker instance is initialized already.
    pub fn spawn(self) -> MmInitResult<()> {
        let init_ctx = MmInitContext::from_ctx(&self.ctx).map_to_mm(MmInitError::Internal)?;
        let task_id = RpcTaskManager::spawn_rpc_task(&init_ctx.mm_init_task_manager, self)?;
        init_ctx
            .mm_init_task_id
            .pin(task_id)
            .expect("MarketMaker initialization task has been spawned already");
        Ok(())
    }
}

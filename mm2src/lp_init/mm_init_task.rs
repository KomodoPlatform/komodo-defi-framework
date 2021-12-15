use crate::mm2::lp_native_dex::init_context::MmInitContext;
use crate::mm2::lp_native_dex::{lp_init_continue, MmInitError, MmInitResult};
use async_trait::async_trait;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::SuccessResponse;
use crypto::hw_rpc_task::{TrezorConnectStatuses, TrezorRpcTaskConnectProcessor};
use crypto::trezor::TrezorPinMatrix3x3Response;
use crypto::{CryptoCtx, HwWalletType};
use rpc_task::{RpcTask, RpcTaskError, RpcTaskHandle, RpcTaskManager, RpcTaskManagerShared, RpcTaskStatus};
use serde_json as json;
use std::convert::TryFrom;
use std::time::Duration;

const MM_TREZOR_CONNECT_TIMEOUT: Duration = Duration::from_secs(300);
const MM_INIT_TREZOR_PIN_TIMEOUT: Duration = Duration::from_secs(600);

pub type MmInitTaskManager =
    RpcTaskManager<SuccessResponse, MmInitError, MmInitInProgressStatus, MmInitAwaitingStatus, MmInitUserAction>;
pub type MmInitTaskManagerArc =
    RpcTaskManagerShared<SuccessResponse, MmInitError, MmInitInProgressStatus, MmInitAwaitingStatus, MmInitUserAction>;
pub type MmInitStatus = RpcTaskStatus<SuccessResponse, MmInitError, MmInitInProgressStatus, MmInitAwaitingStatus>;
type MmInitTaskHandle =
    RpcTaskHandle<SuccessResponse, MmInitError, MmInitInProgressStatus, MmInitAwaitingStatus, MmInitUserAction>;

#[derive(Clone, Deserialize, Serialize)]
pub enum MmInitInProgressStatus {
    /// TODO replace with more specific statuses.
    Initializing,
    WaitingForTrezorToConnect,
    InitializingCryptoCtx,
    ReadPublicKeyFromTrezor,
}

#[derive(Clone, Deserialize, Serialize)]
pub enum MmInitAwaitingStatus {
    WaitForTrezorPin,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "action_type")]
pub enum MmInitUserAction {
    TrezorPin(TrezorPinMatrix3x3Response),
}

impl TryFrom<MmInitUserAction> for TrezorPinMatrix3x3Response {
    type Error = RpcTaskError;

    fn try_from(value: MmInitUserAction) -> Result<Self, Self::Error> {
        match value {
            MmInitUserAction::TrezorPin(pin) => Ok(pin),
        }
    }
}

pub struct MmInitTask {
    ctx: MmArc,
}

#[async_trait]
impl RpcTask for MmInitTask {
    type Item = SuccessResponse;
    type Error = MmInitError;
    type InProgressStatus = MmInitInProgressStatus;
    type AwaitingStatus = MmInitAwaitingStatus;
    type UserAction = MmInitUserAction;

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
                let trezor_connect_processor = TrezorRpcTaskConnectProcessor::new(task_handle, TrezorConnectStatuses {
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
        let task_id = MmInitTaskManager::spawn_rpc_task(&init_ctx.mm_init_task_manager, self)?;
        init_ctx
            .mm_init_task_id
            .pin(task_id)
            .expect("MarketMaker initialization task has been spawned already");
        Ok(())
    }
}

use crate::hw_client::{HwProcessingError, TrezorConnectProcessor};
use crate::trezor::TrezorPinMatrix3x3Response;
use async_trait::async_trait;
use common::mm_error::prelude::*;
use rpc_task::rpc_common::RpcTaskUserActionRequest;
use serde::Serialize;
use std::convert::{TryFrom, TryInto};
use std::time::Duration;
use trezor::trezor_rpc_task::{RpcTaskError, RpcTaskHandle, TrezorRequestStatuses, TrezorRpcTaskProcessor};
use trezor::{TrezorProcessingError, TrezorRequestProcessor};

const CONNECT_DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

pub type HwRpcTaskUserActionRequest = RpcTaskUserActionRequest<HwRpcTaskUserAction>;

/// When it comes to interacting with a HW device, this is a common awaiting RPC status.
/// The status says to the user that he should pass a Trezor PIN to continue the pending RPC task.
#[derive(Clone, Serialize)]
pub enum HwRpcTaskAwaitingStatus {
    WaitForTrezorPin,
}

/// When it comes to interacting with a HW device,
/// this is a common user action in answer to awaiting RPC task status.
#[derive(Deserialize, Serialize)]
#[serde(tag = "action_type")]
pub enum HwRpcTaskUserAction {
    TrezorPin(TrezorPinMatrix3x3Response),
}

impl TryFrom<HwRpcTaskUserAction> for TrezorPinMatrix3x3Response {
    type Error = RpcTaskError;

    fn try_from(value: HwRpcTaskUserAction) -> Result<Self, Self::Error> {
        match value {
            HwRpcTaskUserAction::TrezorPin(pin) => Ok(pin),
        }
    }
}

#[derive(Clone)]
pub struct HwConnectStatuses<InProgressStatus, AwaitingStatus> {
    pub on_connect: InProgressStatus,
    pub on_connected: InProgressStatus,
    pub on_connection_failed: InProgressStatus,
    pub on_button_request: InProgressStatus,
    pub on_pin_request: AwaitingStatus,
    pub on_ready: InProgressStatus,
}

impl<InProgressStatus, AwaitingStatus> HwConnectStatuses<InProgressStatus, AwaitingStatus>
where
    InProgressStatus: Clone,
    AwaitingStatus: Clone,
{
    pub fn to_trezor_request_statuses(&self) -> TrezorRequestStatuses<InProgressStatus, AwaitingStatus> {
        TrezorRequestStatuses {
            on_button_request: self.on_button_request.clone(),
            on_pin_request: self.on_pin_request.clone(),
            on_ready: self.on_ready.clone(),
        }
    }
}

pub struct TrezorRpcTaskConnectProcessor<'a, Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    request_processor: TrezorRpcTaskProcessor<'a, Item, Error, InProgressStatus, AwaitingStatus, UserAction>,
    on_connect: InProgressStatus,
    on_connected: InProgressStatus,
    on_connection_failed: InProgressStatus,
    connect_timeout: Duration,
}

#[async_trait]
impl<'a, Item, Error, InProgressStatus, AwaitingStatus, UserAction> TrezorRequestProcessor
    for TrezorRpcTaskConnectProcessor<'a, Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize + Send,
    Error: SerMmErrorType + Send,
    InProgressStatus: Clone + Send + Sync,
    AwaitingStatus: Clone + Send + Sync,
    UserAction: TryInto<TrezorPinMatrix3x3Response, Error = RpcTaskError> + Send,
{
    type Error = RpcTaskError;

    async fn on_button_request(&self) -> MmResult<(), TrezorProcessingError<Self::Error>> {
        self.request_processor.on_button_request().await
    }

    async fn on_pin_request(&self) -> MmResult<TrezorPinMatrix3x3Response, TrezorProcessingError<Self::Error>> {
        self.request_processor.on_pin_request().await
    }

    async fn on_ready(&self) -> MmResult<(), TrezorProcessingError<Self::Error>> {
        self.request_processor.on_ready().await
    }
}

#[async_trait]
impl<'a, Item, Error, InProgressStatus, AwaitingStatus, UserAction> TrezorConnectProcessor
    for TrezorRpcTaskConnectProcessor<'a, Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize + Send,
    Error: SerMmErrorType + Send,
    InProgressStatus: Clone + Send + Sync,
    AwaitingStatus: Clone + Send + Sync,
    UserAction: TryInto<TrezorPinMatrix3x3Response, Error = RpcTaskError> + Send,
{
    async fn on_connect(&self) -> MmResult<Duration, HwProcessingError<RpcTaskError>> {
        self.request_processor
            .update_in_progress_status(self.on_connect.clone())?;
        Ok(self.connect_timeout)
    }

    async fn on_connected(&self) -> MmResult<(), HwProcessingError<RpcTaskError>> {
        Ok(self
            .request_processor
            .update_in_progress_status(self.on_connected.clone())?)
    }

    async fn on_connection_failed(&self) -> MmResult<(), HwProcessingError<RpcTaskError>> {
        Ok(self
            .request_processor
            .update_in_progress_status(self.on_connection_failed.clone())?)
    }
}

impl<'a, Item, Error, InProgressStatus, AwaitingStatus, UserAction>
    TrezorRpcTaskConnectProcessor<'a, Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    pub fn new(
        task_handle: &'a RpcTaskHandle<Item, Error, InProgressStatus, AwaitingStatus, UserAction>,
        statuses: HwConnectStatuses<InProgressStatus, AwaitingStatus>,
    ) -> Self {
        let request_statuses = TrezorRequestStatuses {
            on_button_request: statuses.on_button_request,
            on_pin_request: statuses.on_pin_request,
            on_ready: statuses.on_ready,
        };
        let request_processor = TrezorRpcTaskProcessor::new(task_handle, request_statuses);
        TrezorRpcTaskConnectProcessor {
            request_processor,
            on_connect: statuses.on_connect,
            on_connected: statuses.on_connected,
            on_connection_failed: statuses.on_connection_failed,
            connect_timeout: CONNECT_DEFAULT_TIMEOUT,
        }
    }

    pub fn with_pin_timeout(mut self, pin_timeout: Duration) -> Self {
        self.request_processor = self.request_processor.with_pin_timeout(pin_timeout);
        self
    }

    pub fn with_connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }
}

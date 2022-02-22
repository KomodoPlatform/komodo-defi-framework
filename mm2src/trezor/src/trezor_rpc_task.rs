use crate::response_processor::{TrezorProcessingError, TrezorRequestProcessor};
use crate::TrezorPinMatrix3x3Response;
use async_trait::async_trait;
use common::mm_error::prelude::*;
use std::convert::TryInto;
use std::time::Duration;

pub use rpc_task::{RpcTask, RpcTaskError, RpcTaskHandle};

const DEFAULT_PIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

pub struct TrezorRequestStatuses<InProgressStatus, AwaitingStatus> {
    pub on_button_request: InProgressStatus,
    pub on_pin_request: AwaitingStatus,
    pub on_ready: InProgressStatus,
}

pub struct TrezorRpcTaskProcessor<'a, Task: RpcTask> {
    task_handle: &'a RpcTaskHandle<Task>,
    statuses: TrezorRequestStatuses<Task::InProgressStatus, Task::AwaitingStatus>,
    pin_timeout: Duration,
}

#[async_trait]
impl<'a, Task> TrezorRequestProcessor for TrezorRpcTaskProcessor<'a, Task>
where
    Task: RpcTask,
    Task::UserAction: TryInto<TrezorPinMatrix3x3Response, Error = RpcTaskError> + Send,
{
    type Error = RpcTaskError;

    async fn on_button_request(&self) -> MmResult<(), TrezorProcessingError<RpcTaskError>> {
        self.update_in_progress_status(self.statuses.on_button_request.clone())
    }

    async fn on_pin_request(&self) -> MmResult<TrezorPinMatrix3x3Response, TrezorProcessingError<RpcTaskError>> {
        let user_action = self
            .task_handle
            .wait_for_user_action(self.pin_timeout, self.statuses.on_pin_request.clone())
            .await
            .mm_err(TrezorProcessingError::ProcessorError)?;
        let pin_response: TrezorPinMatrix3x3Response = user_action
            .try_into()
            .map_to_mm(TrezorProcessingError::ProcessorError)?;
        Ok(pin_response)
    }

    async fn on_ready(&self) -> MmResult<(), TrezorProcessingError<RpcTaskError>> {
        self.update_in_progress_status(self.statuses.on_ready.clone())
    }
}

impl<'a, Task: RpcTask> TrezorRpcTaskProcessor<'a, Task> {
    pub fn new(
        task_handle: &'a RpcTaskHandle<Task>,
        statuses: TrezorRequestStatuses<Task::InProgressStatus, Task::AwaitingStatus>,
    ) -> TrezorRpcTaskProcessor<'a, Task> {
        TrezorRpcTaskProcessor {
            task_handle,
            statuses,
            pin_timeout: DEFAULT_PIN_REQUEST_TIMEOUT,
        }
    }

    pub fn with_pin_timeout(mut self, pin_timeout: Duration) -> Self {
        self.pin_timeout = pin_timeout;
        self
    }

    pub fn update_in_progress_status(
        &self,
        in_progress: Task::InProgressStatus,
    ) -> MmResult<(), TrezorProcessingError<RpcTaskError>> {
        self.task_handle
            .update_in_progress_status(in_progress)
            .mm_err(TrezorProcessingError::ProcessorError)
    }
}

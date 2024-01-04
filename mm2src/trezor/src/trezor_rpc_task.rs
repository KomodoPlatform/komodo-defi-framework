use crate::response_processor::{TrezorProcessingError, TrezorRequestProcessor};
use crate::user_interaction::TrezorPassphraseResponse;
use crate::TrezorPinMatrix3x3Response;
use async_trait::async_trait;
use mm2_err_handle::prelude::*;
use std::convert::TryInto;
use std::time::Duration;

pub use rpc_task::{RpcTask, RpcTaskError, RpcTaskHandleShared};

const DEFAULT_USER_ACTION_TIMEOUT: Duration = Duration::from_secs(300);

pub trait TryIntoUserAction:
    TryInto<TrezorPinMatrix3x3Response, Error = RpcTaskError>
    + TryInto<TrezorPassphraseResponse, Error = RpcTaskError>
    + Send
{
}

impl<T> TryIntoUserAction for T where
    T: TryInto<TrezorPinMatrix3x3Response, Error = RpcTaskError>
        + TryInto<TrezorPassphraseResponse, Error = RpcTaskError>
        + Send
{
}

#[derive(Clone)]
pub struct TrezorRequestStatuses<InProgressStatus, AwaitingStatus> {
    pub on_button_request: InProgressStatus,
    pub on_pin_request: AwaitingStatus,
    pub on_passphrase_request: AwaitingStatus,
    pub on_ready: InProgressStatus,
}

pub struct TrezorRpcTaskProcessor<Task: RpcTask> {
    task_handle: RpcTaskHandleShared<Task>,
    statuses: TrezorRequestStatuses<Task::InProgressStatus, Task::AwaitingStatus>,
    user_action_timeout: Duration,
}

/// Custom Clone to avoid clone derivations for structs implementing RpcTask
impl<Task: RpcTask> Clone for TrezorRpcTaskProcessor<Task> {
    fn clone(&self) -> Self {
        Self {
            task_handle: self.task_handle.clone(),
            statuses: self.statuses.clone(),
            user_action_timeout: self.user_action_timeout,
        }
    }
}

#[async_trait]
impl<Task> TrezorRequestProcessor for TrezorRpcTaskProcessor<Task>
where
    Task: RpcTask,
    Task::UserAction: TryIntoUserAction + Send,
{
    type Error = RpcTaskError;

    async fn on_button_request(&self) -> MmResult<(), TrezorProcessingError<RpcTaskError>> {
        self.update_in_progress_status(self.statuses.on_button_request.clone())
    }

    async fn on_pin_request(&self) -> MmResult<TrezorPinMatrix3x3Response, TrezorProcessingError<RpcTaskError>> {
        let user_action = self
            .task_handle
            .wait_for_user_action(self.user_action_timeout, self.statuses.on_pin_request.clone())
            .await
            .mm_err(TrezorProcessingError::ProcessorError)?;
        let pin_response: TrezorPinMatrix3x3Response = user_action
            .try_into()
            .map_to_mm(TrezorProcessingError::ProcessorError)?;
        Ok(pin_response)
    }

    async fn on_passphrase_request(&self) -> MmResult<TrezorPassphraseResponse, TrezorProcessingError<Self::Error>> {
        let user_action = self
            .task_handle
            .wait_for_user_action(self.user_action_timeout, self.statuses.on_passphrase_request.clone())
            .await
            .mm_err(TrezorProcessingError::ProcessorError)?;
        let passphrase_response: TrezorPassphraseResponse = user_action
            .try_into()
            .map_to_mm(TrezorProcessingError::ProcessorError)?;
        Ok(passphrase_response)
    }

    async fn on_ready(&self) -> MmResult<(), TrezorProcessingError<RpcTaskError>> {
        self.update_in_progress_status(self.statuses.on_ready.clone())
    }
}

impl<Task: RpcTask> TrezorRpcTaskProcessor<Task> {
    pub fn new(
        task_handle: RpcTaskHandleShared<Task>,
        statuses: TrezorRequestStatuses<Task::InProgressStatus, Task::AwaitingStatus>,
    ) -> TrezorRpcTaskProcessor<Task> {
        TrezorRpcTaskProcessor {
            task_handle,
            statuses,
            user_action_timeout: DEFAULT_USER_ACTION_TIMEOUT,
        }
    }

    pub fn with_user_action_timeout(mut self, pin_timeout: Duration) -> Self {
        self.user_action_timeout = pin_timeout;
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

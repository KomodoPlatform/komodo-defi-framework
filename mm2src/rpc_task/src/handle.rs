use crate::manager::{RpcTaskManager, RpcTaskManagerWeak};
use crate::{FinishedTaskResult, RpcTaskError, RpcTaskResult, TaskId, TaskStatus};
use common::custom_futures::FutureTimerExt;
use common::log::LogOnError;
use common::mm_error::prelude::*;
use futures::channel::oneshot;
use serde::Serialize;
use std::sync::MutexGuard;
use std::time::Duration;

type TaskManagerLock<'a, Item, Error, InProgressStatus, AwaitingStatus, UserAction> =
    MutexGuard<'a, RpcTaskManager<Item, Error, InProgressStatus, AwaitingStatus, UserAction>>;

pub struct RpcTaskHandle<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    pub(crate) task_manager: RpcTaskManagerWeak<Item, Error, InProgressStatus, AwaitingStatus, UserAction>,
    pub(crate) task_id: TaskId,
}

impl<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
    RpcTaskHandle<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    pub(crate) fn abort(self) {
        self.lock_and_then(|mut task_manager| task_manager.cancel_task(self.task_id))
            .ok();
    }

    fn lock_and_then<F, T>(&self, f: F) -> RpcTaskResult<T>
    where
        F: FnOnce(TaskManagerLock<Item, Error, InProgressStatus, AwaitingStatus, UserAction>) -> RpcTaskResult<T>,
    {
        let arc = self
            .task_manager
            .upgrade()
            .or_mm_err(|| RpcTaskError::Internal("RpcTaskManager is not available".to_owned()))?;
        let lock = arc
            .lock()
            .map_to_mm(|e| RpcTaskError::Internal(format!("RpcTaskManager is not available: {}", e)))?;
        f(lock)
    }

    fn update_task_status(
        &self,
        status: TaskStatus<Item, Error, InProgressStatus, AwaitingStatus, UserAction>,
    ) -> RpcTaskResult<()> {
        self.lock_and_then(|mut task_manager| task_manager.update_task_status(self.task_id, status))
    }
}

impl<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
    RpcTaskHandle<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    pub fn update_in_progress_status(&self, in_progress: InProgressStatus) -> RpcTaskResult<()> {
        self.update_task_status(TaskStatus::InProgress(in_progress))
    }
}

impl<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
    RpcTaskHandle<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    pub async fn wait_for_user_action(
        &self,
        timeout: Duration,
        awaiting_status: AwaitingStatus,
    ) -> RpcTaskResult<UserAction> {
        let (user_action_tx, user_action_rx) = oneshot::channel();
        // Set the status to 'UserActionRequired' to let the user know that we are waiting for an action.
        self.update_task_status(TaskStatus::UserActionRequired {
            awaiting_status,
            user_action_tx,
        })?;

        // Wait for the user action.
        user_action_rx
            .timeout(timeout)
            .await?
            .map_to_mm(|_canceled| RpcTaskError::Canceled)
    }
}

impl<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
    RpcTaskHandle<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    pub(crate) fn finish(self, result: Result<Item, MmError<Error>>) {
        let task_result = Self::prepare_task_result(result);
        self.lock_and_then(|mut task_manager| {
            task_manager.update_task_status(self.task_id, TaskStatus::Ready(task_result))
        })
        .warn_log();
    }

    fn prepare_task_result(result: Result<Item, MmError<Error>>) -> FinishedTaskResult<Item, Error> {
        match result {
            Ok(task_item) => FinishedTaskResult::ok(task_item),
            Err(task_error) => FinishedTaskResult::Err(task_error),
        }
    }
}

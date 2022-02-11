use common::custom_futures::TimeoutError;
use common::mm_error::prelude::*;
use common::mm_rpc_protocol::MmRpcResult;
use derive_more::Display;
use futures::channel::oneshot;
use serde::Serialize;
use std::time::Duration;

#[macro_use] extern crate ser_error_derive;
#[macro_use] extern crate serde_derive;

mod handle;
mod manager;
pub mod rpc_common;
mod task;

pub use handle::RpcTaskHandle;
pub use manager::{RpcTaskManager, RpcTaskManagerShared};
pub use task::RpcTask;

pub type FinishedTaskResult<Item, Error> = MmRpcResult<Item, Error>;
pub type RpcTaskResult<T> = Result<T, MmError<RpcTaskError>>;
pub type TaskId = u64;

type UserActionSender<UserAction> = oneshot::Sender<UserAction>;
type TaskAbortHandle = oneshot::Sender<()>;
type TaskAbortHandler = oneshot::Receiver<()>;

#[derive(Clone, Display)]
pub enum RpcTaskError {
    #[display(fmt = "RPC task timeout '{:?}'", _0)]
    Timeout(Duration),
    NoSuchTask(TaskId),
    #[display(
        fmt = "RPC '{}' task is in unexpected status. Actual: '{}', expected: '{}'",
        task_id,
        actual,
        expected
    )]
    UnexpectedTaskStatus {
        task_id: TaskId,
        actual: TaskStatusError,
        expected: TaskStatusError,
    },
    Canceled,
    Internal(String),
}

#[derive(Clone, Display)]
pub enum TaskStatusError {
    Idle,
    InProgress,
    AwaitingUserAction,
    Finished,
}

impl From<TimeoutError> for RpcTaskError {
    fn from(e: TimeoutError) -> Self { RpcTaskError::Timeout(e.duration) }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", content = "details")]
pub enum RpcTaskStatus<Item, Error, InProgressStatus, AwaitingStatus>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    Ready(FinishedTaskResult<Item, Error>),
    InProgress(InProgressStatus),
    UserActionRequired(AwaitingStatus),
}

impl<Item, Error, InProgressStatus, AwaitingStatus> RpcTaskStatus<Item, Error, InProgressStatus, AwaitingStatus>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    pub fn map_err<NewError, F>(self, f: F) -> RpcTaskStatus<Item, NewError, InProgressStatus, AwaitingStatus>
    where
        F: FnOnce(Error) -> NewError,
        NewError: SerMmErrorType,
    {
        match self {
            RpcTaskStatus::Ready(result) => RpcTaskStatus::Ready(result.map_err(f)),
            RpcTaskStatus::InProgress(in_progress) => RpcTaskStatus::InProgress(in_progress),
            RpcTaskStatus::UserActionRequired(awaiting) => RpcTaskStatus::UserActionRequired(awaiting),
        }
    }
}

enum TaskStatus<Item, Error, InProgressStatus, AwaitingStatus, UserAction>
where
    Item: Serialize,
    Error: SerMmErrorType,
{
    Ready(FinishedTaskResult<Item, Error>),
    InProgress(InProgressStatus),
    UserActionRequired {
        awaiting_status: AwaitingStatus,
        user_action_tx: UserActionSender<UserAction>,
    },
}

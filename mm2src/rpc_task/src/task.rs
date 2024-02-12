use crate::handle::RpcTaskHandleShared;
use async_trait::async_trait;
use mm2_err_handle::prelude::*;
use serde::Serialize;

pub trait RpcTaskTypes {
    type Item: Serialize + Clone + Send + Sync + 'static;
    type Error: SerMmErrorType + Clone + Send + Sync + 'static;
    type InProgressStatus: Clone + Send + Sync + 'static;
    type AwaitingStatus: Clone + Send + Sync + 'static;
    type UserAction: NotMmError + Send + Sync + 'static;
}

#[async_trait]
pub trait RpcTask: RpcTaskTypes + Sized + Send + 'static {
    fn initial_status(&self) -> Self::InProgressStatus;

    /// The method is invoked when the task has been cancelled.
    async fn cancel(self);

    async fn run(&mut self, task_handle: RpcTaskHandleShared<Self>) -> Result<Self::Item, MmError<Self::Error>>;
}

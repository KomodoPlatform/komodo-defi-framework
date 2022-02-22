use crate::handle::RpcTaskHandle;
use async_trait::async_trait;
use common::mm_error::prelude::*;
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

    async fn run(self, task_handle: &RpcTaskHandle<Self>) -> Result<Self::Item, MmError<Self::Error>>;
}

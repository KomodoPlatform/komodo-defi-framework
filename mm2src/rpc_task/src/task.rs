use crate::handle::RpcTaskHandle;
use async_trait::async_trait;
use common::mm_error::prelude::*;
use serde::de::DeserializeOwned;
use serde::Serialize;

#[async_trait]
pub trait RpcTask: Sized {
    type Item: Serialize;
    type Error: SerMmErrorType;
    type InProgressStatus: Serialize;
    type AwaitingStatus: Serialize;
    type UserAction: DeserializeOwned;

    fn initial_status(&self) -> Self::InProgressStatus;

    /// # Clippy
    ///
    /// Currently, there is no way to simplify the task handle type:
    /// https://github.com/rust-lang/rust-clippy/issues/1013#issuecomment-587054810
    #[allow(clippy::type_complexity)]
    async fn run(
        self,
        task_handle: &RpcTaskHandle<
            Self::Item,
            Self::Error,
            Self::InProgressStatus,
            Self::AwaitingStatus,
            Self::UserAction,
        >,
    ) -> Result<Self::Item, MmError<Self::Error>>;
}

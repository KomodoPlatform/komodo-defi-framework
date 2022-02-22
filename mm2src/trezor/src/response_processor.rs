use crate::{TrezorError, TrezorPinMatrix3x3Response};
use async_trait::async_trait;
use common::mm_error::prelude::*;
use common::NotSame;
use derive_more::Display;

#[derive(Display)]
pub enum TrezorProcessingError<E> {
    TrezorError(TrezorError),
    ProcessorError(E),
}

impl<E> From<TrezorError> for TrezorProcessingError<E> {
    fn from(e: TrezorError) -> Self { TrezorProcessingError::TrezorError(e) }
}

/// This is required for implementing `MmError<TrezorProcessingError<E>>: From<MmError<TrezorError>>`.
impl<E> NotSame for TrezorProcessingError<E> {}

#[async_trait]
pub trait TrezorRequestProcessor {
    type Error: NotMmError + Send;

    async fn on_button_request(&self) -> MmResult<(), TrezorProcessingError<Self::Error>>;

    async fn on_pin_request(&self) -> MmResult<TrezorPinMatrix3x3Response, TrezorProcessingError<Self::Error>>;

    async fn on_ready(&self) -> MmResult<(), TrezorProcessingError<Self::Error>>;
}

#[async_trait]
pub trait ProcessTrezorResponse<T>
where
    T: Send + Sync + 'static,
{
    async fn process<Processor>(self, processor: &Processor) -> MmResult<T, TrezorProcessingError<Processor::Error>>
    where
        Self: Sized,
        Processor: TrezorRequestProcessor + Sync;
}

use async_trait::async_trait;
use derive_more::Display;
use mm2_err_handle::prelude::*;

use crate::user_interaction::TrezorPassphraseResponse;
use crate::{TrezorError, TrezorPinMatrix3x3Response};

#[derive(Display)]
pub enum TrezorProcessingError<E> {
    TrezorError(TrezorError),
    ProcessorError(E),
}

impl<E> From<TrezorError> for TrezorProcessingError<E> {
    fn from(e: TrezorError) -> Self { TrezorProcessingError::TrezorError(e) }
}

/// This is required for implementing `MmError<TrezorProcessingError<E>>: From<MmError<TrezorError>>`.
impl<E> NotEqual for TrezorProcessingError<E> {}

#[async_trait]
pub trait TrezorRequestProcessor {
    type Error: NotMmError + Send;

    async fn on_button_request(&self) -> MmResult<(), TrezorProcessingError<Self::Error>>;

    async fn on_pin_request(&self) -> MmResult<TrezorPinMatrix3x3Response, TrezorProcessingError<Self::Error>>;

    async fn on_passphrase_request(&self) -> MmResult<TrezorPassphraseResponse, TrezorProcessingError<Self::Error>>;

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

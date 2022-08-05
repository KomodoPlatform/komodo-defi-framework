//! This file is inspired by https://github.com/tezedge/tezedge-client/blob/master/trezor_api/src/client.rs

use crate::error::OperationFailure;
use crate::proto::messages::MessageType;
use crate::proto::messages_common as proto_common;
use crate::proto::messages_management as proto_management;
use crate::proto::{ProtoMessage, TrezorMessage};
use crate::response::TrezorResponse;
use crate::result_handler::ResultHandler;
use crate::transport::Transport;
use crate::{TrezorError, TrezorResult};
use futures::lock::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use mm2_err_handle::prelude::*;
use std::sync::Arc;

#[derive(Clone)]
pub struct TrezorClient {
    inner: Arc<AsyncMutex<TrezorClientImpl>>,
}

impl TrezorClient {
    pub fn from_transport<T>(transport: T) -> TrezorClient
    where
        T: Transport + Send + Sync + 'static,
    {
        let transport = Box::new(transport);
        let inner = Arc::new(AsyncMutex::new(TrezorClientImpl { transport }));
        TrezorClient { inner }
    }

    /// Initialize a Trezor session by sending
    /// [Initialize](https://docs.trezor.io/trezor-firmware/common/communication/sessions.html#examples).
    pub async fn session(&self) -> TrezorResult<TrezorSession<'_>> {
        let mut session = TrezorSession {
            inner: self.inner.lock().await,
        };
        session.initialize_device().await?;
        Ok(session)
    }
}

pub struct TrezorClientImpl {
    transport: Box<dyn Transport + Send + Sync + 'static>,
}

pub struct TrezorSession<'a> {
    inner: AsyncMutexGuard<'a, TrezorClientImpl>,
}

impl<'a> TrezorSession<'a> {
    /// Sends a message and returns a TrezorResponse with either the
    /// expected response message, a failure or an interaction request.
    pub async fn call<'b, T: 'static, S: TrezorMessage>(
        &'b mut self,
        message: S,
        result_handler: ResultHandler<T>,
    ) -> TrezorResult<TrezorResponse<'a, 'b, T>> {
        let resp = self.call_raw(message).await?;
        match resp.message_type() {
            mt if mt == result_handler.message_type() => Ok(TrezorResponse::Ready(result_handler.handle_raw(resp)?)),
            MessageType::Failure => {
                let fail_msg: proto_common::Failure = resp.into_message()?;
                MmError::err(TrezorError::Failure(OperationFailure::from(fail_msg)))
            },
            MessageType::ButtonRequest => {
                let req_msg = resp.into_message()?;
                Ok(TrezorResponse::new_button_request(self, req_msg, result_handler))
            },
            MessageType::PinMatrixRequest => {
                let req_msg = resp.into_message()?;
                Ok(TrezorResponse::new_pin_matrix_request(self, req_msg, result_handler))
            },
            MessageType::PassphraseRequest => {
                let req_msg = resp.into_message()?;
                Ok(TrezorResponse::new_passphrase_request(self, req_msg, result_handler))
            },
            mtype => MmError::err(TrezorError::UnexpectedMessageType(mtype)),
        }
    }

    /// Sends a message and returns the raw ProtoMessage struct that was
    /// responded by the device.
    async fn call_raw<'b, S: TrezorMessage>(&'b mut self, message: S) -> TrezorResult<ProtoMessage> {
        let mut buf = Vec::with_capacity(message.encoded_len());
        message.encode(&mut buf)?;

        let proto_msg = ProtoMessage::new(S::message_type(), buf);
        self.inner.transport.write_message(proto_msg).await?;
        self.inner.transport.read_message().await
    }

    /// Initialize the device.
    ///
    /// The Initialize packet will cause the device to stop what it is currently doing
    /// and should work at any time.
    /// Thus, it can also be used to recover from previous errors.
    ///
    /// # Usage
    ///
    /// Must be called before sending requests to Trezor.
    async fn initialize_device<'b>(&'b mut self) -> TrezorResult<proto_management::Features> {
        // Don't set the session_id since currently there is no need to restore the previous session.
        // https://docs.trezor.io/trezor-firmware/common/communication/sessions.html#session-lifecycle
        let req = proto_management::Initialize { session_id: None };

        let result_handler = ResultHandler::<proto_management::Features>::new(Ok);
        self.call(req, result_handler).await?.ok()
    }

    pub(crate) async fn cancel_last_op<'b>(&'b mut self) {
        let req = proto_management::Cancel {};
        let result_handler = ResultHandler::new(|_m: proto_common::Failure| Ok(()));
        // Ignore result.
        self.call(req, result_handler).await.ok();
    }
}

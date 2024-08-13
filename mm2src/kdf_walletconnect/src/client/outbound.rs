use crate::error::{ClientError, ServiceErrorExt};

use pin_project::pin_project;
use relay_rpc::rpc::{Params, ServiceRequest};
use std::{future::Future,
          marker::PhantomData,
          pin::Pin,
          task::{ready, Context, Poll}};
use tokio::sync::oneshot;

type TxSender = oneshot::Sender<Result<serde_json::Value, ClientError>>;
type TxHandler = oneshot::Receiver<Result<serde_json::Value, ClientError>>;

// An outbound request wrapper created by [`create_request()`]. Intended be
/// used with [`ClientStream`][crate::client::ClientStream].
#[derive(Debug)]
pub struct OutboundRequest {
    pub(crate) params: Params,
    pub(crate) tx: TxSender,
}

impl OutboundRequest {
    pub(crate) fn new(params: Params, tx: TxSender) -> Self { Self { params, tx } }
}

/// Future that resolves with the RPC response for the specified request.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project]
pub struct ResponseFuture<T> {
    #[pin]
    rx: TxHandler,
    _marker: PhantomData<T>,
}

impl<T> ResponseFuture<T> {
    pub(crate) fn new(rx: TxHandler) -> Self {
        Self {
            rx,
            _marker: PhantomData,
        }
    }
}

impl<T> Future for ResponseFuture<T>
where
    T: ServiceRequest,
{
    type Output = Result<T::Response, ServiceErrorExt<T::Error>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let result = ready!(this.rx.poll(cx)).map_err(|_| ClientError::ChannelClosed)?;
        let result = match result {
            Ok(value) => {
                serde_json::from_value(value).map_err(|err| ClientError::DeserializationError(err.to_string()))
            },
            Err(err) => Err(err),
        };

        Poll::Ready(result.map_err(Into::into))
    }
}

/// Future that resolves with the RPC response, consuming it and returning
/// `Result<(), Error>`.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project]
pub struct EmptyResponseFuture<T> {
    #[pin]
    rx: ResponseFuture<T>,
}

impl<T> EmptyResponseFuture<T> {
    pub(crate) fn new(rx: ResponseFuture<T>) -> Self { Self { rx } }
}

impl<T> Future for EmptyResponseFuture<T>
where
    T: ServiceRequest,
{
    type Output = Result<(), ServiceErrorExt<T::Error>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(ready!(self.project().rx.poll(cx)).map(|_| ()))
    }
}

/// Creates an RPC request and returns a tuple of the request and a response
/// future. The request is intended to be used with
/// [`ClientStream`][crate::client::ClientStream].
pub fn create_request<T>(data: T) -> (OutboundRequest, ResponseFuture<T>)
where
    T: ServiceRequest,
{
    let (tx, rx) = oneshot::channel();

    (OutboundRequest::new(data.into_params(), tx), ResponseFuture::new(rx))
}

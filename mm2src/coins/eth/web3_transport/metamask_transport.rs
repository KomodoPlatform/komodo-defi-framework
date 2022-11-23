use crate::eth::web3_transport::Web3SendOut;
use crate::RpcTransportEventHandlerShared;
use crypto::MetamaskWeak;
use futures::{FutureExt, TryFutureExt};
use jsonrpc_core::{Call, Params};
use serde_json::Value as Json;
use std::fmt;
use web3::error::{Error, ErrorKind};
use web3::helpers::build_request;
use web3::{RequestId, Transport};

#[derive(Clone)]
pub struct MetamaskTransport {
    metamask_ctx: MetamaskWeak,
    // TODO use `even_handlers` properly.
    _event_handlers: Vec<RpcTransportEventHandlerShared>,
}

impl MetamaskTransport {
    pub fn new(metamask_ctx: MetamaskWeak, event_handlers: Vec<RpcTransportEventHandlerShared>) -> MetamaskTransport {
        MetamaskTransport {
            metamask_ctx,
            _event_handlers: event_handlers,
        }
    }

    async fn send_request(&self, request: Call) -> Result<Json, Error> {
        let metamask_ctx = self.metamask_ctx.upgrade().ok_or_else(|| {
            Error::from(ErrorKind::Transport(
                "MetaMask context doesn't exist already".to_string(),
            ))
        })?;
        let provider = metamask_ctx.metamask_provider();
        let mut session = provider.session().await;

        let (method, params) = match request {
            Call::MethodCall(method_call) => (method_call.method, method_call.params),
            Call::Notification(notification) => (notification.method, notification.params),
            Call::Invalid(_) => return Err(Error::from(ErrorKind::Internal)),
        };

        let params = match params {
            // EthProvider doesn't allow to pass an object as the params,
            // but we still can try to pass the object as a single array item.
            Some(Params::Map(object)) => vec![Json::Object(object)],
            Some(Params::Array(array)) => array,
            Some(Params::None) | None => Vec::new(),
        };
        session
            .eth_request(method, params)
            .await
            // TODO consider matching this error.
            .map_err(|e| Error::from(ErrorKind::Transport(e.to_string())))
    }
}

impl Transport for MetamaskTransport {
    type Out = Web3SendOut;

    fn prepare(&self, method: &str, params: Vec<Json>) -> (RequestId, Call) {
        // RequestId doesn't make sense for `MetamaskProvider`.
        const REQUEST_ID: RequestId = 0;

        let request = build_request(REQUEST_ID, method, params);
        (REQUEST_ID, request)
    }

    fn send(&self, _id: RequestId, request: Call) -> Self::Out {
        let transport = self.clone();
        let fut = async move { transport.send_request(request).await };
        Box::new(fut.boxed().compat())
    }
}

impl fmt::Debug for MetamaskTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "MetamaskTransport") }
}

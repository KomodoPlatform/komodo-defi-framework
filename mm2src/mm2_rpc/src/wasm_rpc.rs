use std::pin::Pin;
use std::task::{Context, Poll};

use futures::channel::{mpsc, oneshot};
use futures::lock::Mutex as AsyncMutex;
use futures::Stream;
use gstuff::{ERR, ERRL};
use serde_json::Value as Json;

const CHANNEL_BUF_SIZE: usize = 1024;

pub type WasmRpcResponse = Result<Json, String>;
pub type WasmRpcRequest = (Json, oneshot::Sender<WasmRpcResponse>);

pub fn channel() -> (WasmRpcSender, WasmRpcReceiver) {
    let (tx, rx) = mpsc::channel(CHANNEL_BUF_SIZE);
    let tx = AsyncMutex::new(tx);
    (WasmRpcSender { tx }, WasmRpcReceiver { rx })
}

pub struct WasmRpcSender {
    tx: AsyncMutex<mpsc::Sender<WasmRpcRequest>>,
}

impl WasmRpcSender {
    pub async fn request(&self, request_json: Json) -> WasmRpcResponse {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .lock()
            .await
            .try_send((request_json, response_tx))
            .map_err(|e| ERRL!("Couldn't send RPC request: {}", e))?;

        match response_rx.await {
            Ok(res) => res,
            Err(e) => ERR!("{}", e),
        }
    }
}

pub struct WasmRpcReceiver {
    rx: mpsc::Receiver<WasmRpcRequest>,
}

impl Stream for WasmRpcReceiver {
    type Item = WasmRpcRequest;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().rx).poll_next(cx)
    }
}

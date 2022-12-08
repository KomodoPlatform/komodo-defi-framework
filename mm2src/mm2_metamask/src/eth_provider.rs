//! This module is inspired by https://github.com/tomusdrw/rust-web3/blob/master/src/transports/eip_1193.rs

use common::executor::{spawn_local_abortable, AbortOnDropHandle};
use common::{deserialize_from_js, serialize_to_js, stringify_js_error};
use futures::channel::{mpsc, oneshot};
use futures::StreamExt;
use mm2_err_handle::prelude::*;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;
use wasm_bindgen::prelude::*;

pub type EthProviderResult<T> = MmResult<T, EthProviderError>;

type EthCommandSender = mpsc::Sender<EthCommand>;
type EthCommandReceiver = mpsc::Receiver<EthCommand>;
type EthCommandResultSender<T> = oneshot::Sender<EthProviderResult<T>>;

pub enum EthProviderError {
    ErrorSerializingArguments(String),
    ErrorDeserializingMethodResult(String),
    ErrorInvokingMethod { method: String, error: String },
    Internal(String),
}

pub struct EthProvider {
    command_tx: EthCommandSender,
    /// This abort handle is needed to drop the spawned at [`WebUsbWrapper::new`] future immediately.
    _abort_handle: AbortOnDropHandle,
}

impl EthProvider {
    pub fn detect_ethereum_provider(command_channel_capacity: usize) -> Option<EthProvider> {
        let raw_provider = get_window_ethereum()?;

        let (command_tx, command_rx) = mpsc::channel(command_channel_capacity);
        let abort_handle = spawn_local_abortable(Self::command_loop(raw_provider, command_rx));

        Some(EthProvider {
            command_tx,
            _abort_handle: abort_handle,
        })
    }

    pub async fn invoke_method<T>(&mut self, method: String, params: Vec<Json>) -> EthProviderResult<T>
    where
        T: DeserializeOwned,
    {
        let (result_tx, result_rx) = oneshot::channel();
        let command = EthCommand::InvokeMethod {
            method,
            params,
            result_tx,
        };
        let result = send_command_recv_response(&mut self.command_tx, command, result_rx).await?;
        serde_json::from_value(result).map_to_mm(|e| EthProviderError::ErrorDeserializingMethodResult(e.to_string()))
    }

    async fn command_loop(raw_provider: RawEthProvider, mut command_rx: EthCommandReceiver) {
        while let Some(command) = command_rx.next().await {
            match command {
                EthCommand::InvokeMethod {
                    method,
                    params,
                    result_tx,
                } => {
                    let res = Self::on_invoke_method(&raw_provider, method, params).await;
                    result_tx.send(res).ok();
                },
            }
        }
    }

    async fn on_invoke_method(
        raw_provider: &RawEthProvider,
        method: String,
        params: Vec<Json>,
    ) -> EthProviderResult<Json> {
        let js_params = js_sys::Array::new();

        for param in params {
            let arg_js_value =
                serialize_to_js(&param).map_to_mm(|e| EthProviderError::ErrorSerializingArguments(e.to_string()))?;

            js_params.push(&arg_js_value);
        }

        let args = RawRequestArguments {
            method: method.clone(),
            params: js_params,
        };

        // TODO consider specifying a timeout.
        let js_result =
            raw_provider
                .request(args)
                .await
                .map_to_mm(|js_error| EthProviderError::ErrorInvokingMethod {
                    method,
                    error: stringify_js_error(&js_error),
                })?;

        deserialize_from_js(js_result).map_to_mm(|e| EthProviderError::ErrorDeserializingMethodResult(e.to_string()))
    }
}

#[wasm_bindgen]
extern "C" {
    /// An EIP-1193 provider object. Available by convention at `window.ethereum`.
    type RawEthProvider;

    #[wasm_bindgen(catch, method)]
    async fn request(_: &RawEthProvider, args: RawRequestArguments) -> Result<JsValue, JsValue>;

    // #[wasm_bindgen(method)]
    // fn on(_: &RawEthProvider, event_name: &str, listener: &Closure<dyn FnMut(JsValue)>);
    //
    // #[wasm_bindgen(method, js_name = "removeListener")]
    // fn removeListener(_: &RawEthProvider, event_name: &str, listener: &Closure<dyn FnMut(JsValue)>);
}

#[wasm_bindgen(inline_js = "export function get_window_ethereum() { return window.ethereum; }")]
extern "C" {
    fn get_window_ethereum() -> Option<RawEthProvider>;
}

#[wasm_bindgen]
struct RawRequestArguments {
    method: String,
    params: js_sys::Array,
}

#[wasm_bindgen]
impl RawRequestArguments {
    #[wasm_bindgen(getter)]
    pub fn method(&self) -> String { self.method.clone() }

    #[wasm_bindgen(getter)]
    pub fn params(&self) -> js_sys::Array { self.params.clone() }
}

async fn send_command_recv_response<Ok>(
    command_tx: &mut EthCommandSender,
    command: EthCommand,
    result_rx: oneshot::Receiver<EthProviderResult<Ok>>,
) -> EthProviderResult<Ok> {
    if let Err(e) = command_tx.try_send(command) {
        let error = format!("Error sending command: {}", e);
        return MmError::err(EthProviderError::Internal(error));
    }
    match result_rx.await {
        Ok(result) => result,
        Err(e) => {
            let error = format!("Error receiving result: {}", e);
            MmError::err(EthProviderError::Internal(error))
        },
    }
}

enum EthCommand {
    InvokeMethod {
        method: String,
        params: Vec<Json>,
        result_tx: EthCommandResultSender<Json>,
    },
}

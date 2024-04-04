use async_std::future::TimeoutError;
use async_std::prelude::FutureExt;
use common::expirable_map::ExpirableMap;
use common::{HttpStatusCode, StatusCode};
use derive_more::Display;
use futures::channel::oneshot;
use futures::lock::Mutex as AsyncMutex;
use instant::Duration;
use mm2_err_handle::prelude::*;
use mm2_event_stream::Event;
use ser_error_derive::SerializeErrorType;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{self, AtomicUsize};
use std::sync::Arc;

use crate::mm_ctx::{MmArc, MmCtx};

const EVENT_NAME: &str = "DATA_NEEDED";

#[derive(Clone, Debug, Default)]
pub struct DataAsker {
    data_id: Arc<AtomicUsize>,
    awaiting_asks: Arc<AsyncMutex<ExpirableMap<usize, oneshot::Sender<serde_json::Value>>>>,
}

#[derive(Debug, Display)]
pub enum AskForDataError {
    #[display(fmt = "Operation timed out. {}", _0)]
    Timeout(TimeoutError),
    #[display(fmt = "Expected JSON data, but given one(from asker) was not serializable: {:?}", _0)]
    SerializationError(serde_json::Error),
    #[display(
        fmt = "Expected JSON data, but given(from data provider) one was not deserializable: {:?}",
        _0
    )]
    DeserializationError(serde_json::Error),
    Internal(String),
}

impl From<TimeoutError> for AskForDataError {
    fn from(e: TimeoutError) -> Self { AskForDataError::Timeout(e) }
}

impl MmCtx {
    pub async fn ask_for_data<Input, Output>(
        &self,
        data_type: &str,
        data: Input,
        timeout: Duration,
    ) -> Result<Output, MmError<AskForDataError>>
    where
        Input: Serialize,
        Output: DeserializeOwned,
    {
        if data_type.contains(char::is_whitespace) {
            return MmError::err(AskForDataError::Internal(format!(
                "data_type can not contain whitespace, but got {data_type}"
            )));
        }

        let fut = async {
            let data_id = self.data_asker.data_id.fetch_add(1, atomic::Ordering::SeqCst);
            let (sender, receiver) = futures::channel::oneshot::channel::<serde_json::Value>();

            // We don't want to hold the lock, so call this in an inner-scope.
            {
                self.data_asker
                    .awaiting_asks
                    .lock()
                    .await
                    .insert(data_id, sender, timeout);
            }

            let data_input = match serde_json::to_string(&data) {
                Ok(t) => t,
                Err(e) => return MmError::err(AskForDataError::SerializationError(e)),
            };

            self.stream_channel_controller
                .broadcast(Event::new(EVENT_NAME.to_owned(), data_input))
                .await;

            match receiver.await {
                Ok(response) => match serde_json::from_value::<Output>(response) {
                    Ok(value) => Ok(value),
                    Err(error) => MmError::err(AskForDataError::DeserializationError(error)),
                },
                Err(error) => MmError::err(AskForDataError::Internal(format!(
                    "Sender channel is not alive. {error}"
                ))),
            }
        };

        fut.timeout(timeout).await?
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct SendAskedDataRequest {
    data_id: usize,
    data: serde_json::Value,
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum SendAskedDataError {
    #[display(fmt = "No data was asked for id={}", _0)]
    NotFound(usize),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl HttpStatusCode for SendAskedDataError {
    fn status_code(&self) -> StatusCode {
        match self {
            SendAskedDataError::NotFound(_) => StatusCode::NOT_FOUND,
            SendAskedDataError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn send_asked_data_rpc(
    ctx: MmArc,
    asked_data: SendAskedDataRequest,
) -> Result<bool, MmError<SendAskedDataError>> {
    let mut awaiting_asks = ctx.data_asker.awaiting_asks.lock().await;
    awaiting_asks.clear_expired_entries();

    match awaiting_asks.remove(&asked_data.data_id) {
        Some(sender) => {
            sender.send(asked_data.data).map_to_mm(|_| {
                SendAskedDataError::Internal("Receiver channel is not alive. Most likely timed out.".to_owned())
            })?;
            Ok(true)
        },
        None => MmError::err(SendAskedDataError::NotFound(asked_data.data_id)),
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
mod tests {
    use crate::mm_ctx::MmCtxBuilder;
    use common::executor::Timer;
    use common::{block_on, cfg_wasm32, cross_test};
    use instant::Duration;
    use serde::Deserialize;
    use serde_json::json;
    use std::thread;

    cfg_wasm32! {
        use wasm_bindgen_test::*;
        wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
    }

    cross_test!(simulate_ask_and_send_data, {
        let ctx = MmCtxBuilder::new().into_mm_arc();
        let ctx_clone = ctx.clone();

        #[derive(Clone, Debug, Deserialize)]
        struct DummyType {
            name: String,
        }

        thread::scope(|scope| {
            scope.spawn(move || {
                let output: DummyType =
                    block_on(ctx.ask_for_data("TEST", serde_json::Value::Null, Duration::from_secs(3))).unwrap();
                let output2: DummyType =
                    block_on(ctx.ask_for_data("TEST", serde_json::Value::Null, Duration::from_secs(3))).unwrap();

                // Assert values sent from the other thread.
                assert_eq!(&output.name, "Onur");
                assert_eq!(&output2.name, "Reggi");
            });

            scope.spawn(move || {
                // Wait until we ask for data from the other thread.
                common::block_on(Timer::sleep(1.));

                let data = super::SendAskedDataRequest {
                    data_id: 0,
                    data: json!({
                        "name": "Onur".to_owned()
                    }),
                };

                block_on(super::send_asked_data_rpc(ctx_clone.clone(), data)).unwrap();

                // Wait until we ask for data from the other thread.
                common::block_on(Timer::sleep(1.));

                let data = super::SendAskedDataRequest {
                    data_id: 1,
                    data: json!({
                        "name": "Reggi".to_owned()
                    }),
                };

                block_on(super::send_asked_data_rpc(ctx_clone, data)).unwrap();
            });
        });
    });
}

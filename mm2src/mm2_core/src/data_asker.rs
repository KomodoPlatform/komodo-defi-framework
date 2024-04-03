use common::expirable_map::ExpirableMap;
use futures::channel::oneshot;
use futures::lock::Mutex as AsyncMutex;
use instant::Duration;
use mm2_event_stream::Event;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{self, AtomicUsize};
use std::sync::Arc;

use crate::mm_ctx::{MmArc, MmCtx};

#[derive(Clone, Debug, Default)]
pub struct DataAsker {
    data_id: Arc<AtomicUsize>,
    awaiting_asks: Arc<AsyncMutex<ExpirableMap<usize, oneshot::Sender<serde_json::Value>>>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SendAskedData {
    id: usize,
    data: serde_json::Value,
}

impl MmCtx {
    /// TODO:
    pub async fn ask_for_data<Input, Output>(&self, data: Input) -> Output
    where
        Input: Serialize,
        Output: DeserializeOwned,
    {
        let data_id = self.data_asker.data_id.fetch_add(1, atomic::Ordering::SeqCst);
        let (sender, receiver) = futures::channel::oneshot::channel::<serde_json::Value>();

        self.data_asker
            .awaiting_asks
            .lock()
            .await
            .insert(data_id, sender, Duration::from_secs(10));

        self.stream_channel_controller
            .broadcast(Event::new(
                "DATA_NEEDED".to_string(),
                serde_json::to_string(&data).expect("TODO"),
            ))
            .await;

        match receiver.await {
            Ok(response) => {
                println!("000000000000000 {response}");
            },
            Err(error) => todo!("11111111111 {error}"),
        };

        todo!()
    }

    /// TODO:
    pub async fn send_asked_data(&self, asked_data: SendAskedData) {
        let mut awaiting_asks = self.data_asker.awaiting_asks.lock().await;
        awaiting_asks.clear_expired_entries();

        if let Some(sender) = awaiting_asks.remove(&asked_data.id) {
            sender.send(asked_data.data).expect("TODO");
        } else {
            panic!("TODO: DEBUGGING");
        };
    }
}

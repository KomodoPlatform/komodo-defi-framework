use crate::TransactionDetails;
use mm2_event_stream::{Broadcaster, Event, EventStreamer, StreamHandlerInput};

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::StreamExt;

pub struct TxHistoryEventStreamer {
    coin: String,
}

impl TxHistoryEventStreamer {
    #[inline(always)]
    pub fn new(coin: String) -> Self { Self { coin } }

    #[inline(always)]
    pub fn derive_streamer_id(coin: &str) -> String { format!("TX_HISTORY:{coin}") }
}

#[async_trait]
impl EventStreamer for TxHistoryEventStreamer {
    type DataInType = Vec<TransactionDetails>;

    fn streamer_id(&self) -> String { Self::derive_streamer_id(&self.coin) }

    async fn handle(
        self,
        broadcaster: Broadcaster,
        ready_tx: oneshot::Sender<Result<(), String>>,
        mut data_rx: impl StreamHandlerInput<Self::DataInType>,
    ) {
        ready_tx
            .send(Ok(()))
            .expect("Receiver is dropped, which should never happen.");

        while let Some(new_txs) = data_rx.next().await {
            for new_tx in new_txs {
                let tx_details = serde_json::to_value(new_tx).expect("Serialization should't fail.");
                broadcaster.broadcast(Event::new(self.streamer_id(), tx_details));
            }
        }
    }
}

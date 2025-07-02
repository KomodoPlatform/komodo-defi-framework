use super::{MakerMatch, TakerMatch};
use mm2_event_stream::{Broadcaster, Event, EventStreamer, StreamHandlerInput, StreamerId};

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::StreamExt;

pub struct OrderStatusStreamer;

impl OrderStatusStreamer {
    #[inline(always)]
    pub fn new() -> Self { Self }

    #[inline(always)]
    pub const fn derive_streamer_id() -> StreamerId { StreamerId::OrderStatus }
}

#[derive(Serialize)]
#[serde(tag = "order_type", content = "order_data")]
pub enum OrderStatusEvent {
    MakerMatch(MakerMatch),
    TakerMatch(TakerMatch),
    MakerConnected(MakerMatch),
    TakerConnected(TakerMatch),
}

#[async_trait]
impl EventStreamer for OrderStatusStreamer {
    type DataInType = OrderStatusEvent;

    fn streamer_id(&self) -> StreamerId { Self::derive_streamer_id() }

    async fn handle(
        self,
        broadcaster: Broadcaster,
        ready_tx: oneshot::Sender<Result<(), String>>,
        mut data_rx: impl StreamHandlerInput<Self::DataInType>,
    ) {
        ready_tx
            .send(Ok(()))
            .expect("Receiver is dropped, which should never happen.");

        while let Some(order_data) = data_rx.next().await {
            let event_data = serde_json::to_value(order_data).expect("Serialization shouldn't fail.");
            let event = Event::new(self.streamer_id(), event_data);
            broadcaster.broadcast(event);
        }
    }
}

use crate::mm_ctx::MmCtx;
use async_trait::async_trait;
use futures::channel::oneshot;
use mm2_event_stream::behaviour::{EventBehaviour, EventInitStatus};
use mm2_event_stream::EventStreamConfiguration;

#[async_trait]
impl EventBehaviour for MmCtx {
    const EVENT_NAME: &'static str = "SEND_AND_RECEIVE";
    const ERROR_EVENT_NAME: &'static str = "SEND_AND_RECEIVE_ERROR";

    async fn handle(self, _interval: f64, _tx: oneshot::Sender<EventInitStatus>) {}
    async fn spawn_if_active(self, _config: &EventStreamConfiguration) -> EventInitStatus { todo!() }
}

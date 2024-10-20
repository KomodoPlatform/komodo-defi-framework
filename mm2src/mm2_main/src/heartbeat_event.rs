use async_trait::async_trait;
use common::{executor::{SpawnFuture, Timer},
             log::info};
use futures::channel::oneshot::{self, Receiver, Sender};
use mm2_core::mm_ctx::MmArc;
use mm2_event_stream::{behaviour::{EventBehaviour, EventInitStatus},
                       Event, EventName, EventStreamConfiguration};

pub struct HeartbeatEvent {
    ctx: MmArc,
}

impl HeartbeatEvent {
    pub fn new(ctx: MmArc) -> Self { Self { ctx } }
}

#[async_trait]
impl EventBehaviour for HeartbeatEvent {
    fn event_name() -> EventName { EventName::HEARTBEAT }

    async fn handle(self, interval: f64, tx: oneshot::Sender<EventInitStatus>) {
        tx.send(EventInitStatus::Success).unwrap();

        loop {
            self.ctx
                .stream_channel_controller
                .broadcast(Event::new(Self::event_name().to_string(), json!({}).to_string()))
                .await;

            Timer::sleep(interval).await;
        }
    }

    async fn spawn_if_active(self, config: &EventStreamConfiguration) -> EventInitStatus {
        if let Some(event) = config.get_event(&Self::event_name()) {
            info!(
                "{} event is activated with {} seconds interval.",
                Self::event_name(),
                event.stream_interval_seconds
            );

            let (tx, rx): (Sender<EventInitStatus>, Receiver<EventInitStatus>) = oneshot::channel();
            self.ctx.spawner().spawn(self.handle(event.stream_interval_seconds, tx));

            rx.await.unwrap_or_else(|e| {
                EventInitStatus::Failed(format!("Event initialization status must be received: {}", e))
            })
        } else {
            EventInitStatus::Inactive
        }
    }
}

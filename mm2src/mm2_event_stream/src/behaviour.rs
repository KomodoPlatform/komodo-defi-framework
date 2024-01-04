use crate::EventStreamConfiguration;
use async_trait::async_trait;
use futures::channel::oneshot;

#[derive(Clone, Debug)]
pub enum EventInitStatus {
    Inactive,
    Success,
    Failed(String),
}

#[async_trait]
pub trait EventBehaviour {
    /// Unique name of the event.
    const EVENT_NAME: &'static str;

    /// Event handler that is responsible for broadcasting event data to the streaming channels.
    async fn handle(self, interval: f64, tx: oneshot::Sender<EventInitStatus>);

    /// Spawns the `Self::handle` in a separate thread if the event is active according to the mm2 configuration.
    /// Does nothing if the event is not active.
    async fn spawn_if_active(self, config: &EventStreamConfiguration) -> EventInitStatus;
}

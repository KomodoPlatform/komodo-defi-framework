use crate::p2p::P2PContext;
use async_trait::async_trait;
use common::{executor::{SpawnFuture, Timer},
             log::info};
use futures::channel::oneshot::{self, Receiver, Sender};
use mm2_core::mm_ctx::MmArc;
pub use mm2_event_stream::behaviour::EventBehaviour;
use mm2_event_stream::{behaviour::EventInitStatus, Event, EventStreamConfiguration};
use mm2_libp2p::behaviours::atomicdex;
use serde_json::json;

pub struct NetworkEvent {
    ctx: MmArc,
}

impl NetworkEvent {
    pub fn new(ctx: MmArc) -> Self { Self { ctx } }
}

#[async_trait]
impl EventBehaviour for NetworkEvent {
    const EVENT_NAME: &'static str = "NETWORK";

    async fn handle(self, interval: f64, tx: oneshot::Sender<EventInitStatus>) {
        let p2p_ctx = P2PContext::fetch_from_mm_arc(&self.ctx);
        let mut previously_sent = json!({});

        tx.send(EventInitStatus::Success).unwrap();

        loop {
            let p2p_cmd_tx = p2p_ctx.cmd_tx.lock().clone();

            let peers_info = atomicdex::get_peers_info(p2p_cmd_tx.clone()).await;
            let gossip_mesh = atomicdex::get_gossip_mesh(p2p_cmd_tx.clone()).await;
            let gossip_peer_topics = atomicdex::get_gossip_peer_topics(p2p_cmd_tx.clone()).await;
            let gossip_topic_peers = atomicdex::get_gossip_topic_peers(p2p_cmd_tx.clone()).await;
            let relay_mesh = atomicdex::get_relay_mesh(p2p_cmd_tx).await;

            let event_data = json!({
                "peers_info": peers_info,
                "gossip_mesh": gossip_mesh,
                "gossip_peer_topics": gossip_peer_topics,
                "gossip_topic_peers": gossip_topic_peers,
                "relay_mesh": relay_mesh,
            });

            if previously_sent != event_data {
                self.ctx
                    .stream_channel_controller
                    .broadcast(Event::new(Self::EVENT_NAME.to_string(), event_data.to_string()))
                    .await;

                previously_sent = event_data;
            }

            Timer::sleep(interval).await;
        }
    }

    async fn spawn_if_active(self, config: &EventStreamConfiguration) -> EventInitStatus {
        if let Some(event) = config.get_event(Self::EVENT_NAME) {
            info!(
                "NETWORK event is activated with {} seconds interval.",
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

use crate::p2p::P2PContext;
use async_trait::async_trait;
use common::{executor::{SpawnFuture, Timer},
             log::info};
use mm2_core::mm_ctx::MmArc;
pub use mm2_event_stream::behaviour::EventBehaviour;
use mm2_event_stream::{Event, EventStreamConfiguration};
use mm2_libp2p::atomicdex_behaviour;
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

    async fn handle(self, interval: f64) {
        let p2p_ctx = P2PContext::fetch_from_mm_arc(&self.ctx);

        loop {
            let p2p_cmd_tx = p2p_ctx.cmd_tx.lock().clone();

            let peers_info = atomicdex_behaviour::get_peers_info(p2p_cmd_tx.clone()).await;
            let gossip_mesh = atomicdex_behaviour::get_gossip_mesh(p2p_cmd_tx.clone()).await;
            let gossip_peer_topics = atomicdex_behaviour::get_gossip_peer_topics(p2p_cmd_tx.clone()).await;
            let gossip_topic_peers = atomicdex_behaviour::get_gossip_topic_peers(p2p_cmd_tx.clone()).await;
            let relay_mesh = atomicdex_behaviour::get_relay_mesh(p2p_cmd_tx).await;

            let event_data = json!({
                "peers_info": peers_info,
                "gossip_mesh": gossip_mesh,
                "gossip_peer_topics": gossip_peer_topics,
                "gossip_topic_peers": gossip_topic_peers,
                "relay_mesh": relay_mesh,
            });

            self.ctx
                .stream_channel_controller
                .broadcast(Event::new(Self::EVENT_NAME.to_string(), event_data.to_string()))
                .await;

            Timer::sleep(interval).await;
        }
    }

    fn spawn_if_active(self, config: &EventStreamConfiguration) {
        if let Some(event) = config.get_event(Self::EVENT_NAME) {
            info!(
                "NETWORK event is activated with {} seconds interval.",
                event.stream_interval_seconds
            );
            self.ctx.spawner().spawn(self.handle(event.stream_interval_seconds));
        }
    }
}

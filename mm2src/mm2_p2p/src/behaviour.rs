use futures::channel::mpsc::{Receiver, Sender};
use instant::Duration;

use crate::event::AdexBehaviourEvent;

// pub type AdexCmdTx = Sender<AdexBehaviourCmd>;
pub type AdexEventRx = Receiver<AdexBehaviourEvent>;

pub const PEERS_TOPIC: &str = "PEERS";
const CONNECTED_RELAYS_CHECK_INTERVAL: Duration = Duration::from_secs(30);
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(600);
const ANNOUNCE_INITIAL_DELAY: Duration = Duration::from_secs(60);
const CHANNEL_BUF_SIZE: usize = 1024 * 8;

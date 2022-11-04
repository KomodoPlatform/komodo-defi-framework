// Copyright 2020 Sigma Prime Pty Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use crate::config::GossipsubConfig;
use crate::handler::GossipsubHandler;
use crate::mcache::MessageCache;
use crate::protocol::{GossipsubControlAction, GossipsubMessage, GossipsubSubscription, GossipsubSubscriptionAction,
                      MessageId};
use crate::topic::{Topic, TopicHash};
use common::time_cache::{Entry as TimeCacheEntry, TimeCache};
use futures::prelude::*;
use libp2p_core::{connection::ConnectionId, ConnectedPoint, Multiaddr, PeerId};
use libp2p_swarm::{IntoConnectionHandler, NetworkBehaviour, NetworkBehaviourAction, NotifyHandler, PollParameters};
use log::{debug, error, info, trace, warn};
use rand::seq::SliceRandom;
use smallvec::SmallVec;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::iter;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use wasm_timer::{Instant, Interval};

mod tests;

/// Network behaviour that handles the gossipsub protocol.
pub struct Gossipsub {
    /// Configuration providing gossipsub performance parameters.
    config: GossipsubConfig,

    /// Events that need to be yielded to the outside when polling.
    events: VecDeque<NetworkBehaviourAction<GossipsubEvent, GossipsubHandler, Arc<GossipsubRpc>>>,

    /// Pools non-urgent control messages between heartbeats.
    control_pool: HashMap<PeerId, Vec<GossipsubControlAction>>,

    /// Peer id of the local node. Used for the source of the messages that we publish.
    local_peer_id: PeerId,

    /// A map of all connected peers - A map of topic hash to a list of gossipsub peer Ids.
    topic_peers: HashMap<TopicHash, Vec<PeerId>>,

    /// A map of all connected peers to their subscribed topics.
    peer_topics: HashMap<PeerId, Vec<TopicHash>>,

    /// The peer ids of connected relay nodes
    connected_relays: HashSet<PeerId>,

    /// relays to which we forward the messages. Also tracks the relay mesh size of nodes in mesh.
    relays_mesh: HashMap<PeerId, usize>,

    /// Peers included our node to their relays mesh
    included_to_relays_mesh: HashSet<PeerId>,

    /// Overlay network of connected peers - Maps topics to connected gossipsub peers.
    mesh: HashMap<TopicHash, Vec<PeerId>>,

    /// Map of topics to list of peers that we publish to, but don't subscribe to.
    fanout: HashMap<TopicHash, Vec<PeerId>>,

    /// The last publish time for fanout topics.
    fanout_last_pub: HashMap<TopicHash, Instant>,

    /// Message cache for the last few heartbeats.
    mcache: MessageCache,

    /// We keep track of the messages we received (in the format `string(source ID, seq_no)`) so that
    /// we don't dispatch the same message twice if we receive it twice on the network.
    /// Also store the peers from which message was received so we don't manually propagate already known message to them
    received: TimeCache<MessageId, SmallVec<[PeerId; 12]>>,

    /// Heartbeat interval stream.
    heartbeat: Interval,

    /// Relay mesh maintenance interval stream.
    relay_mesh_maintenance_interval: Interval,

    peer_connections: HashMap<PeerId, Vec<(ConnectionId, ConnectedPoint)>>,

    connected_addresses: Vec<Multiaddr>,

    /// The relay list which are forcefully kept in relay mesh
    explicit_relay_list: Vec<PeerId>,
}

impl Gossipsub {
    /// Creates a `Gossipsub` struct given a set of parameters specified by `gs_config`.
    pub fn new(local_peer_id: PeerId, gs_config: GossipsubConfig) -> Self {
        let local_peer_id = if gs_config.no_source_id {
            PeerId::from_bytes(&crate::config::IDENTITY_SOURCE).expect("Valid peer id")
        } else {
            local_peer_id
        };

        Gossipsub {
            config: gs_config.clone(),
            events: VecDeque::new(),
            control_pool: HashMap::new(),
            local_peer_id,
            topic_peers: HashMap::new(),
            peer_topics: HashMap::new(),
            mesh: HashMap::new(),
            fanout: HashMap::new(),
            fanout_last_pub: HashMap::new(),
            mcache: MessageCache::new(
                gs_config.history_gossip,
                gs_config.history_length,
                gs_config.message_id_fn,
            ),
            received: TimeCache::new(gs_config.duplicate_cache_time),
            heartbeat: Interval::new_at(
                Instant::now() + gs_config.heartbeat_initial_delay,
                gs_config.heartbeat_interval,
            ),
            relay_mesh_maintenance_interval: Interval::new_at(
                Instant::now() + Duration::from_secs(10),
                Duration::from_secs(10),
            ),
            peer_connections: HashMap::new(),
            connected_relays: HashSet::new(),
            relays_mesh: HashMap::new(),
            included_to_relays_mesh: HashSet::new(),
            connected_addresses: Vec::new(),
            explicit_relay_list: Vec::new(),
        }
    }

    pub fn add_explicit_relay(&mut self, peer_id: PeerId) {
        info!("Adding peer {} to explicit relay list", peer_id);
        self.explicit_relay_list.push(peer_id);
    }

    /// Subscribe to a topic.
    ///
    /// Returns true if the subscription worked. Returns false if we were already subscribed.
    pub fn subscribe(&mut self, topic: Topic) -> bool {
        debug!("Subscribing to topic: {}", topic);
        if self.config.i_am_relay {
            debug!("Relay is subscribed to all topics by default. Subscribe has no effect.");
            return false;
        }
        let topic_hash = self.topic_hash(topic.clone());
        if self.mesh.get(&topic_hash).is_some() {
            debug!("Topic: {} is already in the mesh.", topic);
            return false;
        }

        let peers: Vec<_> = self.peer_topics.keys().copied().collect();
        for peer_id in peers {
            let mut fixed_event = None; // initialise the event once if needed
            if fixed_event.is_none() {
                fixed_event = Some(Arc::new(GossipsubRpc {
                    messages: Vec::new(),
                    subscriptions: vec![GossipsubSubscription {
                        topic_hash: topic_hash.clone(),
                        action: GossipsubSubscriptionAction::Subscribe,
                    }],
                    control_msgs: Vec::new(),
                }));
            }

            let event = fixed_event.expect("event has been initialised");

            debug!("Sending SUBSCRIBE to peer: {:?}", peer_id);
            self.notify_primary(peer_id, event);
        }

        // call JOIN(topic)
        // this will add new peers to the mesh for the topic
        self.join(&topic_hash);
        info!("Subscribed to topic: {}", topic);
        true
    }

    pub fn is_subscribed(&self, topic_hash: &TopicHash) -> bool { self.mesh.contains_key(topic_hash) }

    /// Unsubscribes from a topic.
    ///
    /// Returns true if we were subscribed to this topic.
    pub fn unsubscribe(&mut self, topic: Topic) -> bool {
        debug!("Unsubscribing from topic: {}", topic);
        if self.config.i_am_relay {
            debug!("Relay is subscribed to all topics by default. Unsubscribe has no effect");
            return false;
        }
        let topic_hash = &self.topic_hash(topic);

        if self.mesh.get(topic_hash).is_none() {
            debug!("Already unsubscribed from topic: {:?}", topic_hash);
            // we are not subscribed
            return false;
        }

        // announce to all peers in the topic
        let mut fixed_event = None; // initialise the event once if needed
        if let Some(peer_list) = self.topic_peers.get(topic_hash) {
            if fixed_event.is_none() {
                fixed_event = Some(Arc::new(GossipsubRpc {
                    messages: Vec::new(),
                    subscriptions: vec![GossipsubSubscription {
                        topic_hash: topic_hash.clone(),
                        action: GossipsubSubscriptionAction::Unsubscribe,
                    }],
                    control_msgs: Vec::new(),
                }));
            }

            let event = fixed_event.expect("event has been initialised");

            for peer in peer_list.clone() {
                debug!("Sending UNSUBSCRIBE to peer: {:?}", peer);
                self.notify_primary(peer, event.clone());
            }
        }

        // call LEAVE(topic)
        // this will remove the topic from the mesh
        self.leave(topic_hash);

        info!("Unsubscribed from topic: {:?}", topic_hash);
        true
    }

    /// Publishes a message to the network.
    pub fn publish(&mut self, topic: &Topic, data: impl Into<Vec<u8>>) {
        self.publish_many_from(iter::once(topic.clone()), data, self.local_peer_id)
    }

    /// Publishes a message with multiple topics to the network.
    pub fn publish_many(&mut self, topic: impl IntoIterator<Item = Topic>, data: impl Into<Vec<u8>>) {
        self.publish_many_from(topic, data, self.local_peer_id);
    }

    /// Publishes a message with multiple topics to the network.
    pub fn publish_many_from(
        &mut self,
        topic: impl IntoIterator<Item = Topic>,
        data: impl Into<Vec<u8>>,
        source: PeerId,
    ) {
        let message = GossipsubMessage {
            source,
            data: data.into(),
            // To be interoperable with the go-implementation this is treated as a 64-bit
            // big-endian uint.
            sequence_number: rand::random(),
            topics: topic.into_iter().map(|t| self.topic_hash(t)).collect(),
        };

        debug!("Publishing message: {:?}", (self.config.message_id_fn)(&message));

        // forward the message to mesh peers
        self.forward_msg(message.clone(), &source);

        let mut recipient_peers = HashSet::new();
        for topic_hash in &message.topics {
            // if not subscribed to the topic, use fanout peers
            if self.mesh.get(topic_hash).is_none() {
                debug!("Topic: {:?} not in the mesh", topic_hash);
                // build a list of peers to forward the message to
                // if we have fanout peers add them to the map
                if self.fanout.contains_key(topic_hash) {
                    for peer in self.fanout.get(topic_hash).expect("Topic must exist") {
                        recipient_peers.insert(*peer);
                    }
                } else {
                    // we have no fanout peers, select mesh_n of them and add them to the fanout
                    let mesh_n = self.config.mesh_n;
                    let new_peers = Self::get_random_peers(&self.topic_peers, topic_hash, mesh_n, |_| true);
                    // add the new peers to the fanout and recipient peers
                    self.fanout.insert(topic_hash.clone(), new_peers.clone());
                    for peer in new_peers {
                        debug!("Peer added to fanout: {:?}", peer);
                        recipient_peers.insert(peer);
                    }
                }
                // we are publishing to fanout peers - update the time we published
                self.fanout_last_pub.insert(topic_hash.clone(), Instant::now());
            }
        }

        // add published message to our received caches
        let msg_id = (self.config.message_id_fn)(&message);
        self.mcache.put(message.clone());
        self.received.insert(msg_id.clone(), SmallVec::from_elem(source, 1));

        debug!("Published message: {:?}", msg_id);

        let event = Arc::new(GossipsubRpc {
            subscriptions: Vec::new(),
            messages: vec![message],
            control_msgs: Vec::new(),
        });
        // Send to peers we know are subscribed to the topic.
        for peer_id in recipient_peers.iter() {
            debug!("Sending message to peer: {:?}", peer_id);
            self.notify_primary(*peer_id, event.clone());
        }
    }

    /// This function should be called when `config.manual_propagation` is `true` in order to
    /// propagate messages. Messages are stored in the ['Memcache'] and validation is expected to be
    /// fast enough that the messages should still exist in the cache.
    ///
    /// Calling this function will propagate a message stored in the cache, if it still exists.
    /// If the message still exists in the cache, it will be forwarded and this function will return true,
    /// otherwise it will return false.
    pub fn propagate_message(&mut self, message_id: &MessageId, propagation_source: &PeerId) -> bool {
        let message = match self.mcache.get(message_id) {
            Some(message) => message.clone(),
            None => {
                warn!(
                    "Message not in cache. Ignoring forwarding. Message Id: {}",
                    message_id.0
                );
                return false;
            },
        };
        self.forward_msg(message, propagation_source);
        true
    }

    /// Gossipsub JOIN(topic) - adds topic peers to mesh and sends them GRAFT messages.
    fn join(&mut self, topic_hash: &TopicHash) {
        debug!("Running JOIN for topic: {:?}", topic_hash);

        // if we are already in the mesh, return
        if self.mesh.contains_key(topic_hash) {
            info!("JOIN: The topic is already in the mesh, ignoring JOIN");
            return;
        }

        let mut added_peers = vec![];

        // check if we have mesh_n peers in fanout[topic] and add them to the mesh if we do,
        // removing the fanout entry.
        if let Some((_, peers)) = self.fanout.remove_entry(topic_hash) {
            debug!("JOIN: Removing peers from the fanout for topic: {:?}", topic_hash);
            // add up to mesh_n of them them to the mesh
            // Note: These aren't randomly added, currently FIFO
            let add_peers = std::cmp::min(peers.len(), self.config.mesh_n);
            debug!(
                "JOIN: Adding {:?} peers from the fanout for topic: {:?}",
                add_peers, topic_hash
            );
            added_peers.extend_from_slice(&peers[..add_peers]);
            self.mesh.insert(topic_hash.clone(), peers[..add_peers].to_vec());
            // remove the last published time
            self.fanout_last_pub.remove(topic_hash);
        }

        // check if we need to get more peers, which we randomly select
        if added_peers.len() < self.config.mesh_n {
            // get the peers
            let new_peers = Self::get_random_peers(
                &self.topic_peers,
                topic_hash,
                self.config.mesh_n - added_peers.len(),
                |_| true,
            );
            added_peers.extend_from_slice(&new_peers);
            // add them to the mesh
            debug!("JOIN: Inserting {:?} random peers into the mesh", new_peers.len());
            let mesh_peers = self.mesh.entry(topic_hash.clone()).or_insert_with(Vec::new);
            mesh_peers.extend_from_slice(&new_peers);
        }

        for peer_id in added_peers {
            // Send a GRAFT control message
            info!("JOIN: Sending Graft message to peer: {:?}", peer_id);
            Self::control_pool_add(&mut self.control_pool, peer_id, GossipsubControlAction::Graft {
                topic_hash: topic_hash.clone(),
            });
        }
        debug!("Completed JOIN for topic: {:?}", topic_hash);
    }

    /// Gossipsub LEAVE(topic) - Notifies mesh\[topic\] peers with PRUNE messages.
    fn leave(&mut self, topic_hash: &TopicHash) {
        debug!("Running LEAVE for topic {:?}", topic_hash);

        // if our mesh contains the topic, send prune to peers and delete it from the mesh
        if let Some((_, peers)) = self.mesh.remove_entry(topic_hash) {
            for peer in peers {
                // Send a PRUNE control message
                info!("LEAVE: Sending PRUNE to peer: {:?}", peer);
                Self::control_pool_add(&mut self.control_pool, peer, GossipsubControlAction::Prune {
                    topic_hash: topic_hash.clone(),
                });
            }
        }
        debug!("Completed LEAVE for topic: {:?}", topic_hash);
    }

    /// Handles an IHAVE control message. Checks our cache of messages. If the message is unknown,
    /// requests it with an IWANT control message.
    fn handle_ihave(&mut self, peer_id: &PeerId, ihave_msgs: Vec<(TopicHash, Vec<MessageId>)>) {
        debug!("Handling IHAVE for peer: {:?}", peer_id);
        // use a hashset to avoid duplicates efficiently
        let mut iwant_ids = HashSet::new();

        for (topic, ids) in ihave_msgs {
            // only process the message if we are subscribed
            if !self.mesh.contains_key(&topic) {
                debug!("IHAVE: Ignoring IHAVE - Not subscribed to topic: {:?}", topic);
                continue;
            }

            for id in ids {
                if !self.received.contains_key(&id) {
                    // have not seen this message, request it
                    iwant_ids.insert(id);
                }
            }
        }

        if !iwant_ids.is_empty() {
            // Send the list of IWANT control messages
            debug!("IHAVE: Sending IWANT message");
            Self::control_pool_add(&mut self.control_pool, *peer_id, GossipsubControlAction::IWant {
                message_ids: iwant_ids.iter().cloned().collect(),
            });
        }
        debug!("Completed IHAVE handling for peer: {:?}", peer_id);
    }

    /// Handles an IWANT control message. Checks our cache of messages. If the message exists it is
    /// forwarded to the requesting peer.
    fn handle_iwant(&mut self, peer_id: &PeerId, iwant_msgs: Vec<MessageId>) {
        debug!("Handling IWANT for peer: {:?}", peer_id);
        // build a hashmap of available messages
        let mut cached_messages = HashMap::new();

        for id in iwant_msgs {
            // if we have it, add it do the cached_messages mapping
            if let Some(msg) = self.mcache.get(&id) {
                cached_messages.insert(id.clone(), msg.clone());
            }
        }

        if !cached_messages.is_empty() {
            debug!("IWANT: Sending cached messages to peer: {:?}", peer_id);
            // Send the messages to the peer
            let message_list = cached_messages.into_iter().map(|entry| entry.1).collect();
            let event = Arc::new(GossipsubRpc {
                subscriptions: Vec::new(),
                messages: message_list,
                control_msgs: Vec::new(),
            });
            self.notify_primary(*peer_id, event);
        }
        debug!("Completed IWANT handling for peer: {:?}", peer_id);
    }

    /// Handles GRAFT control messages. If subscribed to the topic, adds the peer to mesh, if not,
    /// responds with PRUNE messages.
    fn handle_graft(&mut self, peer_id: &PeerId, topics: Vec<TopicHash>) {
        debug!("Handling GRAFT message for peer: {:?}", peer_id);

        let mut to_prune_topics = HashSet::new();
        for topic_hash in topics {
            if let Some(peers) = self.mesh.get_mut(&topic_hash) {
                // if we are subscribed, add peer to the mesh, if not already added
                info!(
                    "GRAFT: Mesh link added for peer: {:?} in topic: {:?}",
                    peer_id, topic_hash
                );
                // ensure peer is not already added
                if !peers.contains(peer_id) {
                    peers.push(*peer_id);
                }
            } else {
                to_prune_topics.insert(topic_hash.clone());
            }
        }

        if !to_prune_topics.is_empty() {
            // build the prune messages to send
            let prune_messages = to_prune_topics
                .iter()
                .map(|t| GossipsubControlAction::Prune { topic_hash: t.clone() })
                .collect();
            // Send the prune messages to the peer
            info!(
                "GRAFT: Not subscribed to topics -  Sending PRUNE to peer: {:?}",
                peer_id
            );
            let event = Arc::new(GossipsubRpc {
                subscriptions: Vec::new(),
                messages: Vec::new(),
                control_msgs: prune_messages,
            });
            self.notify_primary(*peer_id, event);
        }
        debug!("Completed GRAFT handling for peer: {:?}", peer_id);
    }

    /// Handles PRUNE control messages. Removes peer from the mesh.
    fn handle_prune(&mut self, peer_id: &PeerId, topics: Vec<TopicHash>) {
        debug!("Handling PRUNE message for peer: {:?}", peer_id);
        for topic_hash in topics {
            if let Some(peers) = self.mesh.get_mut(&topic_hash) {
                // remove the peer if it exists in the mesh
                info!(
                    "PRUNE: Removing peer: {:?} from the mesh for topic: {:?}",
                    peer_id, topic_hash
                );
                peers.retain(|p| p != peer_id);
            }
        }
        debug!("Completed PRUNE handling for peer: {:?}", peer_id);
    }

    /// Handles IAmrelay control message, does nothing if remote peer already subscribed to some topic
    fn handle_i_am_relay(&mut self, peer_id: &PeerId, is_relay: bool) {
        debug!("Handling IAmrelay message for peer: {:?}", peer_id);
        if self.peer_topics.entry(*peer_id).or_insert_with(Vec::new).is_empty() && is_relay {
            info!("IAmrelay: Adding peer: {:?} to the relays list", peer_id);
            self.connected_relays.insert(*peer_id);
            if self.relays_mesh.len() < self.config.mesh_n_low {
                info!("IAmrelay: Adding peer: {:?} to the relay mesh", peer_id);
                self.add_peers_to_relays_mesh(vec![*peer_id]);
            }
        }
        debug!("Completed IAmrelay handling for peer: {:?}", peer_id);
    }

    /// Handles IncludedTorelaysMesh message
    fn handle_included_to_relays_mesh(&mut self, peer_id: &PeerId, is_included: bool, other_mesh_size: usize) {
        if self.is_relay() {
            debug!(
                "Handling IncludedTorelaysMesh message for peer: {:?}, is_included: {}",
                peer_id, is_included
            );
            if is_included {
                if self.connected_relays.contains(peer_id) {
                    if self.relays_mesh.len() > self.config.mesh_n_high {
                        self.notify_excluded_from_relay_mesh(*peer_id);
                    } else {
                        debug!("Adding peer {:?} to relays_mesh", peer_id);
                        self.relays_mesh.insert(*peer_id, other_mesh_size);
                    }
                } else {
                    debug!("Adding peer {:?} to included_to_relays_mesh", peer_id);
                    self.included_to_relays_mesh.insert(*peer_id);
                }
            } else {
                debug!(
                    "Removing peer {:?} from included_to_relays_mesh and relays mesh",
                    peer_id
                );
                self.included_to_relays_mesh.remove(peer_id);
                self.relays_mesh.remove(peer_id);
            }
        } else {
            debug!(
                "Ignoring IncludedTorelaysMesh message for peer: {:?}, is_included: {}",
                peer_id, is_included
            );
        }
    }

    /// Handles a newly received GossipsubMessage.
    /// Forwards the message to all peers in the mesh.
    fn handle_received_message(&mut self, msg: GossipsubMessage, propagation_source: &PeerId) {
        let msg_id = (self.config.message_id_fn)(&msg);
        debug!("Handling message: {:?} from peer: {:?}", msg_id, propagation_source);
        match self.received.entry(msg_id.clone()) {
            TimeCacheEntry::Occupied(entry) => {
                debug!("Message already received, ignoring. Message: {:?}", msg_id);
                entry.into_mut().push(*propagation_source);
                return;
            },
            TimeCacheEntry::Vacant(entry) => {
                entry.insert(SmallVec::from_elem(*propagation_source, 1));
            },
        }
        // add to the memcache
        self.mcache.put(msg.clone());

        // dispatch the message to the user
        debug!("Sending received message to user");
        self.events
            .push_back(NetworkBehaviourAction::GenerateEvent(GossipsubEvent::Message(
                *propagation_source,
                msg_id,
                msg.clone(),
            )));

        // forward the message to mesh peers, if no validation is required
        if !self.config.manual_propagation {
            let message_id = (self.config.message_id_fn)(&msg);
            self.forward_msg(msg, propagation_source);
            debug!("Completed message handling for message: {:?}", message_id);
        }
    }

    /// Handles received subscriptions.
    fn handle_received_subscriptions(&mut self, subscriptions: &[GossipsubSubscription], propagation_source: &PeerId) {
        debug!(
            "Handling subscriptions: {:?}, from source: {:?}",
            subscriptions, propagation_source
        );
        let subscribed_topics = match self.peer_topics.get_mut(propagation_source) {
            Some(topics) => topics,
            None => {
                error!("Subscription by unknown peer: {:?}", &propagation_source);
                return;
            },
        };

        for subscription in subscriptions {
            // get the peers from the mapping, or insert empty lists if topic doesn't exist
            let peer_list = self
                .topic_peers
                .entry(subscription.topic_hash.clone())
                .or_insert_with(Vec::new);

            match subscription.action {
                GossipsubSubscriptionAction::Subscribe => {
                    if !peer_list.contains(propagation_source) {
                        debug!(
                            "SUBSCRIPTION: topic_peer: Adding gossip peer: {:?} to topic: {:?}",
                            propagation_source, subscription.topic_hash
                        );
                        peer_list.push(*propagation_source);
                    }

                    // add to the peer_topics mapping
                    if !subscribed_topics.contains(&subscription.topic_hash) {
                        info!(
                            "SUBSCRIPTION: Adding peer: {:?} to topic: {:?}",
                            propagation_source, subscription.topic_hash
                        );
                        subscribed_topics.push(subscription.topic_hash.clone());
                    }

                    // if the mesh needs peers add the peer to the mesh
                    if let Some(peers) = self.mesh.get_mut(&subscription.topic_hash) {
                        if peers.len() < self.config.mesh_n_low {
                            debug!("SUBSCRIPTION: Adding peer {:?} to the mesh", propagation_source,);
                        }
                        peers.push(*propagation_source);
                    }
                    // generates a subscription event to be polled
                    self.events
                        .push_back(NetworkBehaviourAction::GenerateEvent(GossipsubEvent::Subscribed {
                            peer_id: *propagation_source,
                            topic: subscription.topic_hash.clone(),
                        }));
                },
                GossipsubSubscriptionAction::Unsubscribe => {
                    if let Some(pos) = peer_list.iter().position(|p| p == propagation_source) {
                        info!(
                            "SUBSCRIPTION: Removing gossip peer: {:?} from topic: {:?}",
                            propagation_source, subscription.topic_hash
                        );
                        peer_list.remove(pos);
                    }
                    // remove topic from the peer_topics mapping
                    if let Some(pos) = subscribed_topics.iter().position(|t| t == &subscription.topic_hash) {
                        subscribed_topics.remove(pos);
                    }
                    // remove the peer from the mesh if it exists
                    if let Some(peers) = self.mesh.get_mut(&subscription.topic_hash) {
                        peers.retain(|peer| peer != propagation_source);
                    }

                    // generate an unsubscribe event to be polled
                    self.events
                        .push_back(NetworkBehaviourAction::GenerateEvent(GossipsubEvent::Unsubscribed {
                            peer_id: *propagation_source,
                            topic: subscription.topic_hash.clone(),
                        }));
                },
            }
        }
        trace!("Completed handling subscriptions from source: {:?}", propagation_source);
    }

    /// Heartbeat function which shifts the memcache and updates the mesh.
    fn heartbeat(&mut self) {
        debug!("Starting heartbeat");

        let mut to_graft = HashMap::new();
        let mut to_prune = HashMap::new();

        // maintain the mesh for each topic
        for (topic_hash, peers) in self.mesh.iter_mut() {
            // too little peers - add some
            if peers.len() < self.config.mesh_n_low {
                debug!(
                    "HEARTBEAT: Mesh low. Topic: {:?} Contains: {:?} needs: {:?}",
                    topic_hash.clone().into_string(),
                    peers.len(),
                    self.config.mesh_n_low
                );
                // not enough peers - get mesh_n - current_length more
                let desired_peers = self.config.mesh_n - peers.len();
                let peer_list = Self::get_random_peers(&self.topic_peers, topic_hash, desired_peers, {
                    |peer| !peers.contains(peer)
                });
                for peer in &peer_list {
                    let current_topic = to_graft.entry(*peer).or_insert_with(Vec::new);
                    current_topic.push(topic_hash.clone());
                }
                // update the mesh
                debug!("Updating mesh, new mesh: {:?}", peer_list);
                peers.extend(peer_list);
            }

            // too many peers - remove some
            if peers.len() > self.config.mesh_n_high {
                debug!(
                    "HEARTBEAT: Mesh high. Topic: {:?} Contains: {:?} needs: {:?}",
                    topic_hash,
                    peers.len(),
                    self.config.mesh_n_high
                );
                let excess_peer_no = peers.len() - self.config.mesh_n;
                // shuffle the peers
                let mut rng = rand::thread_rng();
                peers.shuffle(&mut rng);
                // remove the first excess_peer_no peers adding them to to_prune
                for _ in 0..excess_peer_no {
                    let peer = peers.pop().expect("There should always be enough peers to remove");
                    let current_topic = to_prune.entry(peer).or_insert_with(Vec::new);
                    current_topic.push(topic_hash.clone());
                }
            }
        }

        // remove expired fanout topics
        {
            let fanout = &mut self.fanout; // help the borrow checker
            let fanout_ttl = self.config.fanout_ttl;
            self.fanout_last_pub.retain(|topic_hash, last_pub_time| {
                if *last_pub_time + fanout_ttl < Instant::now() {
                    debug!(
                        "HEARTBEAT: Fanout topic removed due to timeout. Topic: {:?}",
                        topic_hash
                    );
                    fanout.remove(topic_hash);
                    return false;
                }
                true
            });
        }

        // maintain fanout
        // check if our peers are still a part of the topic
        for (topic_hash, peers) in self.fanout.iter_mut() {
            let mut to_remove_peers = Vec::new();
            for peer in peers.iter() {
                // is the peer still subscribed to the topic?
                match self.peer_topics.get(peer) {
                    Some(topics) => {
                        if !topics.contains(topic_hash) {
                            debug!("HEARTBEAT: Peer removed from fanout for topic: {:?}", topic_hash);
                            to_remove_peers.push(*peer);
                        }
                    },
                    None => {
                        // remove if the peer has disconnected
                        to_remove_peers.push(*peer);
                    },
                }
            }
            peers.retain(|peer| !to_remove_peers.contains(peer));

            // not enough peers
            if peers.len() < self.config.mesh_n {
                debug!(
                    "HEARTBEAT: Fanout low. Contains: {:?} needs: {:?}",
                    peers.len(),
                    self.config.mesh_n
                );
                let needed_peers = self.config.mesh_n - peers.len();
                let new_peers = Self::get_random_peers(&self.topic_peers, topic_hash, needed_peers, |peer| {
                    !peers.contains(peer)
                });
                peers.extend(new_peers);
            }
        }

        self.emit_gossip();

        // send graft/prunes
        if !to_graft.is_empty() | !to_prune.is_empty() {
            self.send_graft_prune(to_graft, to_prune);
        }

        // piggyback pooled control messages
        self.flush_control_pool();

        // shift the memcache
        self.mcache.shift();
        debug!("Completed Heartbeat");
    }

    /// Emits gossip - Send IHAVE messages to a random set of gossip peers. This is applied to mesh
    /// and fanout peers
    fn emit_gossip(&mut self) {
        debug!("Started gossip");
        for (topic_hash, peers) in self.mesh.iter().chain(self.fanout.iter()) {
            let message_ids = self.mcache.get_gossip_ids(topic_hash);
            if message_ids.is_empty() {
                return;
            }

            // get gossip_lazy random peers
            let to_msg_peers = Self::get_random_peers(&self.topic_peers, topic_hash, self.config.gossip_lazy, |peer| {
                !peers.contains(peer)
            });
            for peer in to_msg_peers {
                // send an IHAVE message
                Self::control_pool_add(&mut self.control_pool, peer, GossipsubControlAction::IHave {
                    topic_hash: topic_hash.clone(),
                    message_ids: message_ids.clone(),
                });
            }
        }
        debug!("Completed gossip");
    }

    /// Handles multiple GRAFT/PRUNE messages and coalesces them into chunked gossip control
    /// messages.
    fn send_graft_prune(
        &mut self,
        to_graft: HashMap<PeerId, Vec<TopicHash>>,
        mut to_prune: HashMap<PeerId, Vec<TopicHash>>,
    ) {
        // handle the grafts and overlapping prunes
        for (peer, topics) in to_graft.iter() {
            let mut grafts: Vec<GossipsubControlAction> = topics
                .iter()
                .map(|topic_hash| GossipsubControlAction::Graft {
                    topic_hash: topic_hash.clone(),
                })
                .collect();
            let mut prunes: Vec<GossipsubControlAction> = to_prune
                .remove(peer)
                .unwrap_or_default()
                .iter()
                .map(|topic_hash| GossipsubControlAction::Prune {
                    topic_hash: topic_hash.clone(),
                })
                .collect();
            grafts.append(&mut prunes);

            // send the control messages
            let event = Arc::new(GossipsubRpc {
                subscriptions: Vec::new(),
                messages: Vec::new(),
                control_msgs: grafts,
            });
            self.notify_primary(*peer, event);
        }

        // handle the remaining prunes
        for (peer, topics) in to_prune.iter() {
            let remaining_prunes = topics
                .iter()
                .map(|topic_hash| GossipsubControlAction::Prune {
                    topic_hash: topic_hash.clone(),
                })
                .collect();
            let event = Arc::new(GossipsubRpc {
                subscriptions: Vec::new(),
                messages: Vec::new(),
                control_msgs: remaining_prunes,
            });
            self.notify_primary(*peer, event);
        }
    }

    /// Helper function which forwards a message to mesh\[topic\] peers.
    fn forward_msg(&mut self, message: GossipsubMessage, source: &PeerId) {
        let msg_id = (self.config.message_id_fn)(&message);
        debug!("Forwarding message: {:?}", msg_id);
        let mut recipient_peers = HashSet::new();

        if self.config.i_am_relay {
            // relay simply forwards the message to topic peers that included the relay to their relays mesh
            for topic in &message.topics {
                if let Some(topic_peers) = self.topic_peers.get(topic) {
                    for peer_id in topic_peers {
                        if peer_id != source
                            && peer_id != &message.source
                            && self.included_to_relays_mesh.contains(peer_id)
                        {
                            recipient_peers.insert(*peer_id);
                        }
                    }
                }
            }
        } else {
            // add mesh peers if the node is not relay
            for topic in &message.topics {
                if let Some(mesh_peers) = self.mesh.get(topic) {
                    for peer_id in mesh_peers {
                        if peer_id != source && peer_id != &message.source {
                            recipient_peers.insert(*peer_id);
                        }
                    }
                }
            }
        }

        // forward the message to peers
        if !recipient_peers.is_empty() {
            let event = Arc::new(GossipsubRpc {
                subscriptions: Vec::new(),
                messages: vec![message.clone()],
                control_msgs: Vec::new(),
            });

            for peer in recipient_peers.iter() {
                if let Some(received_from_peers) = self.received.get(&msg_id) {
                    if received_from_peers.contains(peer) {
                        continue;
                    }
                }

                debug!("Sending message: {:?} to peer {:?}", msg_id, peer);
                self.notify_primary(*peer, event.clone());
            }
        }

        if !self.relays_mesh.is_empty() {
            debug!("Forwarding message to relays: {:?}", msg_id);
            let message_source = message.source;
            let event = Arc::new(GossipsubRpc {
                subscriptions: Vec::new(),
                messages: vec![message],
                control_msgs: Vec::new(),
            });

            let relays: Vec<_> = self.relays_mesh.keys().copied().collect();
            for relay in relays {
                if let Some(received_from_peers) = self.received.get(&msg_id) {
                    if received_from_peers.contains(&relay) {
                        continue;
                    }
                }

                if relay != *source && relay != message_source {
                    debug!("Sending message: {:?} to relay {:?}", msg_id, relay);
                    self.notify_primary(relay, event.clone());
                }
            }
            debug!("Completed forwarding message to relays");
        }
        debug!("Completed forwarding message");
    }

    /// Helper function to get a set of `n` random gossipsub peers for a `topic_hash`
    /// filtered by the function `f`.
    fn get_random_peers(
        topic_peers: &HashMap<TopicHash, Vec<PeerId>>,
        topic_hash: &TopicHash,
        n: usize,
        mut f: impl FnMut(&PeerId) -> bool,
    ) -> Vec<PeerId> {
        let mut gossip_peers = match topic_peers.get(topic_hash) {
            // if they exist, filter the peers by `f`
            Some(peer_list) => peer_list.iter().cloned().filter(|p| f(p)).collect(),
            None => Vec::new(),
        };

        // if we have less than needed, return them
        if gossip_peers.len() <= n {
            debug!("RANDOM PEERS: Got {:?} peers", gossip_peers.len());
            return gossip_peers.to_vec();
        }

        // we have more peers than needed, shuffle them and return n of them
        let mut rng = rand::thread_rng();
        gossip_peers.partial_shuffle(&mut rng, n);

        debug!("RANDOM PEERS: Got {:?} peers", n);

        gossip_peers[..n].to_vec()
    }

    /// The helper function to get a set of `n` random peers from the `relays` hashset
    /// filtered by the function `f`.
    fn get_random_relays(relays: &HashSet<PeerId>, n: usize, mut f: impl FnMut(&PeerId) -> bool) -> Vec<PeerId> {
        let mut relays: Vec<_> = relays.iter().cloned().filter(|p| f(p)).collect();

        // if we have less than needed, return them
        if relays.len() <= n {
            debug!("RANDOM RELAYS: Got {:?} peers", relays.len());
            return relays;
        }

        // we have more peers than needed, shuffle them and return n of them
        let mut rng = rand::thread_rng();
        relays.partial_shuffle(&mut rng, n);
        debug!("RANDOM RELAYS: Got {:?} peers", n);

        relays[..n].to_vec()
    }

    // adds a control action to control_pool
    fn control_pool_add(
        control_pool: &mut HashMap<PeerId, Vec<GossipsubControlAction>>,
        peer: PeerId,
        control: GossipsubControlAction,
    ) {
        control_pool.entry(peer).or_insert_with(Vec::new).push(control);
    }

    /// Produces a `TopicHash` for a topic given the gossipsub configuration.
    fn topic_hash(&self, topic: Topic) -> TopicHash {
        if self.config.hash_topics {
            topic.sha256_hash()
        } else {
            topic.no_hash()
        }
    }

    /// Takes each control action mapping and turns it into a message
    fn flush_control_pool(&mut self) {
        let control_pool: Vec<_> = self.control_pool.drain().collect();
        for (peer, controls) in control_pool {
            let event = Arc::new(GossipsubRpc {
                subscriptions: Vec::new(),
                messages: Vec::new(),
                control_msgs: controls,
            });
            self.notify_primary(peer, event);
        }
    }

    pub fn get_mesh_peers(&self, topic: &TopicHash) -> Vec<PeerId> { self.mesh.get(topic).cloned().unwrap_or_default() }

    pub fn get_topic_peers(&self, topic: &TopicHash) -> Vec<PeerId> {
        self.topic_peers.get(topic).cloned().unwrap_or_default()
    }

    pub fn get_num_peers(&self) -> usize { self.peer_topics.len() }

    pub fn get_peers_connections(&self) -> HashMap<PeerId, Vec<(ConnectionId, ConnectedPoint)>> {
        self.peer_connections.clone()
    }

    pub fn get_mesh(&self) -> &HashMap<TopicHash, Vec<PeerId>> { &self.mesh }

    pub fn get_relay_mesh(&self) -> Vec<PeerId> { self.relays_mesh.keys().cloned().collect() }

    pub fn relay_mesh_len(&self) -> usize { self.relays_mesh.len() }

    pub fn get_all_topic_peers(&self) -> &HashMap<TopicHash, Vec<PeerId>> { &self.topic_peers }

    pub fn get_all_peer_topics(&self) -> &HashMap<PeerId, Vec<TopicHash>> { &self.peer_topics }

    /// Get count of received messages in the [`GossipsubConfig::duplicate_cache_time`] period.
    pub fn get_received_messages_in_period(&self) -> (Duration, usize) { (self.received.ttl(), self.received.len()) }

    pub fn get_config(&self) -> &GossipsubConfig { &self.config }

    /// Adds peers to relays mesh and notifies them they are added
    fn add_peers_to_relays_mesh(&mut self, peers: Vec<PeerId>) {
        for peer in &peers {
            // other mesh size is unknown at this point
            self.relays_mesh.insert(*peer, 0);
        }
        for peer in peers {
            self.notify_included_to_relay_mesh(peer);
        }
    }

    #[allow(dead_code)]
    fn remove_peer_from_relay_mesh(&mut self, peer: &PeerId) {
        if self.relays_mesh.remove(peer).is_some() {
            self.notify_excluded_from_relay_mesh(*peer)
        }
    }

    /// Cleans up relays mesh so it remains mesh_n peers
    fn clean_up_relays_mesh(&mut self) {
        let mesh_n = self.config.mesh_n;
        let mut removed = Vec::with_capacity(self.relays_mesh.len() - mesh_n);
        let explicit_relay_list = self.explicit_relay_list.clone();
        // perform 2 filter iterations to not keep excessive number of explicit peers in mesh
        self.relays_mesh = self
            .relays_mesh
            .drain()
            .enumerate()
            .filter_map(|(i, peer)| {
                if i < mesh_n || explicit_relay_list.contains(&peer.0) {
                    Some(peer)
                } else {
                    removed.push(peer);
                    None
                }
            })
            .collect();

        self.relays_mesh = self
            .relays_mesh
            .drain()
            .enumerate()
            .filter_map(|(i, peer)| {
                if i < mesh_n {
                    Some(peer)
                } else {
                    removed.push(peer);
                    None
                }
            })
            .collect();

        for (peer, _) in removed {
            self.notify_excluded_from_relay_mesh(peer)
        }
    }

    fn notify_included_to_relay_mesh(&mut self, peer: PeerId) {
        let event = Arc::new(GossipsubRpc {
            subscriptions: Vec::new(),
            messages: Vec::new(),
            control_msgs: vec![GossipsubControlAction::IncludedToRelaysMesh {
                included: true,
                mesh_size: self.relay_mesh_len(),
            }],
        });
        self.notify_primary(peer, event);
    }

    fn notify_excluded_from_relay_mesh(&mut self, peer: PeerId) {
        let event = Arc::new(GossipsubRpc {
            subscriptions: Vec::new(),
            messages: Vec::new(),
            control_msgs: vec![GossipsubControlAction::IncludedToRelaysMesh {
                included: false,
                mesh_size: self.relay_mesh_len(),
            }],
        });
        self.notify_primary(peer, event);
    }

    /// Notify the primary connection (the first connected point) of the peer.
    /// Since `NotifyHandler::All` has been removed, the original `libp2p_gossipsub` notifies the connected peers using their primary connections.
    /// See an example: https://github.com/libp2p/rust-libp2p/blob/v0.38.0/protocols/gossipsub/src/behaviour.rs#L3013
    fn notify_primary(&mut self, peer_id: PeerId, event: Arc<GossipsubRpc>) {
        if let Some(points) = self.peer_connections.get(&peer_id) {
            if !points.is_empty() {
                let conn_id = points[0].0;
                return self.notify_one(peer_id, conn_id, event);
            }
        }
        warn!("Expected at least one connection of the peer '{}'", peer_id);
        self.notify_any(peer_id, event);
    }

    #[allow(dead_code)]
    fn notify_all(&mut self, peer_id: PeerId, event: Arc<GossipsubRpc>) {
        match self.peer_connections.get(&peer_id) {
            Some(connected_points) => {
                let connections: Vec<_> = connected_points.iter().map(|(conn_id, _point)| *conn_id).collect();
                for conn_id in connections {
                    self.notify_one(peer_id, conn_id, event.clone());
                }
            },
            None => {
                warn!(
                    "An attempt to notify a peer '{:?}' that is not in 'Gossipsub::peer_connections'",
                    peer_id
                )
            },
        }
    }

    fn notify_any(&mut self, peer_id: PeerId, event: Arc<GossipsubRpc>) {
        self.events.push_back(NetworkBehaviourAction::NotifyHandler {
            peer_id,
            handler: NotifyHandler::Any,
            event,
        });
    }

    fn notify_one(&mut self, peer_id: PeerId, conn_id: ConnectionId, event: Arc<GossipsubRpc>) {
        self.events.push_back(NetworkBehaviourAction::NotifyHandler {
            peer_id,
            handler: NotifyHandler::One(conn_id),
            event,
        });
    }

    fn maintain_relays_mesh(&mut self) {
        if self.relays_mesh.len() < self.config.mesh_n_low {
            info!(
                "HEARTBEAT: relays low. Contains: {:?} needs: {:?}",
                self.relays_mesh.len(),
                self.config.mesh_n_low,
            );
            // add peers 1 by 1 to avoid overloading peaks when node connects to several other nodes at once
            let required = 1;
            // get `n` relays that are not in the `relays_mesh`
            let to_add =
                Self::get_random_relays(&self.connected_relays, required, |p| !self.relays_mesh.contains_key(p));
            self.add_peers_to_relays_mesh(to_add);
        }

        if self.relays_mesh.len() > self.config.mesh_n_high {
            info!(
                "HEARTBEAT: relays high. Contains: {:?} needs: {:?}",
                self.relays_mesh.len(),
                self.config.mesh_n,
            );
            self.clean_up_relays_mesh();
        }

        let size = self.relays_mesh.len();
        for relay in self.relays_mesh.keys() {
            Self::control_pool_add(&mut self.control_pool, *relay, GossipsubControlAction::MeshSize(size));
        }
    }

    pub fn is_relay(&self) -> bool { self.config.i_am_relay }

    pub fn connected_relays(&self) -> Vec<PeerId> { self.connected_relays.iter().cloned().collect() }

    pub fn connected_relays_len(&self) -> usize { self.connected_relays.len() }

    pub fn is_connected_to_addr(&self, addr: &Multiaddr) -> bool { self.connected_addresses.contains(addr) }
}

impl NetworkBehaviour for Gossipsub {
    type ConnectionHandler = GossipsubHandler;
    type OutEvent = GossipsubEvent;

    fn new_handler(&mut self) -> Self::ConnectionHandler {
        GossipsubHandler::new(self.config.protocol_id.clone(), self.config.max_transmit_size)
    }

    fn addresses_of_peer(&mut self, _: &PeerId) -> Vec<Multiaddr> { Vec::new() }

    fn inject_connection_established(
        &mut self,
        id: &PeerId,
        conn_id: &ConnectionId,
        point: &ConnectedPoint,
        _: Option<&Vec<Multiaddr>>,
        other_established: usize,
    ) {
        self.peer_connections
            .entry(*id)
            .or_insert_with(Default::default)
            .push((*conn_id, point.clone()));
        self.connected_addresses.push(point.get_remote_address().clone());

        if other_established > 0 {
            // For other actions, we only care about the first time a peer connects.
            return;
        }

        info!("New peer connected: {:?}", id);
        // We need to send our subscriptions to the newly-connected node if we are not relay.
        // Notify peer that we act as relay otherwise
        if self.config.i_am_relay {
            debug!("Sending IAmRelay to peer {:?}", id);
            let event = Arc::new(GossipsubRpc {
                messages: Vec::new(),
                subscriptions: Vec::new(),
                control_msgs: vec![GossipsubControlAction::IAmRelay(true)],
            });
            self.notify_primary(*id, event);
        } else {
            let mut subscriptions = vec![];
            for topic_hash in self.mesh.keys() {
                subscriptions.push(GossipsubSubscription {
                    topic_hash: topic_hash.clone(),
                    action: GossipsubSubscriptionAction::Subscribe,
                });
            }

            if !subscriptions.is_empty() {
                // send our subscriptions to the peer
                let event = Arc::new(GossipsubRpc {
                    messages: Vec::new(),
                    subscriptions,
                    control_msgs: Vec::new(),
                });
                self.notify_primary(*id, event);
            }
        }
        // For the time being assume all gossipsub peers
        self.peer_topics.insert(*id, Vec::new());
    }

    fn inject_connection_closed(
        &mut self,
        peer_id: &PeerId,
        disconnected_conn_id: &ConnectionId,
        disconnected_point: &ConnectedPoint,
        _: <Self::ConnectionHandler as IntoConnectionHandler>::Handler,
        remaining_established: usize,
    ) {
        if let Entry::Occupied(mut o) = self.peer_connections.entry(*peer_id) {
            let connected_points = o.get_mut();
            connected_points.retain(|(conn_id, _point)| conn_id != disconnected_conn_id);
            if connected_points.is_empty() {
                o.remove_entry();
            }
        }

        self.connected_addresses
            .retain(|addr| addr != disconnected_point.get_remote_address());

        if remaining_established > 0 {
            return;
        }

        // remove from mesh, topic_peers, peer_topic and fanout
        debug!("Peer disconnected: {:?}", peer_id);
        {
            let topics = match self.peer_topics.get(peer_id) {
                Some(topics) => topics,
                None => {
                    warn!("Disconnected node, not in connected nodes");
                    return;
                },
            };

            // remove peer from all mappings
            for topic in topics {
                // check the mesh for the topic
                if let Some(mesh_peers) = self.mesh.get_mut(topic) {
                    // check if the peer is in the mesh and remove it
                    if let Some(pos) = mesh_peers.iter().position(|p| p == peer_id) {
                        mesh_peers.remove(pos);
                    }
                }

                // remove from topic_peers
                if let Some(peer_list) = self.topic_peers.get_mut(topic) {
                    if let Some(pos) = peer_list.iter().position(|p| p == peer_id) {
                        peer_list.remove(pos);
                    }
                    // debugging purposes
                    else {
                        warn!("Disconnected node: {:?} not in topic_peers peer list", peer_id);
                    }
                } else {
                    warn!(
                        "Disconnected node: {:?} with topic: {:?} not in topic_peers",
                        &peer_id, &topic
                    );
                }

                // remove from fanout
                if let Some(peers) = self.fanout.get_mut(topic) {
                    peers.retain(|p| p != peer_id)
                }
            }
        }

        self.relays_mesh.remove(peer_id);
        self.connected_relays.remove(peer_id);
        self.included_to_relays_mesh.remove(peer_id);
        self.peer_connections.remove(peer_id);
        // remove peer from peer_topics
        let was_in = self.peer_topics.remove(peer_id);
        debug_assert!(was_in.is_some());
    }

    fn inject_event(&mut self, propagation_source: PeerId, _: ConnectionId, event: GossipsubRpc) {
        // Handle subscriptions
        // Update connected peers topics
        debug!("Event injected {:?}, source {:?}", event, propagation_source);
        self.handle_received_subscriptions(&event.subscriptions, &propagation_source);

        // Handle messages
        for message in event.messages {
            self.handle_received_message(message, &propagation_source);
        }

        // Handle control messages
        // group some control messages, this minimises SendEvents (code is simplified to handle each event at a time however)
        let mut ihave_msgs = vec![];
        let mut graft_msgs = vec![];
        let mut prune_msgs = vec![];
        for control_msg in event.control_msgs {
            match control_msg {
                GossipsubControlAction::IHave {
                    topic_hash,
                    message_ids,
                } => {
                    ihave_msgs.push((topic_hash, message_ids));
                },
                GossipsubControlAction::IWant { message_ids } => self.handle_iwant(&propagation_source, message_ids),
                GossipsubControlAction::Graft { topic_hash } => graft_msgs.push(topic_hash),
                GossipsubControlAction::Prune { topic_hash } => prune_msgs.push(topic_hash),
                GossipsubControlAction::IAmRelay(is_relay) => self.handle_i_am_relay(&propagation_source, is_relay),
                GossipsubControlAction::IncludedToRelaysMesh { included, mesh_size } => {
                    self.handle_included_to_relays_mesh(&propagation_source, included, mesh_size)
                },
                GossipsubControlAction::MeshSize(size) => {
                    if let Some(old_size) = self.relays_mesh.get_mut(&propagation_source) {
                        *old_size = size;
                    }
                },
            }
        }
        if !ihave_msgs.is_empty() {
            self.handle_ihave(&propagation_source, ihave_msgs);
        }
        if !graft_msgs.is_empty() {
            self.handle_graft(&propagation_source, graft_msgs);
        }
        if !prune_msgs.is_empty() {
            self.handle_prune(&propagation_source, prune_msgs);
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context,
        _: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<Self::OutEvent, Self::ConnectionHandler>> {
        if let Some(event) = self.events.pop_front() {
            // clone send event reference if others references are present
            match event {
                NetworkBehaviourAction::NotifyHandler {
                    peer_id,
                    handler,
                    event: send_event,
                } => match Arc::try_unwrap(send_event) {
                    Ok(event) => {
                        return Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                            peer_id,
                            event,
                            handler,
                        });
                    },
                    Err(event) => {
                        return Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                            peer_id,
                            event: (*event).clone(),
                            handler,
                        });
                    },
                },
                NetworkBehaviourAction::GenerateEvent(e) => {
                    return Poll::Ready(NetworkBehaviourAction::GenerateEvent(e));
                },
                NetworkBehaviourAction::Dial { opts, handler } => {
                    return Poll::Ready(NetworkBehaviourAction::Dial { opts, handler });
                },
                NetworkBehaviourAction::ReportObservedAddr { address, score } => {
                    return Poll::Ready(NetworkBehaviourAction::ReportObservedAddr { address, score });
                },
                NetworkBehaviourAction::CloseConnection { peer_id, connection } => {
                    return Poll::Ready(NetworkBehaviourAction::CloseConnection { peer_id, connection });
                },
            }
        }

        while let Poll::Ready(Some(())) = self.heartbeat.poll_next_unpin(cx) {
            self.heartbeat();
        }

        while let Poll::Ready(Some(())) = self.relay_mesh_maintenance_interval.poll_next_unpin(cx) {
            self.maintain_relays_mesh();
        }

        Poll::Pending
    }
}

/// An RPC received/sent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GossipsubRpc {
    /// List of messages that were part of this RPC query.
    pub messages: Vec<GossipsubMessage>,
    /// List of subscriptions.
    pub subscriptions: Vec<GossipsubSubscription>,
    /// List of Gossipsub control messages.
    pub control_msgs: Vec<GossipsubControlAction>,
}

/// Event that can happen on the gossipsub behaviour.
#[derive(Debug)]
pub enum GossipsubEvent {
    /// A message has been received. This contains the PeerId that we received the message from,
    /// the message id (used if the application layer needs to propagate the message) and the
    /// message itself.
    Message(PeerId, MessageId, GossipsubMessage),

    /// A remote subscribed to a topic.
    Subscribed {
        /// Remote that has subscribed.
        peer_id: PeerId,
        /// The topic it has subscribed to.
        topic: TopicHash,
    },

    /// A remote unsubscribed from a topic.
    Unsubscribed {
        /// Remote that has unsubscribed.
        peer_id: PeerId,
        /// The topic it has subscribed from.
        topic: TopicHash,
    },
}

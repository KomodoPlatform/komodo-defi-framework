use serde::Serialize;
use std::collections::HashMap;

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "get_gossip_mesh")]
pub(crate) struct GetGossipMeshRequest {}

pub(crate) type GetGossipMeshResponse = HashMap<String, Vec<String>>;

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "get_gossip_peer_topics")]
pub(crate) struct GetGossipPeerTopicsRequest {}

pub(crate) type GetGossipPeerTopicsResponse = HashMap<String, Vec<String>>;

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "get_relay_mesh")]
pub(crate) struct GetRelayMeshRequest {}

pub(crate) type GetRelayMeshResponse = Vec<String>;

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "get_gossip_topic_peers")]
pub(crate) struct GetGossipTopicPeersRequest {}

pub(crate) type GetGossipTopicPeersResponse = HashMap<String, Vec<String>>;

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "get_my_peer_id")]
pub(crate) struct GetMyPeerIdRequest {}

pub(crate) type GetMyPeerIdResponse = String;

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "get_peers_info")]
pub(crate) struct GetPeersInfoRequest {}

pub(crate) type GetPeersInfoResponse = HashMap<String, Vec<String>>;

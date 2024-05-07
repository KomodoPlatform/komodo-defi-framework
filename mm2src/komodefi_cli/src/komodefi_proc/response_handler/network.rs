use std::io::Write;

use common::write_safe::io::WriteSafeIO;
use common::{write_safe_io, writeln_safe_io};

use super::formatters::{writeln_field, ZERO_INDENT};
use crate::rpc_data::{GetGossipMeshResponse, GetGossipPeerTopicsResponse, GetGossipTopicPeersResponse,
                      GetMyPeerIdResponse, GetPeersInfoResponse, GetRelayMeshResponse};

pub(super) fn on_gossip_mesh(writer: &mut dyn Write, response: GetGossipMeshResponse) {
    if response.is_empty() {
        writeln_field(writer, "gossip_mesh", "empty", ZERO_INDENT);
        return;
    }
    writeln_field(writer, "gossip_mesh", "", ZERO_INDENT);
    for (k, v) in response {
        writeln_field(
            writer,
            k,
            if v.is_empty() { "empty".to_string() } else { v.join(",") },
            ZERO_INDENT,
        );
    }
}

pub(super) fn on_gossip_peer_topics(writer: &mut dyn Write, response: GetGossipPeerTopicsResponse) {
    if response.is_empty() {
        writeln_field(writer, "gossip_peer_topics", "empty", ZERO_INDENT);
        return;
    }
    writeln_field(writer, "gossip_peer_topics", "", ZERO_INDENT);
    for (key, value) in response {
        writeln_field(
            writer,
            key,
            if value.is_empty() {
                "empty".to_string()
            } else {
                value.join(",")
            },
            ZERO_INDENT,
        );
    }
}

pub(super) fn on_gossip_topic_peers(writer: &mut dyn Write, response: GetGossipTopicPeersResponse) {
    if response.is_empty() {
        writeln_field(writer, "gossip_topic_peers", "empty", ZERO_INDENT);
        return;
    }
    writeln_field(writer, "gossip_topic_peers", "", ZERO_INDENT);
    for (key, value) in response {
        writeln_field(
            writer,
            key,
            if value.is_empty() {
                "empty".to_string()
            } else {
                value.join(",")
            },
            ZERO_INDENT,
        );
    }
}

pub(super) fn on_relay_mesh(writer: &mut dyn Write, response: GetRelayMeshResponse) {
    if response.is_empty() {
        writeln_field(writer, "relay_mesh", "empty", ZERO_INDENT);
        return;
    }
    writeln_field(writer, "relay_mesh", "", ZERO_INDENT);
    for value in response {
        writeln_safe_io!(writer, "{}", value);
    }
}

pub(super) fn on_my_peer_id(writer: &mut dyn Write, response: GetMyPeerIdResponse) {
    writeln_safe_io!(writer, "{}", response)
}

pub(super) fn on_peers_info(writer: &mut dyn Write, response: GetPeersInfoResponse) {
    if response.is_empty() {
        writeln_field(writer, "peers_info", "empty", ZERO_INDENT);
        return;
    }
    writeln_field(writer, "peers_info", "", ZERO_INDENT);
    for (key, value) in response {
        writeln_field(
            writer,
            key,
            if value.is_empty() {
                "empty".to_string()
            } else {
                value.join(",")
            },
            ZERO_INDENT,
        );
    }
}

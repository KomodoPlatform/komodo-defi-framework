use clap::Subcommand;

#[allow(clippy::enum_variant_names)]
#[derive(Subcommand)]
pub(crate) enum NetworkCommands {
    #[command(
        visible_alias = "gossip-mesh",
        about = "Return an array of peerIDs added to a topics' mesh for each known gossipsub topic"
    )]
    GetGossipMesh,
    #[command(
        visible_alias = "relay-mesh",
        about = "Return a list of peerIDs included in our local relay mesh"
    )]
    GetRelayMesh,
    #[command(
        visible_alias = "peer-topics",
        about = "Return a map of peerIDs to an array of the topics to which they are subscribed"
    )]
    GetGossipPeerTopics,
    #[command(
        visible_alias = "topic-peers",
        about = "Return a map of topics to an array of the PeerIDs which are subscribers"
    )]
    GetGossipTopicPeers,
    #[command(
        visible_alias = "my-peer-id",
        about = "Return your unique identifying Peer ID on the network"
    )]
    GetMyPeerId,
    #[command(
        visible_alias = "peers-info",
        about = "Return all connected peers with their multiaddresses"
    )]
    GetPeersInfo,
}

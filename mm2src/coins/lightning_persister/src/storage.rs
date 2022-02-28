use async_trait::async_trait;
use lightning::routing::network_graph::NetworkGraph;
use lightning::routing::scoring::Scorer;
use parking_lot::Mutex as PaMutex;
use secp256k1::PublicKey;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

pub type NodesAddressesMap = HashMap<PublicKey, SocketAddr>;
pub type NodesAddressesMapShared = Arc<PaMutex<NodesAddressesMap>>;

#[async_trait]
pub trait Storage: Send + Sync + 'static {
    type Error;

    /// Initializes dirs/collection/tables in storage for a specified coin
    async fn init(&self) -> Result<(), Self::Error>;

    async fn is_initialized(&self) -> Result<bool, Self::Error>;

    async fn get_nodes_addresses(&self) -> Result<HashMap<PublicKey, SocketAddr>, Self::Error>;

    async fn save_nodes_addresses(&self, nodes_addresses: NodesAddressesMapShared) -> Result<(), Self::Error>;

    async fn get_network_graph(&self) -> Result<NetworkGraph, Self::Error>;

    async fn save_network_graph(&self, network_graph: Arc<NetworkGraph>) -> Result<(), Self::Error>;

    async fn get_scorer(&self) -> Result<Scorer, Self::Error>;

    async fn save_scorer(&self, scorer: Arc<Mutex<Scorer>>) -> Result<(), Self::Error>;
}

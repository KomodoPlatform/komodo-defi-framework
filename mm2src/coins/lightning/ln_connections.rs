use super::*;
use common::executor::{spawn, Timer};
use derive_more::Display;
use tokio::net::TcpListener;

const TRY_RECONNECTING_TO_NODE_INTERVAL: f64 = 60.;

pub async fn ln_p2p_loop(peer_manager: Arc<PeerManager>, listener: TcpListener) {
    loop {
        let peer_mgr = peer_manager.clone();
        let tcp_stream = match listener.accept().await {
            Ok((stream, addr)) => {
                log::debug!("New incoming lightning connection from node address: {}", addr);
                stream
            },
            Err(e) => {
                log::error!("Error on accepting lightning connection: {}", e);
                continue;
            },
        };
        if let Ok(stream) = tcp_stream.into_std() {
            spawn(async move {
                lightning_net_tokio::setup_inbound(peer_mgr.clone(), stream).await;
            });
        };
    }
}

#[derive(Display)]
pub enum ConnectToNodeRes {
    #[display(fmt = "Already connected to node: {}@{}", _0, _1)]
    AlreadyConnected(String, String),
    #[display(fmt = "Connected successfully to node : {}@{}", _0, _1)]
    ConnectedSuccessfully(String, String),
}

pub async fn connect_to_node(
    pubkey: PublicKey,
    node_addr: SocketAddr,
    peer_manager: Arc<PeerManager>,
) -> ConnectToNodeResult<ConnectToNodeRes> {
    if peer_manager.get_peer_node_ids().contains(&pubkey) {
        return Ok(ConnectToNodeRes::AlreadyConnected(
            pubkey.to_string(),
            node_addr.to_string(),
        ));
    }

    match lightning_net_tokio::connect_outbound(Arc::clone(&peer_manager), pubkey, node_addr).await {
        Some(connection_closed_future) => {
            let mut connection_closed_future = Box::pin(connection_closed_future);
            loop {
                // Make sure the connection is still established.
                match futures::poll!(&mut connection_closed_future) {
                    std::task::Poll::Ready(_) => {
                        return MmError::err(ConnectToNodeError::ConnectionError(format!(
                            "Node {} disconnected before finishing the handshake",
                            pubkey
                        )));
                    },
                    std::task::Poll::Pending => {},
                }

                match peer_manager.get_peer_node_ids().contains(&pubkey) {
                    true => break,
                    // Wait for the handshake to complete if false.
                    false => Timer::sleep_ms(10).await,
                }
            }
        },
        None => {
            return MmError::err(ConnectToNodeError::ConnectionError(format!(
                "Failed to connect to node: {}",
                pubkey
            )))
        },
    }

    Ok(ConnectToNodeRes::ConnectedSuccessfully(
        pubkey.to_string(),
        node_addr.to_string(),
    ))
}

pub async fn connect_to_nodes_loop(
    nodes_addresses: Arc<PaMutex<HashMap<PublicKey, SocketAddr>>>,
    peer_manager: Arc<PeerManager>,
) {
    loop {
        let nodes_addresses = nodes_addresses.lock().clone();
        for (pubkey, node_addr) in nodes_addresses {
            let peer_manager = peer_manager.clone();
            match connect_to_node(pubkey, node_addr, peer_manager.clone()).await {
                Ok(res) => {
                    if let ConnectToNodeRes::ConnectedSuccessfully(_, _) = res {
                        log::info!("{}", res.to_string());
                    }
                },
                Err(e) => log::error!("{}", e.to_string()),
            }
        }

        Timer::sleep(TRY_RECONNECTING_TO_NODE_INTERVAL).await;
    }
}

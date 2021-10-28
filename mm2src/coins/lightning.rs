#[cfg(not(target_arch = "wasm32"))]
use crate::utxo::rpc_clients::UtxoRpcClientEnum;
#[cfg(not(target_arch = "wasm32"))]
use common::ip_addr::myipaddr;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use ln_errors::{ConnectToNodeError, ConnectToNodeResult, EnableLightningError, EnableLightningResult,
                OpenChannelError, OpenChannelResult};
#[cfg(not(target_arch = "wasm32"))]
use ln_utils::{connect_to_node, network_from_string, nodes_data_path, open_ln_channel, parse_node_info,
               save_node_data_to_file, start_lightning, ConnectToNodeRes, LightningConf, LightningContext};

#[cfg(not(target_arch = "wasm32"))]
use super::{lp_coinfind_or_err, MmCoinEnum};

mod ln_errors;
mod ln_rpc;
#[cfg(not(target_arch = "wasm32"))] mod ln_utils;

#[derive(Deserialize)]
pub struct EnableLightningRequest {
    pub coin: String,
    pub port: Option<u16>,
    pub name: String,
    pub color: Option<String>,
}

#[cfg(target_arch = "wasm32")]
pub async fn enable_lightning(_ctx: MmArc, _req: EnableLightningRequest) -> EnableLightningResult<String> {
    MmError::err(EnableLightningError::UnsupportedMode(
        "'enable_lightning'".into(),
        "native".into(),
    ))
}

/// Start a BTC lightning node (LTC should be added later).
#[cfg(not(target_arch = "wasm32"))]
pub async fn enable_lightning(ctx: MmArc, req: EnableLightningRequest) -> EnableLightningResult<String> {
    // coin has to be enabled in electrum to start a lightning node
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;

    let utxo_coin = match coin {
        MmCoinEnum::UtxoCoin(utxo) => utxo,
        _ => {
            return MmError::err(EnableLightningError::UnsupportedCoin(
                req.coin,
                "Only utxo coins are supported in lightning".into(),
            ))
        },
    };

    if !utxo_coin.as_ref().conf.lightning {
        return MmError::err(EnableLightningError::UnsupportedCoin(
            req.coin,
            "'lightning' field not found in coin config".into(),
        ));
    }

    let client = match &utxo_coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Electrum(c) => c,
        UtxoRpcClientEnum::Native(_) => {
            return MmError::err(EnableLightningError::UnsupportedMode(
                "Lightning network".into(),
                "electrum".into(),
            ))
        },
    };

    let network = match &utxo_coin.as_ref().conf.network {
        Some(n) => network_from_string(n.clone())?,
        None => {
            return MmError::err(EnableLightningError::UnsupportedCoin(
                req.coin,
                "'network' field not found in coin config".into(),
            ))
        },
    };

    if req.name.len() > 32 {
        return MmError::err(EnableLightningError::InvalidRequest(
            "Node name length can't be more than 32 characters".into(),
        ));
    }
    let node_name = format!("{}{:width$}", req.name, " ", width = 32 - req.name.len());

    let mut node_color = [0u8; 3];
    hex::decode_to_slice(
        req.color.unwrap_or_else(|| "000000".into()),
        &mut node_color as &mut [u8],
    )
    .map_to_mm(|_| EnableLightningError::InvalidRequest("Invalid Hex Color".into()))?;

    let listen_addr = myipaddr(ctx.clone())
        .await
        .map_to_mm(EnableLightningError::InvalidAddress)?;
    let port = req.port.unwrap_or(9735);

    let conf = LightningConf::new(client.clone(), network, listen_addr, port, node_name, node_color);
    start_lightning(&ctx, utxo_coin, conf).await?;

    Ok("success".into())
}

#[derive(Deserialize)]
pub struct ConnectToNodeRequest {
    pub coin: String,
    pub node_id: String,
    #[serde(default)]
    pub reconnect_on_restart: bool,
}

#[cfg(target_arch = "wasm32")]
pub async fn connect_to_lightning_node(_ctx: MmArc, _req: ConnectToNodeRequest) -> ConnectToNodeResult<String> {
    MmError::err(ConnectToNodeError::UnsupportedMode(
        "'connect_to_lightning_node'".into(),
        "native".into(),
    ))
}

/// Connect to a certain node on the lightning network.
#[cfg(not(target_arch = "wasm32"))]
pub async fn connect_to_lightning_node(ctx: MmArc, req: ConnectToNodeRequest) -> ConnectToNodeResult<String> {
    let lightning_ctx = LightningContext::from_ctx(&ctx).unwrap();

    {
        let background_processor = lightning_ctx.background_processors.lock().await;
        if !background_processor.contains_key(&req.coin) {
            return MmError::err(ConnectToNodeError::LightningNotEnabled(req.coin));
        }
    }

    let (node_pubkey, node_addr) = parse_node_info(req.node_id.clone())?;

    if req.reconnect_on_restart {
        save_node_data_to_file(&nodes_data_path(&ctx), &req.node_id)?
    }

    let peer_managers = lightning_ctx.peer_managers.lock().await;
    let peer_manager = peer_managers
        .get(&req.coin)
        .ok_or(ConnectToNodeError::LightningNotEnabled(req.coin))?;
    let res = connect_to_node(node_pubkey, node_addr, peer_manager.clone()).await?;

    Ok(res.to_string())
}

fn get_true() -> bool { true }

#[derive(Deserialize)]
pub struct OpenChannelRequest {
    pub coin: String,
    pub node_id: String,
    pub amount_in_sat: u64,
    // Helps in tracking which FundingGenerationReady events corresponds to which open_channel call
    pub request_id: u64,
    #[serde(default = "get_true")]
    pub announce_channel: bool,
}

#[cfg(target_arch = "wasm32")]
pub async fn open_channel(_ctx: MmArc, _req: OpenChannelRequest) -> OpenChannelResult<String> {
    MmError::err(OpenChannelError::UnsupportedMode(
        "'open_channel'".into(),
        "native".into(),
    ))
}

/// Opens a channel on the lightning network.
#[cfg(not(target_arch = "wasm32"))]
pub async fn open_channel(ctx: MmArc, req: OpenChannelRequest) -> OpenChannelResult<String> {
    let lightning_ctx = LightningContext::from_ctx(&ctx).unwrap();

    {
        let background_processor = lightning_ctx.background_processors.lock().await;
        if !background_processor.contains_key(&req.coin) {
            return MmError::err(OpenChannelError::LightningNotEnabled(req.coin));
        }
    }

    // Todo: check the cases when you can open a channel with a node with only the pubkey / and how to use save_node_data_to_file with it
    let (node_pubkey, node_addr) = parse_node_info(req.node_id.clone())?;

    let connect_to_node_res = {
        let peer_managers = lightning_ctx.peer_managers.lock().await;
        let peer_manager = peer_managers
            .get(&req.coin)
            .ok_or_else(|| ConnectToNodeError::LightningNotEnabled(req.coin.clone()))?;
        connect_to_node(node_pubkey, node_addr, peer_manager.clone()).await?
    };

    match connect_to_node_res {
        ConnectToNodeRes::ConnectedSuccessfully(_, _) => save_node_data_to_file(&nodes_data_path(&ctx), &req.node_id)?,
        // Todo: for save_node_data_to_file should have a file for each node to check if it was saved before or not
        // for the case of if a node is already connected through "connect_to_lightning_node" RPC with reconnect_on_restart as true
        // this should be better than keeping track of saved nodes for restart in memory
        ConnectToNodeRes::AlreadyConnected(_, _) => (),
    }

    {
        let channel_managers = lightning_ctx.channel_managers.lock().await;
        let channel_manager = channel_managers
            .get(&req.coin)
            .ok_or(ConnectToNodeError::LightningNotEnabled(req.coin))?;
        open_ln_channel(
            node_pubkey,
            req.amount_in_sat,
            req.request_id,
            req.announce_channel,
            channel_manager.clone(),
        )?;
    }

    Ok("success".into())
}

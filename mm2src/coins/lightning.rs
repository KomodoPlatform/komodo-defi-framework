#[cfg(not(target_arch = "wasm32"))]
use crate::utxo::rpc_clients::UtxoRpcClientEnum;
#[cfg(not(target_arch = "wasm32"))]
use common::ip_addr::myipaddr;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use ln_errors::{EnableLightningError, EnableLightningResult};
#[cfg(not(target_arch = "wasm32"))]
use ln_utils::{network_from_string, start_lightning, LightningConf};

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

    let listen_addr = myipaddr(ctx.clone())
        .await
        .map_to_mm(EnableLightningError::InvalidAddress)?;
    let port = req.port.unwrap_or(9735);

    let node_name = format!("{}{:width$}", req.name, " ", width = 32 - req.name.len());

    let conf = LightningConf::new(client.clone(), network, listen_addr, port, node_name);
    start_lightning(&ctx, conf).await?;

    Ok("success".into())
}

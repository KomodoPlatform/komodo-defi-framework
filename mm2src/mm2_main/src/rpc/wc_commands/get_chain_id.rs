use kdf_walletconnect::WalletConnectCtx;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::Serialize;

use super::{EmptyRpcRequst, WalletConnectRpcError};

#[derive(Debug, PartialEq, Serialize)]
pub struct GetChainIdResponse {
    pub chain_id: String,
}

/// `delete connection` RPC command implementation.
pub async fn get_chain_id(ctx: MmArc, _req: EmptyRpcRequst) -> MmResult<GetChainIdResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    let chain_id = ctx.get_active_chain_id().await;

    Ok(GetChainIdResponse { chain_id })
}

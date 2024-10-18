use kdf_walletconnect::session::rpc::ping::send_session_ping_request;
use kdf_walletconnect::WalletConnectCtx;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::{Deserialize, Serialize};

use super::WalletConnectRpcError;

#[derive(Debug, PartialEq, Serialize)]
pub struct SessionPingResponse {
    pub successful: bool,
}

#[derive(Deserialize)]
pub struct SessionPingRequest {
    topic: String,
}

/// `ping session` RPC command implementation.
pub async fn ping_session(ctx: MmArc, req: SessionPingRequest) -> MmResult<SessionPingResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    send_session_ping_request(&ctx, &req.topic.into())
        .await
        .mm_err(|err| WalletConnectRpcError::SessionRequestError(err.to_string()))?;

    Ok(SessionPingResponse { successful: true })
}

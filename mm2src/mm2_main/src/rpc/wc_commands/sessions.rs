use kdf_walletconnect::{session::SessionRpcInfo, WalletConnectCtx};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::Serialize;

use super::{EmptyRpcRequst, EmptyRpcResponse, WalletConnectRpcError};

#[derive(Debug, PartialEq, Serialize)]
pub struct GetSessionsResponse {
    pub sessions: Vec<SessionRpcInfo>,
}

/// `Get all sessions connection` RPC command implementation.
pub async fn get_all_sessions(
    ctx: MmArc,
    _req: EmptyRpcRequst,
) -> MmResult<GetSessionsResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    let sessions = ctx
        .session
        .get_sessions()
        .into_iter()
        .map(SessionRpcInfo::from)
        .collect::<Vec<_>>();

    Ok(GetSessionsResponse { sessions })
}

#[derive(Debug, Serialize)]
pub struct GetSessionResponse {
    pub session: Option<SessionRpcInfo>,
}

#[derive(Deserialize)]
pub struct GetSessionRequest {
    topic: String,
}

/// `Get all sessions connection` RPC command implementation.
pub async fn get_session(ctx: MmArc, req: GetSessionRequest) -> MmResult<GetSessionResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    let session = ctx
        .session
        .get_session(&req.topic.into())
        .map(|s| SessionRpcInfo::from(s.clone()));

    Ok(GetSessionResponse { session })
}

/// `Get all sessions connection` RPC command implementation.
pub async fn disconnect_session(
    ctx: MmArc,
    req: GetSessionRequest,
) -> MmResult<EmptyRpcResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    ctx.session
        .disconnect_session(&req.topic.into(), &ctx.client)
        .await
        .mm_err(|err| WalletConnectRpcError::SessionRequestError(err.to_string()))?;

    Ok(EmptyRpcResponse {})
}

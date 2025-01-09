use kdf_walletconnect::session::rpc::send_session_ping_request;
use kdf_walletconnect::session::SessionRpcInfo;
use kdf_walletconnect::WalletConnectCtx;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::Serialize;

use super::{EmptyRpcRequst, EmptyRpcResponse, WalletConnectRpcError};

#[derive(Debug, PartialEq, Serialize)]
pub struct SessionResponse {
    pub result: String,
}

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
        .session_manager
        .get_sessions()
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

/// `Get session connection` RPC command implementation.
pub async fn get_session(ctx: MmArc, req: GetSessionRequest) -> MmResult<GetSessionResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    let session = ctx
        .session_manager
        .get_session(&req.topic.into())
        .map(SessionRpcInfo::from);

    Ok(GetSessionResponse { session })
}

/// `Delete session connection` RPC command implementation.
pub async fn disconnect_session(
    ctx: MmArc,
    req: GetSessionRequest,
) -> MmResult<EmptyRpcResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    ctx.drop_session(&req.topic.into())
        .await
        .mm_err(|err| WalletConnectRpcError::SessionRequestError(err.to_string()))?;

    Ok(EmptyRpcResponse {})
}

/// `ping session` RPC command implementation.
pub async fn ping_session(ctx: MmArc, req: GetSessionRequest) -> MmResult<SessionResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    send_session_ping_request(&ctx, &req.topic.into())
        .await
        .mm_err(|err| WalletConnectRpcError::SessionRequestError(err.to_string()))?;

    Ok(SessionResponse {
        result: "Ping successful".to_owned(),
    })
}

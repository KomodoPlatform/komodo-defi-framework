use kdf_walletconnect::session::SessionRpcInfo;
use kdf_walletconnect::session::{disconnect_session_rpc, rpc::send_session_ping_request};
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

/// `Get session connection` RPC command implementation.
pub async fn get_session(ctx: MmArc, req: GetSessionRequest) -> MmResult<GetSessionResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    let session = ctx
        .session
        .get_session(&req.topic.into())
        .map(|s| SessionRpcInfo::from(s.clone()));

    Ok(GetSessionResponse { session })
}

/// `Get session connection` RPC command implementation.
pub async fn set_active_session(
    ctx: MmArc,
    req: GetSessionRequest,
) -> MmResult<SessionResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    ctx.session
        .set_active_session(&req.topic.into())
        .await
        .mm_err(|err| WalletConnectRpcError::SessionRequestError(err.to_string()))?;

    Ok(SessionResponse {
        result: "active session updated!".to_owned(),
    })
}

/// `Delete session connection` RPC command implementation.
pub async fn disconnect_session(
    ctx: MmArc,
    req: GetSessionRequest,
) -> MmResult<EmptyRpcResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    disconnect_session_rpc(&ctx, &req.topic.into())
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

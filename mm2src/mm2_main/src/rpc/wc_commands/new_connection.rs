use kdf_walletconnect::WalletConnectCtx;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::Serialize;

use super::{EmptyRpcRequst, WalletConnectRpcError};

#[derive(Debug, PartialEq, Serialize)]
pub struct CreateConnectionResponse {
    pub url: String,
}

/// `new_connection` RPC command implementation.
pub async fn new_connection(
    ctx: MmArc,
    _req: EmptyRpcRequst,
) -> MmResult<CreateConnectionResponse, WalletConnectRpcError> {
    let ctx =
        WalletConnectCtx::from_ctx(&ctx).mm_err(|err| WalletConnectRpcError::InitializationError(err.to_string()))?;
    let url = ctx
        .new_connection(None)
        .await
        .mm_err(|err| WalletConnectRpcError::SessionRequestError(err.to_string()))?;

    Ok(CreateConnectionResponse { url })
}

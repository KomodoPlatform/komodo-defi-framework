use super::lp_protocol::{MmRpcBuilder, MmRpcRequest};
use super::{DispatcherError, DispatcherResult, PUBLIC_METHODS};
use crate::mm2::lp_ordermatch::{start_simple_market_maker_bot, stop_simple_market_maker_bot};
use crate::{mm2::lp_stats::{add_node_to_version_stat, remove_node_from_version_stat, start_version_stat_collection},
            mm2::lp_swap::trade_preimage_rpc,
            mm2::rpc::get_public_key::get_public_key};
use coins::{add_delegation, remove_delegation, withdraw};
use common::log::{error, warn};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::HttpStatusCode;
use futures::Future as Future03;
use http::Response;
use serde::de::DeserializeOwned;
use serde_json::{self as json, Value as Json};
use std::net::SocketAddr;

pub async fn process_single_request(
    ctx: MmArc,
    req: Json,
    client: SocketAddr,
    local_only: bool,
) -> DispatcherResult<Response<Vec<u8>>> {
    let request: MmRpcRequest = json::from_value(req)?;

    // https://github.com/artemii235/SuperNET/issues/368
    let method_name = Some(request.method.as_str());
    if local_only && !client.ip().is_loopback() && !PUBLIC_METHODS.contains(&method_name) {
        return MmError::err(DispatcherError::LocalHostOnly);
    }

    auth(&request, &ctx)?;
    dispatcher(request, ctx).await
}

/// # Examples
///
/// ```rust
/// async fn withdraw(request: WithdrawRequest) -> Result<TransactionDetails, MmError<WithdrawError>>
/// ```
///
/// where
///     `Request` = `WithdrawRequest`,
///     `T` = `TransactionDetails`,
///     `E` = `WithdrawError`
async fn handle_mmrpc<Handler, Fut, Request, T, E>(
    ctx: MmArc,
    request: MmRpcRequest,
    handler: Handler,
) -> DispatcherResult<Response<Vec<u8>>>
where
    Handler: FnOnce(MmArc, Request) -> Fut,
    Fut: Future03<Output = Result<T, MmError<E>>>,
    Request: DeserializeOwned,
    T: serde::Serialize + 'static,
    E: SerMmErrorType + HttpStatusCode + 'static,
{
    let params = json::from_value(request.params)?;
    let result = handler(ctx, params).await;
    if let Err(ref e) = result {
        error!("RPC error response: {}", e);
    }

    let response = MmRpcBuilder::from_result(result)
        .version(request.mmrpc)
        .id(request.id)
        .build();
    Ok(response.serialize_http_response())
}

fn auth(request: &MmRpcRequest, ctx: &MmArc) -> DispatcherResult<()> {
    if PUBLIC_METHODS.contains(&Some(request.method.as_str())) {
        return Ok(());
    }

    let rpc_password = ctx.conf["rpc_password"].as_str().unwrap_or_else(|| {
        warn!("'rpc_password' is not set in the config");
        ""
    });
    match request.userpass {
        Some(ref userpass) if userpass == rpc_password => Ok(()),
        Some(_) => MmError::err(DispatcherError::UserpassIsInvalid),
        None => MmError::err(DispatcherError::UserpassIsNotSet),
    }
}

async fn dispatcher(request: MmRpcRequest, ctx: MmArc) -> DispatcherResult<Response<Vec<u8>>> {
    match request.method.as_str() {
        "add_delegation" => handle_mmrpc(ctx, request, add_delegation).await,
        "add_node_to_version_stat" => handle_mmrpc(ctx, request, add_node_to_version_stat).await,
        "get_public_key" => handle_mmrpc(ctx, request, get_public_key).await,
        "remove_delegation" => handle_mmrpc(ctx, request, remove_delegation).await,
        "remove_node_from_version_stat" => handle_mmrpc(ctx, request, remove_node_from_version_stat).await,
        "start_simple_market_maker_bot" => handle_mmrpc(ctx, request, start_simple_market_maker_bot).await,
        "start_version_stat_collection" => handle_mmrpc(ctx, request, start_version_stat_collection).await,
        "stop_simple_market_maker_bot" => handle_mmrpc(ctx, request, stop_simple_market_maker_bot).await,
        "trade_preimage" => handle_mmrpc(ctx, request, trade_preimage_rpc).await,
        "withdraw" => handle_mmrpc(ctx, request, withdraw).await,
        _ => MmError::err(DispatcherError::NoSuchMethod { method: request.method }),
    }
}

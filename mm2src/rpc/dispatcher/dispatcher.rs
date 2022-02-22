use super::{DispatcherError, DispatcherResult, PUBLIC_METHODS};
use crate::mm2::lp_native_dex::rpc_command::{mm_init_status, mm_init_user_action};
use crate::mm2::lp_ordermatch::{start_simple_market_maker_bot, stop_simple_market_maker_bot};
use crate::mm2::rpc::rate_limiter::{process_rate_limit, RateLimitContext};
use crate::{mm2::lp_stats::{add_node_to_version_stat, remove_node_from_version_stat, start_version_stat_collection,
                            stop_version_stat_collection, update_version_stat_collection},
            mm2::lp_swap::{recreate_swap_data, trade_preimage_rpc},
            mm2::rpc::get_public_key::get_public_key};
use coins::coin_balance::{account_balance, scan_for_new_addresses};
use coins::hd_wallet::get_new_address;
use coins::init_create_account::{init_create_new_account, init_create_new_account_status,
                                 init_create_new_account_user_action};
use coins::init_withdraw::{init_withdraw, withdraw_status, withdraw_user_action};
use coins::lightning::{connect_to_lightning_node, open_channel, LightningCoin};
use coins::my_tx_history_v2::my_tx_history_v2_rpc;
use coins::utxo::bch::BchCoin;
use coins::utxo::qtum::QtumCoin;
use coins::utxo::slp::SlpToken;
use coins::utxo::utxo_standard::UtxoStandardCoin;
use coins::{add_delegation, get_staking_infos, remove_delegation, withdraw};
use coins_activation::{enable_l2, enable_platform_coin_with_tokens, enable_token, init_standalone_coin,
                       init_standalone_coin_status, init_standalone_coin_user_action};
use common::log::{error, warn};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::mm_rpc_protocol::{MmRpcBuilder, MmRpcRequest, MmRpcVersion};
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

    let rate_limit_ctx = RateLimitContext::from_ctx(&ctx).unwrap();
    if rate_limit_ctx.is_banned(client.ip()).await {
        return MmError::err(DispatcherError::Banned);
    }

    auth(&request, &ctx, &client).await?;
    match request.mmrpc {
        MmRpcVersion::V2 => dispatcher_v2(request, ctx).await,
    }
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

async fn auth(request: &MmRpcRequest, ctx: &MmArc, client: &SocketAddr) -> DispatcherResult<()> {
    if PUBLIC_METHODS.contains(&Some(request.method.as_str())) {
        return Ok(());
    }

    let rpc_password = ctx.conf["rpc_password"].as_str().unwrap_or_else(|| {
        warn!("'rpc_password' is not set in the config");
        ""
    });
    match request.userpass {
        Some(ref userpass) if userpass == rpc_password => Ok(()),
        Some(_) => Err(process_rate_limit(ctx, client).await),
        None => MmError::err(DispatcherError::UserpassIsNotSet),
    }
}

async fn dispatcher_v2(request: MmRpcRequest, ctx: MmArc) -> DispatcherResult<Response<Vec<u8>>> {
    match request.method.as_str() {
        // "activate_bch_protocol_coin" => handle_mmrpc(ctx, request, activate_bch_protocol_coin).await,
        "account_balance" => handle_mmrpc(ctx, request, account_balance).await,
        "add_delegation" => handle_mmrpc(ctx, request, add_delegation).await,
        "add_node_to_version_stat" => handle_mmrpc(ctx, request, add_node_to_version_stat).await,
        "connect_to_lightning_node" => handle_mmrpc(ctx, request, connect_to_lightning_node).await,
        "enable_bch_with_tokens" => handle_mmrpc(ctx, request, enable_platform_coin_with_tokens::<BchCoin>).await,
        "enable_lightning" => handle_mmrpc(ctx, request, enable_l2::<LightningCoin>).await,
        "enable_slp" => handle_mmrpc(ctx, request, enable_token::<SlpToken>).await,
        "get_new_address" => handle_mmrpc(ctx, request, get_new_address).await,
        "get_public_key" => handle_mmrpc(ctx, request, get_public_key).await,
        "get_staking_infos" => handle_mmrpc(ctx, request, get_staking_infos).await,
        "init_create_new_account" => handle_mmrpc(ctx, request, init_create_new_account).await,
        "init_create_new_account_status" => handle_mmrpc(ctx, request, init_create_new_account_status).await,
        "init_create_new_account_user_action" => handle_mmrpc(ctx, request, init_create_new_account_user_action).await,
        "init_qtum" => handle_mmrpc(ctx, request, init_standalone_coin::<QtumCoin>).await,
        "init_qtum_status" => handle_mmrpc(ctx, request, init_standalone_coin_status::<QtumCoin>).await,
        "init_qtum_user_action" => handle_mmrpc(ctx, request, init_standalone_coin_user_action::<QtumCoin>).await,
        "init_utxo" => handle_mmrpc(ctx, request, init_standalone_coin::<UtxoStandardCoin>).await,
        "init_utxo_status" => handle_mmrpc(ctx, request, init_standalone_coin_status::<UtxoStandardCoin>).await,
        "init_utxo_user_action" => {
            handle_mmrpc(ctx, request, init_standalone_coin_user_action::<UtxoStandardCoin>).await
        },
        "init_withdraw" => handle_mmrpc(ctx, request, init_withdraw).await,
        "mm_init_status" => handle_mmrpc(ctx, request, mm_init_status).await,
        "mm_init_user_action" => handle_mmrpc(ctx, request, mm_init_user_action).await,
        "my_tx_history" => handle_mmrpc(ctx, request, my_tx_history_v2_rpc).await,
        "open_channel" => handle_mmrpc(ctx, request, open_channel).await,
        "recreate_swap_data" => handle_mmrpc(ctx, request, recreate_swap_data).await,
        "remove_delegation" => handle_mmrpc(ctx, request, remove_delegation).await,
        "remove_node_from_version_stat" => handle_mmrpc(ctx, request, remove_node_from_version_stat).await,
        "scan_for_new_addresses" => handle_mmrpc(ctx, request, scan_for_new_addresses).await,
        "start_simple_market_maker_bot" => handle_mmrpc(ctx, request, start_simple_market_maker_bot).await,
        "start_version_stat_collection" => handle_mmrpc(ctx, request, start_version_stat_collection).await,
        "stop_simple_market_maker_bot" => handle_mmrpc(ctx, request, stop_simple_market_maker_bot).await,
        "stop_version_stat_collection" => handle_mmrpc(ctx, request, stop_version_stat_collection).await,
        "update_version_stat_collection" => handle_mmrpc(ctx, request, update_version_stat_collection).await,
        "trade_preimage" => handle_mmrpc(ctx, request, trade_preimage_rpc).await,
        "withdraw" => handle_mmrpc(ctx, request, withdraw).await,
        "withdraw_status" => handle_mmrpc(ctx, request, withdraw_status).await,
        "withdraw_user_action" => handle_mmrpc(ctx, request, withdraw_user_action).await,
        _ => MmError::err(DispatcherError::NoSuchMethod),
    }
}

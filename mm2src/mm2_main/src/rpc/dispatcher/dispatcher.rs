use super::{DispatcherError, DispatcherResult, PUBLIC_METHODS};
use crate::mm2::lp_native_dex::init_hw::{init_trezor, init_trezor_status, init_trezor_user_action};
use crate::mm2::lp_ordermatch::{best_orders_rpc_v2, orderbook_rpc_v2, start_simple_market_maker_bot,
                                stop_simple_market_maker_bot};
use crate::mm2::rpc::rate_limiter::{process_rate_limit, RateLimitContext};
use crate::{mm2::lp_stats::{add_node_to_version_stat, remove_node_from_version_stat, start_version_stat_collection,
                            stop_version_stat_collection, update_version_stat_collection},
            mm2::lp_swap::{recreate_swap_data, trade_preimage_rpc},
            mm2::rpc::lp_commands::{get_public_key, get_public_key_hash}};
use coins::eth::EthCoin;
use coins::hd_wallet::get_new_address;
use coins::my_tx_history_v2::my_tx_history_v2_rpc;
use coins::rpc_command::account_balance::account_balance;
use coins::rpc_command::get_current_mtp::get_current_mtp_rpc;
use coins::rpc_command::init_create_account::{init_create_new_account, init_create_new_account_status,
                                              init_create_new_account_user_action};
use coins::rpc_command::init_scan_for_new_addresses::{init_scan_for_new_addresses, init_scan_for_new_addresses_status};
use coins::rpc_command::init_withdraw::{init_withdraw, withdraw_status, withdraw_user_action};
use coins::utxo::bch::BchCoin;
use coins::utxo::qtum::QtumCoin;
use coins::utxo::slp::SlpToken;
use coins::utxo::utxo_standard::UtxoStandardCoin;
use coins::{add_delegation, get_raw_transaction, get_staking_infos, remove_delegation, sign_message, verify_message,
            withdraw};
use coins_activation::{enable_l2, enable_platform_coin_with_tokens, enable_token, init_standalone_coin,
                       init_standalone_coin_status, init_standalone_coin_user_action};
use common::log::{error, warn};
use common::HttpStatusCode;
use futures::Future as Future03;
use http::Response;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_rpc::mm_protocol::{MmRpcBuilder, MmRpcRequest, MmRpcVersion};
use serde::de::DeserializeOwned;
use serde_json::{self as json, Value as Json};
use std::net::SocketAddr;

cfg_native! {
    use coins::lightning::{close_channel, connect_to_lightning_node, generate_invoice, get_channel_details,
        get_claimable_balances, get_payment_details, list_closed_channels_by_filter, list_open_channels_by_filter, list_payments_by_filter, open_channel,
        send_payment, LightningCoin};
    use coins::{SolanaCoin, SplToken};
    use coins::z_coin::ZCoin;
}

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
        "account_balance" => handle_mmrpc(ctx, request, account_balance).await,
        "add_delegation" => handle_mmrpc(ctx, request, add_delegation).await,
        "add_node_to_version_stat" => handle_mmrpc(ctx, request, add_node_to_version_stat).await,
        "best_orders" => handle_mmrpc(ctx, request, best_orders_rpc_v2).await,
        "enable_bch_with_tokens" => handle_mmrpc(ctx, request, enable_platform_coin_with_tokens::<BchCoin>).await,
        "enable_slp" => handle_mmrpc(ctx, request, enable_token::<SlpToken>).await,
        "enable_eth_with_tokens" => handle_mmrpc(ctx, request, enable_platform_coin_with_tokens::<EthCoin>).await,
        "enable_erc20" => handle_mmrpc(ctx, request, enable_token::<EthCoin>).await,
        "get_current_mtp" => handle_mmrpc(ctx, request, get_current_mtp_rpc).await,
        "get_new_address" => handle_mmrpc(ctx, request, get_new_address).await,
        "get_public_key" => handle_mmrpc(ctx, request, get_public_key).await,
        "get_public_key_hash" => handle_mmrpc(ctx, request, get_public_key_hash).await,
        "get_raw_transaction" => handle_mmrpc(ctx, request, get_raw_transaction).await,
        "get_staking_infos" => handle_mmrpc(ctx, request, get_staking_infos).await,
        "init_create_new_account" => handle_mmrpc(ctx, request, init_create_new_account).await,
        "init_create_new_account_status" => handle_mmrpc(ctx, request, init_create_new_account_status).await,
        "init_create_new_account_user_action" => handle_mmrpc(ctx, request, init_create_new_account_user_action).await,
        "init_qtum" => handle_mmrpc(ctx, request, init_standalone_coin::<QtumCoin>).await,
        "init_qtum_status" => handle_mmrpc(ctx, request, init_standalone_coin_status::<QtumCoin>).await,
        "init_qtum_user_action" => handle_mmrpc(ctx, request, init_standalone_coin_user_action::<QtumCoin>).await,
        "init_scan_for_new_addresses" => handle_mmrpc(ctx, request, init_scan_for_new_addresses).await,
        "init_scan_for_new_addresses_status" => handle_mmrpc(ctx, request, init_scan_for_new_addresses_status).await,
        "init_trezor" => handle_mmrpc(ctx, request, init_trezor).await,
        "init_trezor_status" => handle_mmrpc(ctx, request, init_trezor_status).await,
        "init_trezor_user_action" => handle_mmrpc(ctx, request, init_trezor_user_action).await,
        "init_utxo" => handle_mmrpc(ctx, request, init_standalone_coin::<UtxoStandardCoin>).await,
        "init_utxo_status" => handle_mmrpc(ctx, request, init_standalone_coin_status::<UtxoStandardCoin>).await,
        "init_utxo_user_action" => {
            handle_mmrpc(ctx, request, init_standalone_coin_user_action::<UtxoStandardCoin>).await
        },
        "init_withdraw" => handle_mmrpc(ctx, request, init_withdraw).await,
        "my_tx_history" => handle_mmrpc(ctx, request, my_tx_history_v2_rpc).await,
        "orderbook" => handle_mmrpc(ctx, request, orderbook_rpc_v2).await,
        "recreate_swap_data" => handle_mmrpc(ctx, request, recreate_swap_data).await,
        "remove_delegation" => handle_mmrpc(ctx, request, remove_delegation).await,
        "remove_node_from_version_stat" => handle_mmrpc(ctx, request, remove_node_from_version_stat).await,
        "sign_message" => handle_mmrpc(ctx, request, sign_message).await,
        "start_simple_market_maker_bot" => handle_mmrpc(ctx, request, start_simple_market_maker_bot).await,
        "start_version_stat_collection" => handle_mmrpc(ctx, request, start_version_stat_collection).await,
        "stop_simple_market_maker_bot" => handle_mmrpc(ctx, request, stop_simple_market_maker_bot).await,
        "stop_version_stat_collection" => handle_mmrpc(ctx, request, stop_version_stat_collection).await,
        "trade_preimage" => handle_mmrpc(ctx, request, trade_preimage_rpc).await,
        "update_version_stat_collection" => handle_mmrpc(ctx, request, update_version_stat_collection).await,
        "verify_message" => handle_mmrpc(ctx, request, verify_message).await,
        "withdraw" => handle_mmrpc(ctx, request, withdraw).await,
        "withdraw_status" => handle_mmrpc(ctx, request, withdraw_status).await,
        "withdraw_user_action" => handle_mmrpc(ctx, request, withdraw_user_action).await,
        #[cfg(not(target_arch = "wasm32"))]
        native_only_methods => match native_only_methods {
            "close_channel" => handle_mmrpc(ctx, request, close_channel).await,
            "connect_to_lightning_node" => handle_mmrpc(ctx, request, connect_to_lightning_node).await,
            "enable_lightning" => handle_mmrpc(ctx, request, enable_l2::<LightningCoin>).await,
            "generate_invoice" => handle_mmrpc(ctx, request, generate_invoice).await,
            "get_channel_details" => handle_mmrpc(ctx, request, get_channel_details).await,
            "get_claimable_balances" => handle_mmrpc(ctx, request, get_claimable_balances).await,
            "get_payment_details" => handle_mmrpc(ctx, request, get_payment_details).await,
            "init_z_coin" => handle_mmrpc(ctx, request, init_standalone_coin::<ZCoin>).await,
            "init_z_coin_status" => handle_mmrpc(ctx, request, init_standalone_coin_status::<ZCoin>).await,
            "init_z_coin_user_action" => handle_mmrpc(ctx, request, init_standalone_coin_user_action::<ZCoin>).await,
            "list_closed_channels_by_filter" => handle_mmrpc(ctx, request, list_closed_channels_by_filter).await,
            "list_open_channels_by_filter" => handle_mmrpc(ctx, request, list_open_channels_by_filter).await,
            "list_payments_by_filter" => handle_mmrpc(ctx, request, list_payments_by_filter).await,
            "open_channel" => handle_mmrpc(ctx, request, open_channel).await,
            "send_payment" => handle_mmrpc(ctx, request, send_payment).await,
            "enable_solana_with_tokens" => {
                handle_mmrpc(ctx, request, enable_platform_coin_with_tokens::<SolanaCoin>).await
            },
            "enable_spl" => handle_mmrpc(ctx, request, enable_token::<SplToken>).await,
            "z_coin_tx_history" => handle_mmrpc(ctx, request, coins::my_tx_history_v2::z_coin_tx_history_rpc).await,
            _ => MmError::err(DispatcherError::NoSuchMethod),
        },
        #[cfg(target_arch = "wasm32")]
        _ => MmError::err(DispatcherError::NoSuchMethod),
    }
}

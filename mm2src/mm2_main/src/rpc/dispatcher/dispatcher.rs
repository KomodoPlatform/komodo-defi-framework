use super::{DispatcherError, DispatcherResult, PUBLIC_METHODS};
use crate::mm2::lp_native_dex::init_hw::{cancel_init_trezor, init_trezor, init_trezor_status, init_trezor_user_action};
#[cfg(target_arch = "wasm32")]
use crate::mm2::lp_native_dex::init_metamask::{cancel_connect_metamask, connect_metamask, connect_metamask_status};
use crate::mm2::lp_ordermatch::{best_orders_rpc_v2, orderbook_rpc_v2, start_simple_market_maker_bot,
                                stop_simple_market_maker_bot};
use crate::mm2::rpc::rate_limiter::{process_rate_limit, RateLimitContext};
use crate::{mm2::lp_stats::{add_node_to_version_stat, remove_node_from_version_stat, start_version_stat_collection,
                            stop_version_stat_collection, update_version_stat_collection},
            mm2::lp_swap::{get_locked_amount_rpc, max_maker_vol, recreate_swap_data, trade_preimage_rpc},
            mm2::rpc::lp_commands::{get_public_key, get_public_key_hash, get_shared_db_id, trezor_connection_status}};
use coins::eth::EthCoin;
use coins::my_tx_history_v2::my_tx_history_v2_rpc;
use coins::rpc_command::tendermint::{ibc_chains, ibc_transfer_channels, ibc_withdraw};
use coins::rpc_command::{account_balance::account_balance,
                         get_current_mtp::get_current_mtp_rpc,
                         get_enabled_coins::get_enabled_coins,
                         get_new_address::{cancel_get_new_address, get_new_address, init_get_new_address,
                                           init_get_new_address_status, init_get_new_address_user_action},
                         init_account_balance::{cancel_account_balance, init_account_balance,
                                                init_account_balance_status},
                         init_create_account::{cancel_create_new_account, init_create_new_account,
                                               init_create_new_account_status, init_create_new_account_user_action},
                         init_scan_for_new_addresses::{cancel_scan_for_new_addresses, init_scan_for_new_addresses,
                                                       init_scan_for_new_addresses_status},
                         init_withdraw::{cancel_withdraw, init_withdraw, withdraw_status, withdraw_user_action}};
use coins::tendermint::{TendermintCoin, TendermintToken};
use coins::utxo::bch::BchCoin;
use coins::utxo::qtum::QtumCoin;
use coins::utxo::slp::SlpToken;
use coins::utxo::utxo_standard::UtxoStandardCoin;
use coins::{add_delegation, get_my_address, get_raw_transaction, get_staking_infos, nft, remove_delegation,
            sign_message, sign_raw_transaction, verify_message, withdraw};
#[cfg(all(
    feature = "enable-solana",
    not(target_os = "ios"),
    not(target_os = "android"),
    not(target_arch = "wasm32")
))]
use coins::{SolanaCoin, SplToken};
use coins_activation::{cancel_init_l2, cancel_init_standalone_coin, enable_platform_coin_with_tokens, enable_token,
                       init_l2, init_l2_status, init_l2_user_action, init_standalone_coin,
                       init_standalone_coin_status, init_standalone_coin_user_action};
use common::log::{error, warn};
use common::HttpStatusCode;
use futures::Future as Future03;
use http::Response;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_rpc::mm_protocol::{MmRpcBuilder, MmRpcRequest, MmRpcVersion};
use nft::{get_nft_list, get_nft_metadata, get_nft_transfers, refresh_nft_metadata, update_nft, withdraw_nft};
use serde::de::DeserializeOwned;
use serde_json::{self as json, Value as Json};
use std::net::SocketAddr;

cfg_native! {
    use coins::lightning::LightningCoin;
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
    if let Some(task_method) = request.method.strip_prefix("task::") {
        let task_method = task_method.to_string();
        return rpc_task_dispatcher(request, ctx, task_method).await;
    }
    if let Some(gui_storage_method) = request.method.strip_prefix("gui_storage::") {
        let gui_storage_method = gui_storage_method.to_owned();
        return gui_storage_dispatcher(request, ctx, &gui_storage_method).await;
    }

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(lightning_method) = request.method.strip_prefix("lightning::") {
        let lightning_method = lightning_method.to_owned();
        return lightning_dispatcher(request, ctx, &lightning_method).await;
    }

    match request.method.as_str() {
        "account_balance" => handle_mmrpc(ctx, request, account_balance).await,
        "add_delegation" => handle_mmrpc(ctx, request, add_delegation).await,
        "add_node_to_version_stat" => handle_mmrpc(ctx, request, add_node_to_version_stat).await,
        "best_orders" => handle_mmrpc(ctx, request, best_orders_rpc_v2).await,
        "enable_bch_with_tokens" => handle_mmrpc(ctx, request, enable_platform_coin_with_tokens::<BchCoin>).await,
        "enable_slp" => handle_mmrpc(ctx, request, enable_token::<SlpToken>).await,
        "enable_eth_with_tokens" => handle_mmrpc(ctx, request, enable_platform_coin_with_tokens::<EthCoin>).await,
        "enable_erc20" => handle_mmrpc(ctx, request, enable_token::<EthCoin>).await,
        "enable_tendermint_with_assets" => {
            handle_mmrpc(ctx, request, enable_platform_coin_with_tokens::<TendermintCoin>).await
        },
        "enable_tendermint_token" => handle_mmrpc(ctx, request, enable_token::<TendermintToken>).await,
        "get_current_mtp" => handle_mmrpc(ctx, request, get_current_mtp_rpc).await,
        "get_enabled_coins" => handle_mmrpc(ctx, request, get_enabled_coins).await,
        "get_locked_amount" => handle_mmrpc(ctx, request, get_locked_amount_rpc).await,
        "get_my_address" => handle_mmrpc(ctx, request, get_my_address).await,
        "get_new_address" => handle_mmrpc(ctx, request, get_new_address).await,
        "get_nft_list" => handle_mmrpc(ctx, request, get_nft_list).await,
        "get_nft_metadata" => handle_mmrpc(ctx, request, get_nft_metadata).await,
        "get_nft_transfers" => handle_mmrpc(ctx, request, get_nft_transfers).await,
        "get_public_key" => handle_mmrpc(ctx, request, get_public_key).await,
        "get_public_key_hash" => handle_mmrpc(ctx, request, get_public_key_hash).await,
        "get_raw_transaction" => handle_mmrpc(ctx, request, get_raw_transaction).await,
        "get_shared_db_id" => handle_mmrpc(ctx, request, get_shared_db_id).await,
        "get_staking_infos" => handle_mmrpc(ctx, request, get_staking_infos).await,
        "max_maker_vol" => handle_mmrpc(ctx, request, max_maker_vol).await,
        "my_tx_history" => handle_mmrpc(ctx, request, my_tx_history_v2_rpc).await,
        "orderbook" => handle_mmrpc(ctx, request, orderbook_rpc_v2).await,
        "recreate_swap_data" => handle_mmrpc(ctx, request, recreate_swap_data).await,
        "refresh_nft_metadata" => handle_mmrpc(ctx, request, refresh_nft_metadata).await,
        "remove_delegation" => handle_mmrpc(ctx, request, remove_delegation).await,
        "remove_node_from_version_stat" => handle_mmrpc(ctx, request, remove_node_from_version_stat).await,
        "sign_message" => handle_mmrpc(ctx, request, sign_message).await,
        "sign_raw_transaction" => handle_mmrpc(ctx, request, sign_raw_transaction).await,
        "start_simple_market_maker_bot" => handle_mmrpc(ctx, request, start_simple_market_maker_bot).await,
        "start_version_stat_collection" => handle_mmrpc(ctx, request, start_version_stat_collection).await,
        "stop_simple_market_maker_bot" => handle_mmrpc(ctx, request, stop_simple_market_maker_bot).await,
        "stop_version_stat_collection" => handle_mmrpc(ctx, request, stop_version_stat_collection).await,
        "trade_preimage" => handle_mmrpc(ctx, request, trade_preimage_rpc).await,
        "trezor_connection_status" => handle_mmrpc(ctx, request, trezor_connection_status).await,
        "update_nft" => handle_mmrpc(ctx, request, update_nft).await,
        "update_version_stat_collection" => handle_mmrpc(ctx, request, update_version_stat_collection).await,
        "verify_message" => handle_mmrpc(ctx, request, verify_message).await,
        "withdraw" => handle_mmrpc(ctx, request, withdraw).await,
        "ibc_withdraw" => handle_mmrpc(ctx, request, ibc_withdraw).await,
        "ibc_chains" => handle_mmrpc(ctx, request, ibc_chains).await,
        "ibc_transfer_channels" => handle_mmrpc(ctx, request, ibc_transfer_channels).await,
        "withdraw_nft" => handle_mmrpc(ctx, request, withdraw_nft).await,
        #[cfg(not(target_arch = "wasm32"))]
        native_only_methods => match native_only_methods {
            #[cfg(all(feature = "enable-solana", not(target_os = "ios"), not(target_os = "android")))]
            "enable_solana_with_tokens" => {
                handle_mmrpc(ctx, request, enable_platform_coin_with_tokens::<SolanaCoin>).await
            },
            #[cfg(all(feature = "enable-solana", not(target_os = "ios"), not(target_os = "android")))]
            "enable_spl" => handle_mmrpc(ctx, request, enable_token::<SplToken>).await,
            "z_coin_tx_history" => handle_mmrpc(ctx, request, coins::my_tx_history_v2::z_coin_tx_history_rpc).await,
            _ => MmError::err(DispatcherError::NoSuchMethod),
        },
        #[cfg(target_arch = "wasm32")]
        _ => MmError::err(DispatcherError::NoSuchMethod),
    }
}

/// `task` dispatcher.
/// The full path is expected to be `task::method::action`.
/// For example, `task::withdraw::init`, `task::create_new_account::init` etc.
///
/// # Note
///
/// `task_method` is a method name with the `task::` prefix removed.
async fn rpc_task_dispatcher(
    request: MmRpcRequest,
    ctx: MmArc,
    task_method: String,
) -> DispatcherResult<Response<Vec<u8>>> {
    match task_method.as_str() {
        "account_balance::cancel" => handle_mmrpc(ctx, request, cancel_account_balance).await,
        "account_balance::init" => handle_mmrpc(ctx, request, init_account_balance).await,
        "account_balance::status" => handle_mmrpc(ctx, request, init_account_balance_status).await,
        "create_new_account::cancel" => handle_mmrpc(ctx, request, cancel_create_new_account).await,
        "create_new_account::init" => handle_mmrpc(ctx, request, init_create_new_account).await,
        "create_new_account::status" => handle_mmrpc(ctx, request, init_create_new_account_status).await,
        "create_new_account::user_action" => handle_mmrpc(ctx, request, init_create_new_account_user_action).await,
        "enable_qtum::cancel" => handle_mmrpc(ctx, request, cancel_init_standalone_coin::<QtumCoin>).await,
        "enable_qtum::init" => handle_mmrpc(ctx, request, init_standalone_coin::<QtumCoin>).await,
        "enable_qtum::status" => handle_mmrpc(ctx, request, init_standalone_coin_status::<QtumCoin>).await,
        "enable_qtum::user_action" => handle_mmrpc(ctx, request, init_standalone_coin_user_action::<QtumCoin>).await,
        "enable_utxo::cancel" => handle_mmrpc(ctx, request, cancel_init_standalone_coin::<UtxoStandardCoin>).await,
        "enable_utxo::init" => handle_mmrpc(ctx, request, init_standalone_coin::<UtxoStandardCoin>).await,
        "enable_utxo::status" => handle_mmrpc(ctx, request, init_standalone_coin_status::<UtxoStandardCoin>).await,
        "enable_utxo::user_action" => {
            handle_mmrpc(ctx, request, init_standalone_coin_user_action::<UtxoStandardCoin>).await
        },
        "get_new_address::cancel" => handle_mmrpc(ctx, request, cancel_get_new_address).await,
        "get_new_address::init" => handle_mmrpc(ctx, request, init_get_new_address).await,
        "get_new_address::status" => handle_mmrpc(ctx, request, init_get_new_address_status).await,
        "get_new_address::user_action" => handle_mmrpc(ctx, request, init_get_new_address_user_action).await,
        "scan_for_new_addresses::cancel" => handle_mmrpc(ctx, request, cancel_scan_for_new_addresses).await,
        "scan_for_new_addresses::init" => handle_mmrpc(ctx, request, init_scan_for_new_addresses).await,
        "scan_for_new_addresses::status" => handle_mmrpc(ctx, request, init_scan_for_new_addresses_status).await,
        "init_trezor::cancel" => handle_mmrpc(ctx, request, cancel_init_trezor).await,
        "init_trezor::init" => handle_mmrpc(ctx, request, init_trezor).await,
        "init_trezor::status" => handle_mmrpc(ctx, request, init_trezor_status).await,
        "init_trezor::user_action" => handle_mmrpc(ctx, request, init_trezor_user_action).await,
        "withdraw::cancel" => handle_mmrpc(ctx, request, cancel_withdraw).await,
        "withdraw::init" => handle_mmrpc(ctx, request, init_withdraw).await,
        "withdraw::status" => handle_mmrpc(ctx, request, withdraw_status).await,
        "withdraw::user_action" => handle_mmrpc(ctx, request, withdraw_user_action).await,
        #[cfg(not(target_arch = "wasm32"))]
        native_only_methods => match native_only_methods {
            "enable_lightning::cancel" => handle_mmrpc(ctx, request, cancel_init_l2::<LightningCoin>).await,
            "enable_lightning::init" => handle_mmrpc(ctx, request, init_l2::<LightningCoin>).await,
            "enable_lightning::status" => handle_mmrpc(ctx, request, init_l2_status::<LightningCoin>).await,
            "enable_lightning::user_action" => handle_mmrpc(ctx, request, init_l2_user_action::<LightningCoin>).await,
            "enable_z_coin::cancel" => handle_mmrpc(ctx, request, cancel_init_standalone_coin::<ZCoin>).await,
            "enable_z_coin::init" => handle_mmrpc(ctx, request, init_standalone_coin::<ZCoin>).await,
            "enable_z_coin::status" => handle_mmrpc(ctx, request, init_standalone_coin_status::<ZCoin>).await,
            "enable_z_coin::user_action" => handle_mmrpc(ctx, request, init_standalone_coin_user_action::<ZCoin>).await,
            _ => MmError::err(DispatcherError::NoSuchMethod),
        },
        #[cfg(target_arch = "wasm32")]
        wasm_only_methods => match wasm_only_methods {
            "connect_metamask::cancel" => handle_mmrpc(ctx, request, cancel_connect_metamask).await,
            "connect_metamask::init" => handle_mmrpc(ctx, request, connect_metamask).await,
            "connect_metamask::status" => handle_mmrpc(ctx, request, connect_metamask_status).await,
            _ => MmError::err(DispatcherError::NoSuchMethod),
        },
    }
}

/// `gui_storage` dispatcher.
///
/// # Note
///
/// `gui_storage_method` is a method name with the `gui_storage::` prefix removed.
async fn gui_storage_dispatcher(
    request: MmRpcRequest,
    ctx: MmArc,
    gui_storage_method: &str,
) -> DispatcherResult<Response<Vec<u8>>> {
    use mm2_gui_storage::rpc_commands as gui_storage_rpc;

    match gui_storage_method {
        "activate_coins" => handle_mmrpc(ctx, request, gui_storage_rpc::activate_coins).await,
        "add_account" => handle_mmrpc(ctx, request, gui_storage_rpc::add_account).await,
        "deactivate_coins" => handle_mmrpc(ctx, request, gui_storage_rpc::deactivate_coins).await,
        "delete_account" => handle_mmrpc(ctx, request, gui_storage_rpc::delete_account).await,
        "enable_account" => handle_mmrpc(ctx, request, gui_storage_rpc::enable_account).await,
        "get_accounts" => handle_mmrpc(ctx, request, gui_storage_rpc::get_accounts).await,
        "get_account_coins" => handle_mmrpc(ctx, request, gui_storage_rpc::get_account_coins).await,
        "get_enabled_account" => handle_mmrpc(ctx, request, gui_storage_rpc::get_enabled_account).await,
        "set_account_balance" => handle_mmrpc(ctx, request, gui_storage_rpc::set_account_balance).await,
        "set_account_description" => handle_mmrpc(ctx, request, gui_storage_rpc::set_account_description).await,
        "set_account_name" => handle_mmrpc(ctx, request, gui_storage_rpc::set_account_name).await,
        _ => MmError::err(DispatcherError::NoSuchMethod),
    }
}

/// `lightning` dispatcher.
///
/// # Note
///
/// `lightning_method` is a method name with the `lightning::` prefix removed.
#[cfg(not(target_arch = "wasm32"))]
async fn lightning_dispatcher(
    request: MmRpcRequest,
    ctx: MmArc,
    lightning_method: &str,
) -> DispatcherResult<Response<Vec<u8>>> {
    use coins::rpc_command::lightning::{channels, nodes, payments};

    match lightning_method {
        "channels::close_channel" => handle_mmrpc(ctx, request, channels::close_channel).await,
        "channels::get_channel_details" => handle_mmrpc(ctx, request, channels::get_channel_details).await,
        "channels::get_claimable_balances" => handle_mmrpc(ctx, request, channels::get_claimable_balances).await,
        "channels::list_closed_channels_by_filter" => {
            handle_mmrpc(ctx, request, channels::list_closed_channels_by_filter).await
        },
        "channels::list_open_channels_by_filter" => {
            handle_mmrpc(ctx, request, channels::list_open_channels_by_filter).await
        },
        "channels::open_channel" => handle_mmrpc(ctx, request, channels::open_channel).await,
        "channels::update_channel" => handle_mmrpc(ctx, request, channels::update_channel).await,
        "nodes::add_trusted_node" => handle_mmrpc(ctx, request, nodes::add_trusted_node).await,
        "nodes::connect_to_node" => handle_mmrpc(ctx, request, nodes::connect_to_node).await,
        "nodes::list_trusted_nodes" => handle_mmrpc(ctx, request, nodes::list_trusted_nodes).await,
        "nodes::remove_trusted_node" => handle_mmrpc(ctx, request, nodes::remove_trusted_node).await,
        "payments::generate_invoice" => handle_mmrpc(ctx, request, payments::generate_invoice).await,
        "payments::get_payment_details" => handle_mmrpc(ctx, request, payments::get_payment_details).await,
        "payments::list_payments_by_filter" => handle_mmrpc(ctx, request, payments::list_payments_by_filter).await,
        "payments::send_payment" => handle_mmrpc(ctx, request, payments::send_payment).await,
        _ => MmError::err(DispatcherError::NoSuchMethod),
    }
}

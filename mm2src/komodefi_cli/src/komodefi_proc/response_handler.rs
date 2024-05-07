#[path = "response_handler/activation.rs"] mod activation;
#[path = "response_handler/best_orders.rs"] mod best_orders;
#[path = "response_handler/formatters.rs"] mod formatters;
#[path = "response_handler/macros.rs"] mod macros;
#[path = "response_handler/message_signing.rs"]
mod message_signing;
#[path = "response_handler/my_orders.rs"] mod my_orders;
#[path = "response_handler/network.rs"] mod network;
#[path = "response_handler/order_status.rs"] mod order_status;
#[path = "response_handler/orderbook.rs"] mod orderbook;
#[path = "response_handler/orderbook_depth.rs"]
mod orderbook_depth;
#[path = "response_handler/orders_history.rs"]
mod orders_history;
#[path = "response_handler/smart_fraction_fmt.rs"]
mod smart_fraction_fmt;
#[path = "response_handler/swaps.rs"] mod swaps;
#[path = "response_handler/trading.rs"] mod trading;
#[path = "response_handler/utility.rs"] mod utility;
#[path = "response_handler/version_stat.rs"] mod version_stat;
#[path = "response_handler/wallet.rs"] mod wallet;

pub(crate) use orderbook::OrderbookSettings;
pub(crate) use orders_history::OrdersHistorySettings;
pub(crate) use smart_fraction_fmt::SmartFractPrecision;

use anyhow::{anyhow, Result};
use itertools::Itertools;
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use std::cell::RefCell;
use std::io::Write;
use std::ops::DerefMut;

use common::log::error;
use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};
use mm2_rpc::data::legacy::{BalanceResponse, CancelAllOrdersResponse, CoinInitResponse, GetEnabledResponse,
                            MakerOrderForRpc, MinTradingVolResponse, Mm2RpcResult, MmVersionResponse,
                            MyOrdersResponse, OrderStatusResponse, OrderbookResponse, OrdersHistoryResponse,
                            PairWithDepth, SellBuyResponse, Status};
use mm2_rpc::data::version2::{BestOrdersV2Response, GetPublicKeyHashResponse, GetPublicKeyResponse,
                              GetRawTransactionResponse};

use crate::config::KomodefiConfig;
use crate::komodefi_proc::response_handler::formatters::{writeln_field, ZERO_INDENT};
use crate::logging::error_anyhow;
use crate::rpc_data::activation::{InitRpcTaskResponse, TaskId};
use crate::rpc_data::bch::{BchWithTokensActivationResult, SlpInitResult};
use crate::rpc_data::eth::{Erc20InitResult, EthWithTokensActivationResult};
use crate::rpc_data::message_signing::{SignatureError, SignatureResponse, VerificationError, VerificationResponse};
use crate::rpc_data::tendermint::{TendermintActivationResult, TendermintTokenInitResult};
use crate::rpc_data::utility::{GetCurrentMtpError, GetCurrentMtpResponse};
use crate::rpc_data::version_stat::NodeVersionError;
use crate::rpc_data::wallet::{ConvertAddressResponse, ConvertUtxoAddressResponse, KmdRewardsInfoResponse,
                              MyTxHistoryDetails, MyTxHistoryResponse, MyTxHistoryResponseV2, ShowPrivateKeyResponse,
                              ValidateAddressResponse, ZcoinTxDetails};
use crate::rpc_data::zcoin::ZCoinStatus;
use crate::rpc_data::{ActiveSwapsResponse, CancelRpcTaskError, CoinsToKickstartResponse, DisableCoinResponse,
                      GetGossipMeshResponse, GetGossipPeerTopicsResponse, GetGossipTopicPeersResponse,
                      GetMyPeerIdResponse, GetPeersInfoResponse, GetRelayMeshResponse, ListBannedPubkeysResponse,
                      MaxTakerVolResponse, MmRpcErrorV2, MyRecentSwapResponse, MySwapStatusResponse,
                      RecoverFundsOfSwapResponse, SendRawTransactionResponse, SetRequiredConfResponse,
                      SetRequiredNotaResponse, TradePreimageResponse, UnbanPubkeysResponse, WithdrawResponse};

pub(crate) trait ResponseHandler {
    fn print_response(&self, response: Json) -> Result<()>;

    fn on_orderbook_response<Cfg: KomodefiConfig + 'static>(
        &self,
        response: OrderbookResponse,
        config: &Cfg,
        settings: OrderbookSettings,
    ) -> Result<()>;
    fn on_get_enabled_response(&self, response: Mm2RpcResult<GetEnabledResponse>) -> Result<()>;
    fn on_version_response(&self, response: MmVersionResponse) -> Result<()>;
    fn on_enable_response(&self, response: CoinInitResponse) -> Result<()>;
    fn on_enable_bch(&self, response: BchWithTokensActivationResult) -> Result<()>;
    fn on_enable_slp(&self, response: SlpInitResult) -> Result<()>;
    fn on_enable_tendermint(&self, response: TendermintActivationResult) -> Result<()>;
    fn on_enable_tendermint_token(&self, response: TendermintTokenInitResult) -> Result<()>;
    fn on_enable_erc20(&self, response: Erc20InitResult) -> Result<()>;
    fn on_enable_eth_with_tokens(&self, response: EthWithTokensActivationResult) -> Result<()>;
    fn on_enable_z_coin(&self, response: InitRpcTaskResponse) -> TaskId;
    fn on_zcoin_status(&self, respone: ZCoinStatus) -> Result<bool>;
    fn on_disable_coin(&self, response: DisableCoinResponse) -> Result<()>;
    fn on_balance_response(&self, response: BalanceResponse) -> Result<()>;
    fn on_sell_response(&self, response: Mm2RpcResult<SellBuyResponse>) -> Result<()>;
    fn on_buy_response(&self, response: Mm2RpcResult<SellBuyResponse>) -> Result<()>;
    fn on_stop_response(&self, response: Mm2RpcResult<Status>) -> Result<()>;
    fn on_cancel_order_response(&self, response: Mm2RpcResult<Status>) -> Result<()>;
    fn on_cancel_all_response(&self, response: Mm2RpcResult<CancelAllOrdersResponse>) -> Result<()>;
    fn on_order_status(&self, response: OrderStatusResponse) -> Result<()>;
    fn on_best_orders(&self, response: BestOrdersV2Response, show_orig_tickets: bool) -> Result<()>;
    fn on_my_orders(&self, response: Mm2RpcResult<MyOrdersResponse>) -> Result<()>;
    fn on_set_price(&self, response: Mm2RpcResult<MakerOrderForRpc>) -> Result<()>;
    fn on_orderbook_depth(&self, response: Mm2RpcResult<Vec<PairWithDepth>>) -> Result<()>;
    fn on_orders_history(
        &self,
        response: Mm2RpcResult<OrdersHistoryResponse>,
        settings: OrdersHistorySettings,
    ) -> Result<()>;
    fn on_update_maker_order(&self, response: Mm2RpcResult<MakerOrderForRpc>) -> Result<()>;
    fn on_active_swaps(&self, response: ActiveSwapsResponse, uuids_only: bool) -> Result<()>;
    fn on_my_swap_status(&self, response: Mm2RpcResult<MySwapStatusResponse>) -> Result<()>;
    fn on_my_recent_swaps(&self, response: Mm2RpcResult<MyRecentSwapResponse>) -> Result<()>;
    fn on_min_trading_vol(&self, response: Mm2RpcResult<MinTradingVolResponse>) -> Result<()>;
    fn on_max_taker_vol(&self, response: MaxTakerVolResponse) -> Result<()>;
    fn on_recover_funds(&self, response: RecoverFundsOfSwapResponse) -> Result<()>;
    fn on_trade_preimage(&self, response: TradePreimageResponse) -> Result<()>;
    fn on_gossip_mesh(&self, response: Mm2RpcResult<GetGossipMeshResponse>) -> Result<()>;
    fn on_relay_mesh(&self, response: Mm2RpcResult<GetRelayMeshResponse>) -> Result<()>;
    fn on_gossip_peer_topics(&self, response: Mm2RpcResult<GetGossipPeerTopicsResponse>) -> Result<()>;
    fn on_gossip_topic_peers(&self, response: Mm2RpcResult<GetGossipTopicPeersResponse>) -> Result<()>;
    fn on_my_peer_id(&self, response: Mm2RpcResult<GetMyPeerIdResponse>) -> Result<()>;
    fn on_peers_info(&self, response: Mm2RpcResult<GetPeersInfoResponse>) -> Result<()>;
    fn on_set_confirmations(&self, resonse: Mm2RpcResult<SetRequiredConfResponse>) -> Result<()>;
    fn on_set_notarization(&self, response: Mm2RpcResult<SetRequiredNotaResponse>) -> Result<()>;
    fn on_coins_to_kickstart(&self, response: Mm2RpcResult<CoinsToKickstartResponse>) -> Result<()>;
    fn on_ban_pubkey(&self, response: Mm2RpcResult<Status>) -> Result<()>;
    fn on_list_banned_pubkeys(&self, response: Mm2RpcResult<ListBannedPubkeysResponse>) -> Result<()>;
    fn on_unban_pubkeys(&self, response: Mm2RpcResult<UnbanPubkeysResponse>) -> Result<()>;
    fn on_current_mtp(&self, response: GetCurrentMtpResponse) -> Result<()>;
    fn on_get_current_mtp_error(&self, response: GetCurrentMtpError);
    fn on_send_raw_transaction(&self, response: SendRawTransactionResponse, bare_output: bool) -> Result<()>;
    fn on_withdraw(&self, response: WithdrawResponse, bare_output: bool) -> Result<()>;
    fn on_tx_history(&self, response: Mm2RpcResult<MyTxHistoryResponse>) -> Result<()>;
    fn on_tx_history_v2(&self, response: MyTxHistoryResponseV2<MyTxHistoryDetails, BytesJson>) -> Result<()>;
    fn on_tx_history_zcoin(&self, response: MyTxHistoryResponseV2<ZcoinTxDetails, i64>) -> Result<()>;
    fn on_public_key(&self, response: GetPublicKeyResponse) -> Result<()>;
    fn on_public_key_hash(&self, response: GetPublicKeyHashResponse) -> Result<()>;
    fn on_raw_transaction(&self, response: GetRawTransactionResponse, bare_output: bool) -> Result<()>;
    fn on_mm_rpc_error_v2(&self, error: MmRpcErrorV2);
    fn on_enable_zcoin_cancel(&self, response: Status) -> Result<()>;
    fn on_enable_zcoin_cancel_error(&self, error: CancelRpcTaskError) -> Result<()>;
    fn on_vstat_add_node(&self, response: Status) -> Result<()>;
    fn on_vstat_error(&self, error: NodeVersionError) -> Result<()>;
    fn on_vstat_rem_node(&self, response: Status) -> Result<()>;
    fn on_vstat_start_collection(&self, response: Status) -> Result<()>;
    fn on_vstat_stop_collection(&self, response: Status) -> Result<()>;
    fn on_vstat_update_collection(&self, response: Status) -> Result<()>;
    fn on_sign_message(&self, response: SignatureResponse) -> Result<()>;
    fn on_verify_message(&self, response: VerificationResponse) -> Result<()>;
    fn on_signature_error(&self, error: SignatureError);
    fn on_verificaton_error(&self, error: VerificationError);
    fn on_private_key(&self, response: Mm2RpcResult<ShowPrivateKeyResponse>) -> Result<()>;
    fn on_validate_address(&self, response: Mm2RpcResult<ValidateAddressResponse>) -> Result<()>;
    fn on_kmd_rewards_info(&self, response: Mm2RpcResult<KmdRewardsInfoResponse>) -> Result<()>;
    fn on_convert_address(&self, response: Mm2RpcResult<ConvertAddressResponse>) -> Result<()>;
    fn on_convert_utxo_address(&self, response: Mm2RpcResult<ConvertUtxoAddressResponse>) -> Result<()>;
}

pub(crate) struct ResponseHandlerImpl<'a> {
    pub(crate) writer: RefCell<&'a mut dyn Write>,
}

impl ResponseHandler for ResponseHandlerImpl<'_> {
    fn print_response(&self, result: Json) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        let object = result
            .as_object()
            .ok_or_else(|| error_anyhow!("Failed to cast result as object"))?;

        object
            .iter()
            .for_each(|value| writeln_safe_io!(writer, "{}: {:?}", value.0, value.1));
        Ok(())
    }

    fn on_orderbook_response<Cfg: KomodefiConfig + 'static>(
        &self,
        response: OrderbookResponse,
        config: &Cfg,
        settings: OrderbookSettings,
    ) -> Result<()> {
        orderbook::on_orderbook_response(self.writer.borrow_mut().deref_mut(), response, config, settings)
    }

    fn on_get_enabled_response(&self, response: Mm2RpcResult<GetEnabledResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        writeln_safe_io!(writer, "{:8} {}", "Ticker", "Address");
        for row in &response.result {
            writeln_safe_io!(writer, "{:8} {}", row.ticker, row.address);
        }
        Ok(())
    }

    fn on_version_response(&self, response: MmVersionResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        writeln_safe_io!(writer, "Version: {}", response.result);
        writeln_safe_io!(writer, "Datetime: {}", response.datetime);
        Ok(())
    }

    fn on_enable_response(&self, response: CoinInitResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        writeln_safe_io!(
            writer,
            "coin: {}\naddress: {}\nbalance: {}\nunspendable_balance: {}\nrequired_confirmations: {}\nrequires_notarization: {}",
            response.coin,
            response.address,
            response.balance,
            response.unspendable_balance,
            response.required_confirmations,
            if response.requires_notarization { "Yes" } else { "No" }
        );
        if let Some(mature_confirmations) = response.mature_confirmations {
            writeln_safe_io!(writer, "mature_confirmations: {}", mature_confirmations);
        }
        Ok(())
    }

    fn on_enable_bch(&self, response: BchWithTokensActivationResult) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_enable_bch(writer.deref_mut(), response)
    }

    fn on_enable_slp(&self, response: SlpInitResult) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_enable_slp(writer.deref_mut(), response)
    }

    fn on_enable_tendermint(&self, response: TendermintActivationResult) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_enable_tendermint(writer.deref_mut(), response)
    }

    fn on_enable_tendermint_token(&self, response: TendermintTokenInitResult) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_enable_tendermint_token(writer.deref_mut(), response)
    }

    fn on_enable_erc20(&self, response: Erc20InitResult) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_enable_erc20(writer.deref_mut(), response)
    }

    fn on_enable_eth_with_tokens(&self, response: EthWithTokensActivationResult) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_enable_eth_with_tokens(writer.deref_mut(), response)
    }

    fn on_enable_z_coin(&self, response: InitRpcTaskResponse) -> TaskId {
        let mut writer = self.writer.borrow_mut();
        activation::on_enable_zcoin(writer.deref_mut(), response)
    }

    fn on_zcoin_status(&self, response: ZCoinStatus) -> Result<bool> {
        let mut writer = self.writer.borrow_mut();
        activation::z_coin::on_enable_zcoin_status(writer.deref_mut(), response)
    }

    fn on_disable_coin(&self, response: DisableCoinResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_disable_coin(writer.deref_mut(), response);
        Ok(())
    }

    fn on_balance_response(&self, response: BalanceResponse) -> Result<()> {
        writeln_safe_io!(
            self.writer.borrow_mut(),
            "coin: {}\nbalance: {}\nunspendable: {}\naddress: {}",
            response.coin,
            response.balance,
            response.unspendable_balance,
            response.address
        );
        Ok(())
    }

    fn on_sell_response(&self, response: Mm2RpcResult<SellBuyResponse>) -> Result<()> {
        writeln_safe_io!(self.writer.borrow_mut(), "{}", response.request.uuid);
        Ok(())
    }

    fn on_buy_response(&self, response: Mm2RpcResult<SellBuyResponse>) -> Result<()> {
        writeln_safe_io!(self.writer.borrow_mut(), "{}", response.request.uuid);
        Ok(())
    }

    fn on_stop_response(&self, response: Mm2RpcResult<Status>) -> Result<()> {
        writeln_safe_io!(self.writer.borrow_mut(), "Service stopped: {}", response.result);
        Ok(())
    }

    fn on_cancel_order_response(&self, response: Mm2RpcResult<Status>) -> Result<()> {
        writeln_safe_io!(self.writer.borrow_mut(), "Order cancelled: {}", response.result);
        Ok(())
    }

    fn on_cancel_all_response(&self, response: Mm2RpcResult<CancelAllOrdersResponse>) -> Result<()> {
        let cancelled = &response.result.cancelled;
        let mut writer = self.writer.borrow_mut();
        if cancelled.is_empty() {
            writeln_safe_io!(writer, "No orders found to be cancelled");
        } else {
            writeln_safe_io!(writer, "Cancelled: {}", cancelled.iter().join(", "));
        }

        let currently_matched = &response.result.currently_matching;
        if !currently_matched.is_empty() {
            writeln_safe_io!(writer, "Currently matched: {}", currently_matched.iter().join(", "));
        }
        Ok(())
    }

    fn on_order_status(&self, response: OrderStatusResponse) -> Result<()> {
        order_status::on_order_status(self.writer.borrow_mut().deref_mut(), response)
    }

    fn on_best_orders(&self, response: BestOrdersV2Response, show_orig_tickets: bool) -> Result<()> {
        best_orders::on_best_orders(self.writer.borrow_mut().deref_mut(), response, show_orig_tickets)
    }

    fn on_my_orders(&self, response: Mm2RpcResult<MyOrdersResponse>) -> Result<()> {
        my_orders::on_my_orders(self.writer.borrow_mut().deref_mut(), response)
    }

    fn on_set_price(&self, response: Mm2RpcResult<MakerOrderForRpc>) -> Result<()> {
        formatters::on_maker_order_response(self.writer.borrow_mut().deref_mut(), response.result)
    }

    fn on_orderbook_depth(&self, response: Mm2RpcResult<Vec<PairWithDepth>>) -> Result<()> {
        orderbook_depth::on_orderbook_depth(self.writer.borrow_mut().deref_mut(), response)
    }

    fn on_orders_history(
        &self,
        response: Mm2RpcResult<OrdersHistoryResponse>,
        settings: OrdersHistorySettings,
    ) -> Result<()> {
        orders_history::on_orders_history(self.writer.borrow_mut().deref_mut(), response, settings)
    }

    fn on_update_maker_order(&self, response: Mm2RpcResult<MakerOrderForRpc>) -> Result<()> {
        formatters::on_maker_order_response(self.writer.borrow_mut().deref_mut(), response.result)
    }

    fn on_active_swaps(&self, response: ActiveSwapsResponse, uuids_only: bool) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        swaps::on_active_swaps(writer.deref_mut(), response, uuids_only)
    }

    fn on_my_swap_status(&self, response: Mm2RpcResult<MySwapStatusResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        swaps::on_my_swap_status(writer.deref_mut(), response.result)
    }

    fn on_my_recent_swaps(&self, response: Mm2RpcResult<MyRecentSwapResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        swaps::on_my_recent_swaps(writer.deref_mut(), response.result)
    }

    fn on_min_trading_vol(&self, response: Mm2RpcResult<MinTradingVolResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        trading::on_min_trading_vol(writer.deref_mut(), response.result)
    }

    fn on_max_taker_vol(&self, response: MaxTakerVolResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        trading::on_max_taker_vol(writer.deref_mut(), response)
    }

    fn on_recover_funds(&self, response: RecoverFundsOfSwapResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        swaps::on_recover_funds(writer.deref_mut(), response)
    }

    fn on_trade_preimage(&self, response: TradePreimageResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        trading::on_trade_preimage(writer.deref_mut(), response)
    }

    fn on_gossip_mesh(&self, response: Mm2RpcResult<GetGossipMeshResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        network::on_gossip_mesh(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_relay_mesh(&self, response: Mm2RpcResult<GetRelayMeshResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        network::on_relay_mesh(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_gossip_peer_topics(&self, response: Mm2RpcResult<GetGossipPeerTopicsResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        network::on_gossip_peer_topics(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_gossip_topic_peers(&self, response: Mm2RpcResult<GetGossipTopicPeersResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        network::on_gossip_topic_peers(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_my_peer_id(&self, response: Mm2RpcResult<GetMyPeerIdResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        network::on_my_peer_id(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_peers_info(&self, response: Mm2RpcResult<GetPeersInfoResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        network::on_peers_info(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_set_confirmations(&self, response: Mm2RpcResult<SetRequiredConfResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_set_confirmations(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_set_notarization(&self, response: Mm2RpcResult<SetRequiredNotaResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_set_notarization(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_coins_to_kickstart(&self, response: Mm2RpcResult<CoinsToKickstartResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::on_coins_to_kickstart(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_ban_pubkey(&self, response: Mm2RpcResult<Status>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        writeln_field(writer.deref_mut(), "Status", response.result, ZERO_INDENT);
        Ok(())
    }

    fn on_list_banned_pubkeys(&self, response: Mm2RpcResult<ListBannedPubkeysResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        utility::on_list_banned_pubkeys(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_unban_pubkeys(&self, response: Mm2RpcResult<UnbanPubkeysResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        utility::on_unban_pubkeys(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_current_mtp(&self, response: GetCurrentMtpResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        utility::on_current_mtp(writer.deref_mut(), response);
        Ok(())
    }

    fn on_get_current_mtp_error(&self, error: GetCurrentMtpError) {
        let mut writer = self.writer.borrow_mut();
        utility::on_get_current_mtp_error(writer.deref_mut(), error);
    }

    fn on_send_raw_transaction(&self, response: SendRawTransactionResponse, bare_output: bool) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_send_raw_transaction(writer.deref_mut(), response, bare_output);
        Ok(())
    }

    fn on_withdraw(&self, response: WithdrawResponse, bare_output: bool) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_withdraw(writer.deref_mut(), response, bare_output)
    }

    fn on_tx_history(&self, response: Mm2RpcResult<MyTxHistoryResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_tx_history(writer.deref_mut(), response.result)
    }

    fn on_tx_history_v2(&self, response: MyTxHistoryResponseV2<MyTxHistoryDetails, BytesJson>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_tx_history_v2(writer.deref_mut(), response)
    }

    fn on_tx_history_zcoin(&self, response: MyTxHistoryResponseV2<ZcoinTxDetails, i64>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_tx_history_zcoin(writer.deref_mut(), response)
    }

    fn on_public_key(&self, response: GetPublicKeyResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_public_key(writer.deref_mut(), response);
        Ok(())
    }

    fn on_public_key_hash(&self, response: GetPublicKeyHashResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_public_key_hash(writer.deref_mut(), response);
        Ok(())
    }

    fn on_raw_transaction(&self, response: GetRawTransactionResponse, bare_output: bool) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_raw_transaction(writer.deref_mut(), response, bare_output);
        Ok(())
    }

    fn on_private_key(&self, response: Mm2RpcResult<ShowPrivateKeyResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_private_key(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_validate_address(&self, response: Mm2RpcResult<ValidateAddressResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_validate_address(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_kmd_rewards_info(&self, response: Mm2RpcResult<KmdRewardsInfoResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_kmd_rewards_info(writer.deref_mut(), response.result)
    }

    fn on_convert_address(&self, response: Mm2RpcResult<ConvertAddressResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_convert_address(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_convert_utxo_address(&self, response: Mm2RpcResult<ConvertUtxoAddressResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        wallet::on_convert_utxo_address(writer.deref_mut(), response.result);
        Ok(())
    }

    fn on_mm_rpc_error_v2(&self, error: MmRpcErrorV2) {
        let mut writer = self.writer.borrow_mut();
        let writer = writer.deref_mut();
        writeln_field(writer, "error", error.error, ZERO_INDENT);
        writeln_field(writer, "error_path", error.error_path, ZERO_INDENT);
        writeln_field(writer, "error_trace", error.error_trace, ZERO_INDENT);
    }

    fn on_enable_zcoin_cancel(&self, response: Status) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        let writer = writer.deref_mut();
        activation::z_coin::on_enable_zcoin_canceled(writer, response);
        Ok(())
    }

    fn on_enable_zcoin_cancel_error(&self, error: CancelRpcTaskError) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        activation::z_coin::on_enable_zcoin_cancel_error(writer.deref_mut(), error);
        Ok(())
    }

    fn on_vstat_add_node(&self, response: Status) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        version_stat::on_vstat_add_node(writer.deref_mut(), response);
        Ok(())
    }

    fn on_vstat_error(&self, error: NodeVersionError) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        version_stat::on_node_version_error(writer.deref_mut(), error);
        Ok(())
    }

    fn on_vstat_rem_node(&self, response: Status) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        version_stat::on_vstat_rem_node(writer.deref_mut(), response);
        Ok(())
    }

    fn on_vstat_start_collection(&self, response: Status) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        version_stat::on_vstat_start_collection(writer.deref_mut(), response);
        Ok(())
    }

    fn on_vstat_stop_collection(&self, response: Status) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        version_stat::on_vstat_stop_collection(writer.deref_mut(), response);
        Ok(())
    }

    fn on_vstat_update_collection(&self, response: Status) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        version_stat::on_vstat_update_collection(writer.deref_mut(), response);
        Ok(())
    }

    fn on_sign_message(&self, response: SignatureResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        message_signing::on_sign_message(writer.deref_mut(), response);
        Ok(())
    }

    fn on_signature_error(&self, error: SignatureError) {
        let mut writer = self.writer.borrow_mut();
        message_signing::on_signature_error(writer.deref_mut(), error);
    }

    fn on_verify_message(&self, response: VerificationResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        message_signing::on_verify_message(writer.deref_mut(), response);
        Ok(())
    }

    fn on_verificaton_error(&self, error: VerificationError) {
        let mut writer = self.writer.borrow_mut();
        message_signing::on_verification_error(writer.deref_mut(), error);
    }
}

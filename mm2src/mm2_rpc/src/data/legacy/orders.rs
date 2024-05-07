use derive_more::Display;
use rpc::v1::types::H256 as H256Json;
use serde::{Deserialize, Serialize};
use skip_serializing_none::skip_serializing_none;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use common::true_f;
use mm2_number::{construct_detailed, BigDecimal, BigRational, Fraction, MmNumber};

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "orderbook")]
pub struct OrderbookRequest {
    pub base: String,
    pub rel: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrderbookResponse {
    #[serde(rename = "askdepth")]
    pub ask_depth: u32,
    pub asks: Vec<AggregatedOrderbookEntry>,
    pub base: String,
    #[serde(rename = "biddepth")]
    pub bid_depth: u32,
    pub bids: Vec<AggregatedOrderbookEntry>,
    pub netid: u16,
    #[serde(rename = "numasks")]
    pub num_asks: usize,
    #[serde(rename = "numbids")]
    pub num_bids: usize,
    pub rel: String,
    pub timestamp: u64,
    #[serde(flatten)]
    pub total_asks_base: TotalAsksBaseVol,
    #[serde(flatten)]
    pub total_asks_rel: TotalAsksRelVol,
    #[serde(flatten)]
    pub total_bids_base: TotalBidsBaseVol,
    #[serde(flatten)]
    pub total_bids_rel: TotalBidsRelVol,
}

construct_detailed!(TotalAsksBaseVol, total_asks_base_vol);
construct_detailed!(TotalAsksRelVol, total_asks_rel_vol);
construct_detailed!(TotalBidsBaseVol, total_bids_base_vol);
construct_detailed!(TotalBidsRelVol, total_bids_rel_vol);

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcOrderbookEntry {
    pub coin: String,
    pub address: String,
    pub price: BigDecimal,
    pub price_rat: BigRational,
    pub price_fraction: Fraction,
    #[serde(rename = "maxvolume")]
    pub max_volume: BigDecimal,
    pub max_volume_rat: BigRational,
    pub max_volume_fraction: Fraction,
    pub min_volume: BigDecimal,
    pub min_volume_rat: BigRational,
    pub min_volume_fraction: Fraction,
    pub pubkey: String,
    pub age: u64,
    pub uuid: Uuid,
    pub is_mine: bool,
    #[serde(flatten)]
    pub base_max_volume: DetailedBaseMaxVolume,
    #[serde(flatten)]
    pub base_min_volume: DetailedBaseMinVolume,
    #[serde(flatten)]
    pub rel_max_volume: DetailedRelMaxVolume,
    #[serde(flatten)]
    pub rel_min_volume: DetailedRelMinVolume,
    #[serde(flatten)]
    pub conf_settings: Option<OrderConfirmationsSettings>,
}

construct_detailed!(DetailedBaseMaxVolume, base_max_volume);
construct_detailed!(DetailedBaseMinVolume, base_min_volume);
construct_detailed!(DetailedRelMaxVolume, rel_max_volume);
construct_detailed!(DetailedRelMinVolume, rel_min_volume);

#[derive(Debug, Serialize, Deserialize)]
pub struct AggregatedOrderbookEntry {
    #[serde(flatten)]
    pub entry: RpcOrderbookEntry,
    #[serde(flatten)]
    pub base_max_volume_aggr: AggregatedBaseVol,
    #[serde(flatten)]
    pub rel_max_volume_aggr: AggregatedRelVol,
}

construct_detailed!(AggregatedBaseVol, base_max_volume_aggr);
construct_detailed!(AggregatedRelVol, rel_max_volume_aggr);

#[derive(Serialize)]
#[serde(tag = "method", rename = "sell")]
pub struct SellRequest {
    #[serde(flatten)]
    pub delegate: SellBuyRequest,
}

#[derive(Serialize)]
#[serde(tag = "method", rename = "buy")]
pub struct BuyRequest {
    #[serde(flatten)]
    pub delegate: SellBuyRequest,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize)]
pub struct SellBuyRequest {
    pub base: String,
    pub rel: String,
    pub price: MmNumber,
    pub volume: MmNumber,
    pub timeout: Option<u64>,
    /// Not used. Deprecated.
    #[allow(dead_code)]
    pub duration: Option<u32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[allow(dead_code)]
    pub gui: Option<String>,
    #[serde(rename = "destpubkey")]
    #[serde(default)]
    #[allow(dead_code)]
    pub dest_pub_key: H256Json,
    #[serde(default)]
    pub match_by: MatchBy,
    #[serde(default)]
    pub order_type: OrderType,
    pub base_confs: Option<u64>,
    pub base_nota: Option<bool>,
    pub rel_confs: Option<u64>,
    pub rel_nota: Option<bool>,
    pub min_volume: Option<MmNumber>,
    #[serde(default = "true_f")]
    pub save_in_history: bool,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize)]
pub struct SellBuyResponse {
    #[serde(flatten)]
    pub request: TakerRequestForRpc,
    pub order_type: OrderType,
    #[serde(flatten)]
    pub min_volume: DetailedMinVolume,
    pub base_orderbook_ticker: Option<String>,
    pub rel_orderbook_ticker: Option<String>,
}

construct_detailed!(DetailedMinVolume, min_volume);

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TakerRequestForRpc {
    pub uuid: Uuid,
    pub base: String,
    pub rel: String,
    pub base_amount: BigDecimal,
    pub base_amount_rat: BigRational,
    pub rel_amount: BigDecimal,
    pub rel_amount_rat: BigRational,
    pub action: TakerAction,
    pub method: String,
    pub sender_pubkey: H256Json,
    pub dest_pub_key: H256Json,
    pub match_by: MatchBy,
    pub conf_settings: Option<OrderConfirmationsSettings>,
}

#[derive(Clone, Display, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TakerAction {
    Buy,
    Sell,
}

#[derive(Clone, Display, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum OrderType {
    FillOrKill,
    GoodTillCancelled,
}

impl Default for OrderType {
    fn default() -> Self { OrderType::GoodTillCancelled }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum MatchBy {
    Any,
    Orders(HashSet<Uuid>),
    Pubkeys(HashSet<H256Json>),
}

impl Default for MatchBy {
    fn default() -> Self { MatchBy::Any }
}

#[derive(Clone, Default, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OrderConfirmationsSettings {
    pub base_confs: u64,
    pub base_nota: bool,
    pub rel_confs: u64,
    pub rel_nota: bool,
}

impl OrderConfirmationsSettings {
    pub fn reversed(&self) -> OrderConfirmationsSettings {
        OrderConfirmationsSettings {
            base_confs: self.rel_confs,
            base_nota: self.rel_nota,
            rel_confs: self.base_confs,
            rel_nota: self.base_nota,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "cancel_order")]
pub struct CancelOrderRequest {
    pub uuid: Uuid,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "cancel_all_orders")]
pub struct CancelAllOrdersRequest {
    pub cancel_by: CancelBy,
}

#[derive(Serialize, Deserialize)]
pub struct CancelAllOrdersResponse {
    pub cancelled: Vec<Uuid>,
    pub currently_matching: Vec<Uuid>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CancelBy {
    All,
    Pair { base: String, rel: String },
    Coin { ticker: String },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "order_status")]
pub struct OrderStatusRequest {
    pub uuid: Uuid,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "order")]
pub enum OrderStatusResponse {
    Maker(MakerOrderForMyOrdersRpc),
    Taker(TakerOrderForRpc),
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize)]
pub struct MakerOrderForRpc {
    pub uuid: Uuid,
    pub base: String,
    pub rel: String,
    pub price: BigDecimal,
    pub price_rat: BigRational,
    pub max_base_vol: BigDecimal,
    pub max_base_vol_rat: BigRational,
    pub min_base_vol: BigDecimal,
    pub min_base_vol_rat: BigRational,
    pub created_at: u64,
    pub updated_at: Option<u64>,
    pub matches: HashMap<Uuid, MakerMatchForRpc>,
    pub started_swaps: Vec<Uuid>,
    pub conf_settings: Option<OrderConfirmationsSettings>,
    pub changes_history: Option<Vec<HistoricalOrder>>,
    pub base_orderbook_ticker: Option<String>,
    pub rel_orderbook_ticker: Option<String>,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize)]
pub struct TakerOrderForRpc {
    pub request: TakerRequestForRpc,
    pub created_at: u64,
    pub matches: HashMap<Uuid, TakerMatchForRpc>,
    pub order_type: OrderType,
    pub cancellable: bool,
    pub base_orderbook_ticker: Option<String>,
    pub rel_orderbook_ticker: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HistoricalOrder {
    pub max_base_vol: Option<BigRational>,
    pub min_base_vol: Option<BigRational>,
    pub price: Option<BigRational>,
    pub updated_at: Option<u64>,
    pub conf_settings: Option<OrderConfirmationsSettings>,
}

#[derive(Serialize, Deserialize)]
pub struct MakerOrderForMyOrdersRpc {
    #[serde(flatten)]
    pub order: MakerOrderForRpc,
    pub cancellable: bool,
    pub available_amount: BigDecimal,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize)]
pub struct TakerMatchForRpc {
    pub reserved: MakerReservedForRpc,
    pub connect: TakerConnectForRpc,
    pub connected: Option<MakerConnectedForRpc>,
    pub last_updated: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "order")]
pub enum OrderForRpc {
    Maker(MakerOrderForRpc),
    Taker(TakerOrderForRpc),
}

#[skip_serializing_none]
#[derive(Debug, Serialize, Deserialize)]
pub struct MakerMatchForRpc {
    pub request: TakerRequestForRpc,
    pub reserved: MakerReservedForRpc,
    pub connect: Option<TakerConnectForRpc>,
    pub connected: Option<MakerConnectedForRpc>,
    pub last_updated: u64,
}

#[skip_serializing_none]
#[derive(Debug, Serialize, Deserialize)]
pub struct MakerReservedForRpc {
    pub base: String,
    pub rel: String,
    pub base_amount: BigDecimal,
    pub base_amount_rat: BigRational,
    pub rel_amount: BigDecimal,
    pub rel_amount_rat: BigRational,
    pub taker_order_uuid: Uuid,
    pub maker_order_uuid: Uuid,
    pub sender_pubkey: H256Json,
    pub dest_pub_key: H256Json,
    pub conf_settings: Option<OrderConfirmationsSettings>,
    pub method: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TakerConnectForRpc {
    pub taker_order_uuid: Uuid,
    pub maker_order_uuid: Uuid,
    pub method: String,
    pub sender_pubkey: H256Json,
    pub dest_pub_key: H256Json,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MakerConnectedForRpc {
    pub taker_order_uuid: Uuid,
    pub maker_order_uuid: Uuid,
    pub method: String,
    pub sender_pubkey: H256Json,
    pub dest_pub_key: H256Json,
}

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "my_orders")]
pub struct MyOrdersRequest {}

#[derive(Serialize, Deserialize)]
pub struct MyOrdersResponse {
    pub maker_orders: HashMap<Uuid, MakerOrderForMyOrdersRpc>,
    pub taker_orders: HashMap<Uuid, TakerOrderForRpc>,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "setprice")]
pub struct SetPriceRequest {
    pub base: String,
    pub rel: String,
    pub price: MmNumber,
    #[serde(default)]
    pub max: bool,
    #[serde(default)]
    pub volume: MmNumber,
    pub min_volume: Option<MmNumber>,
    #[serde(default = "true_f")]
    pub cancel_previous: bool,
    pub base_confs: Option<u64>,
    pub base_nota: Option<bool>,
    pub rel_confs: Option<u64>,
    pub rel_nota: Option<bool>,
    #[serde(default = "true_f")]
    pub save_in_history: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", rename = "orderbook_depth")]
pub struct OrderbookDepthRequest {
    pub pairs: Vec<(String, String)>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PairWithDepth {
    pub pair: (String, String),
    pub depth: PairDepth,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PairDepth {
    pub asks: usize,
    pub bids: usize,
}

#[skip_serializing_none]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", rename = "orders_history_by_filter")]
pub struct OrdersHistoryRequest {
    pub order_type: Option<String>,
    pub initial_action: Option<String>,
    pub base: Option<String>,
    pub rel: Option<String>,
    pub from_price: Option<MmNumber>,
    pub to_price: Option<MmNumber>,
    pub from_volume: Option<MmNumber>,
    pub to_volume: Option<MmNumber>,
    pub from_timestamp: Option<u64>,
    pub to_timestamp: Option<u64>,
    pub was_taker: Option<bool>,
    pub status: Option<String>,
    #[serde(default)]
    pub include_details: bool,
}

#[derive(Serialize, Deserialize)]
pub struct OrdersHistoryResponse {
    pub orders: Vec<FilteringOrder>,
    pub details: Vec<OrderForRpc>,
    pub found_records: usize,
    pub warnings: Vec<UuidParseError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FilteringOrder {
    pub uuid: String,
    pub order_type: String,
    pub initial_action: String,
    pub base: String,
    pub rel: String,
    pub price: f64,
    pub volume: f64,
    pub created_at: i64,
    pub last_updated: i64,
    pub was_taker: i8,
    pub status: String,
}

#[derive(Serialize, Deserialize)]
pub struct UuidParseError {
    pub uuid: String,
    pub warning: String,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "update_maker_order")]
pub struct UpdateMakerOrderRequest {
    pub uuid: Uuid,
    pub new_price: Option<MmNumber>,
    pub max: Option<bool>,
    pub volume_delta: Option<MmNumber>,
    pub min_volume: Option<MmNumber>,
    pub base_confs: Option<u64>,
    pub base_nota: Option<bool>,
    pub rel_confs: Option<u64>,
    pub rel_nota: Option<bool>,
}

#[derive(Deserialize, Serialize)]
pub struct MinTradingVolResponse {
    pub coin: String,
    #[serde(flatten)]
    pub volume: DetailedMinTradingVol,
}

construct_detailed!(DetailedMinTradingVol, min_trading_vol);

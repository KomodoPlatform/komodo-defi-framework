#![allow(dead_code, unused_variables)]

/// The helper structs used in testing of RPC responses, these should be separated from actual MM2 code to ensure
/// backwards compatibility
use bigdecimal::BigDecimal;
use common::mm_number::Fraction;
use num_rational::BigRational;
use rpc::v1::types::H256 as H256Json;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum OrderType {
    FillOrKill,
    GoodTillCancelled,
}

#[derive(Deserialize)]
pub struct OrderConfirmationsSettings {
    pub base_confs: u64,
    pub base_nota: bool,
    pub rel_confs: u64,
    pub rel_nota: bool,
}

#[derive(Deserialize)]
pub enum TakerAction {
    Buy,
    Sell,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum MatchBy {
    Any,
    Orders(HashSet<Uuid>),
    Pubkeys(HashSet<H256Json>),
}

#[derive(Deserialize)]
pub struct BuyOrSellRpcRes {
    pub base: String,
    pub rel: String,
    pub base_amount: BigDecimal,
    pub base_amount_rat: BigRational,
    pub rel_amount: BigDecimal,
    pub rel_amount_rat: BigRational,
    pub min_volume: BigDecimal,
    pub min_volume_rat: BigRational,
    pub min_volume_fraction: Fraction,
    pub action: TakerAction,
    pub uuid: Uuid,
    pub method: String,
    pub sender_pubkey: H256Json,
    pub dest_pub_key: H256Json,
    pub match_by: MatchBy,
    pub conf_settings: OrderConfirmationsSettings,
    pub order_type: OrderType,
}

#[derive(Deserialize)]
pub struct BuyOrSellRpcResult {
    pub result: BuyOrSellRpcRes,
}

#[derive(Deserialize)]
pub struct TakerRequest {
    base: String,
    rel: String,
    base_amount: BigDecimal,
    base_amount_rat: BigRational,
    rel_amount: BigDecimal,
    rel_amount_rat: BigRational,
    action: TakerAction,
    uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
    match_by: MatchBy,
    conf_settings: OrderConfirmationsSettings,
}

#[derive(Deserialize)]
pub struct MakerReserved {
    base: String,
    rel: String,
    base_amount: BigDecimal,
    base_amount_rat: BigRational,
    rel_amount: BigDecimal,
    rel_amount_rat: BigRational,
    taker_order_uuid: Uuid,
    maker_order_uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
    conf_settings: OrderConfirmationsSettings,
}

#[derive(Deserialize)]
pub struct TakerConnect {
    taker_order_uuid: Uuid,
    maker_order_uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
}

#[derive(Deserialize)]
pub struct MakerConnected {
    taker_order_uuid: Uuid,
    maker_order_uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
}

#[derive(Deserialize)]
pub struct MakerMatch {
    request: TakerRequest,
    reserved: MakerReserved,
    connect: Option<TakerConnect>,
    connected: Option<MakerConnected>,
    last_updated: u64,
}

#[derive(Deserialize)]
pub struct MakerOrderRpcResult {
    pub max_base_vol: BigDecimal,
    pub max_base_vol_rat: BigRational,
    pub min_base_vol: BigDecimal,
    pub min_base_vol_rat: BigRational,
    pub price: BigDecimal,
    pub price_rat: BigRational,
    pub created_at: u64,
    pub base: String,
    pub rel: String,
    pub matches: HashMap<Uuid, MakerMatch>,
    pub started_swaps: Vec<Uuid>,
    pub uuid: Uuid,
    pub conf_settings: Option<OrderConfirmationsSettings>,
}

#[derive(Deserialize)]
pub struct SetPriceResult {
    pub result: MakerOrderRpcResult,
}

#[derive(Deserialize)]
pub struct TakerMatch {
    reserved: MakerReserved,
    connect: TakerConnect,
    connected: Option<MakerConnected>,
    last_updated: u64,
}

#[derive(Deserialize)]
pub struct TakerOrderRpcResult {
    created_at: u64,
    request: TakerRequest,
    matches: HashMap<Uuid, TakerMatch>,
    order_type: OrderType,
}

#[derive(Deserialize)]
pub struct MyOrdersRpc {
    pub maker_orders: HashMap<Uuid, MakerOrderRpcResult>,
    pub taker_orders: HashMap<Uuid, TakerOrderRpcResult>,
}

#[derive(Deserialize)]
pub struct MyOrdersRpcResult {
    pub result: MyOrdersRpc,
}

#[derive(Debug, Deserialize)]
pub struct OrderbookEntry {
    pub coin: String,
    pub address: String,
    pub price: BigDecimal,
    pub price_rat: BigRational,
    pub price_fraction: Fraction,
    #[serde(rename = "maxvolume")]
    pub max_volume: BigDecimal,
    pub max_volume_rat: BigRational,
    pub max_volume_fraction: Fraction,
    pub base_max_volume: BigDecimal,
    pub base_max_volume_rat: BigRational,
    pub base_max_volume_fraction: Fraction,
    pub base_min_volume: BigDecimal,
    pub base_min_volume_rat: BigRational,
    pub base_min_volume_fraction: Fraction,
    pub rel_max_volume: BigDecimal,
    pub rel_max_volume_rat: BigRational,
    pub rel_max_volume_fraction: Fraction,
    pub rel_min_volume: BigDecimal,
    pub rel_min_volume_rat: BigRational,
    pub rel_min_volume_fraction: Fraction,
    pub min_volume: BigDecimal,
    pub min_volume_rat: BigRational,
    pub min_volume_fraction: Fraction,
    pub pubkey: String,
    pub age: i64,
    pub zcredits: u64,
    pub uuid: Uuid,
    pub is_mine: bool,
}

#[derive(Deserialize)]
pub struct BestOrdersResponse {
    pub result: HashMap<String, Vec<OrderbookEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct OrderbookResponse {
    #[serde(rename = "askdepth")]
    pub ask_depth: usize,
    pub asks: Vec<OrderbookEntry>,
    pub bids: Vec<OrderbookEntry>,
}

#[derive(Deserialize)]
pub struct PairDepth {
    pub asks: usize,
    pub bids: usize,
}

#[derive(Deserialize)]
pub struct PairWithDepth {
    pub pair: (String, String),
    pub depth: PairDepth,
}

#[derive(Deserialize)]
pub struct OrderbookDepthResponse {
    pub result: Vec<PairWithDepth>,
}

#[derive(Debug, Deserialize)]
pub struct EnableElectrumResponse {
    pub coin: String,
    pub address: String,
    pub balance: BigDecimal,
    pub required_confirmations: u64,
    pub requires_notarization: bool,
    pub result: String,
}

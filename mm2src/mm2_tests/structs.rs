#![allow(dead_code, unused_variables)]

/// The helper structs used in testing of RPC responses, these should be separated from actual MM2 code to ensure
/// backwards compatibility
use bigdecimal::BigDecimal;
use num_rational::BigRational;
use rpc::v1::types::H256 as H256Json;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
enum OrderType {
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
pub struct BuyOrSellRpcResult {
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
    order_type: OrderType,
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
    max_base_vol: BigDecimal,
    max_base_vol_rat: BigRational,
    min_base_vol: BigDecimal,
    min_base_vol_rat: BigRational,
    price: BigDecimal,
    price_rat: BigRational,
    created_at: u64,
    base: String,
    rel: String,
    matches: HashMap<Uuid, MakerMatch>,
    started_swaps: Vec<Uuid>,
    uuid: Uuid,
    conf_settings: Option<OrderConfirmationsSettings>,
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
pub struct MyOrdersRpcResult {
    maker_orders: HashMap<Uuid, MakerOrderRpcResult>,
    taker_orders: HashMap<Uuid, TakerOrderRpcResult>,
}

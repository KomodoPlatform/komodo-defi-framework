use super::{subscribe_to_orderbook_topic, OrdermatchContext, RpcOrderbookEntry};
use crate::mm2::lp_ordermatch::addr_format_from_protocol_info;
use coins::{address_by_coin_conf_and_pubkey_str, coin_conf, is_wallet_only_conf};
use common::{mm_ctx::MmArc, mm_number::MmNumber, now_ms};
use crypto::CryptoCtx;
use http::Response;
use num_rational::BigRational;
use num_traits::Zero;
use serde_json::{self as json, Value as Json};

#[derive(Deserialize)]
struct OrderbookReq {
    base: String,
    rel: String,
}

construct_detailed!(TotalAsksBaseVol, total_asks_base_vol);
construct_detailed!(TotalAsksRelVol, total_asks_rel_vol);
construct_detailed!(TotalBidsBaseVol, total_bids_base_vol);
construct_detailed!(TotalBidsRelVol, total_bids_rel_vol);
construct_detailed!(AggregatedBaseVol, base_max_volume_aggr);
construct_detailed!(AggregatedRelVol, rel_max_volume_aggr);

#[derive(Debug, Serialize)]
pub struct AggregatedOrderbookEntry {
    #[serde(flatten)]
    entry: RpcOrderbookEntry,
    #[serde(flatten)]
    base_max_volume_aggr: AggregatedBaseVol,
    #[serde(flatten)]
    rel_max_volume_aggr: AggregatedRelVol,
}

#[derive(Debug, Serialize)]
pub struct OrderbookResponse {
    #[serde(rename = "askdepth")]
    ask_depth: u32,
    asks: Vec<AggregatedOrderbookEntry>,
    base: String,
    #[serde(rename = "biddepth")]
    bid_depth: u32,
    bids: Vec<AggregatedOrderbookEntry>,
    netid: u16,
    #[serde(rename = "numasks")]
    num_asks: usize,
    #[serde(rename = "numbids")]
    num_bids: usize,
    rel: String,
    timestamp: u64,
    #[serde(flatten)]
    total_asks_base: TotalAsksBaseVol,
    #[serde(flatten)]
    total_asks_rel: TotalAsksRelVol,
    #[serde(flatten)]
    total_bids_base: TotalBidsBaseVol,
    #[serde(flatten)]
    total_bids_rel: TotalBidsRelVol,
}

fn build_aggregated_entries(entries: Vec<RpcOrderbookEntry>) -> (Vec<AggregatedOrderbookEntry>, MmNumber, MmNumber) {
    let mut total_base = BigRational::zero();
    let mut total_rel = BigRational::zero();
    let aggregated = entries
        .into_iter()
        .map(|entry| {
            total_base += entry.base_max_volume.as_ratio();
            total_rel += entry.rel_max_volume.as_ratio();
            AggregatedOrderbookEntry {
                entry,
                base_max_volume_aggr: MmNumber::from(total_base.clone()).into(),
                rel_max_volume_aggr: MmNumber::from(total_rel.clone()).into(),
            }
        })
        .collect();
    (aggregated, total_base.into(), total_rel.into())
}

pub async fn orderbook_rpc(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: OrderbookReq = try_s!(json::from_value(req));
    if req.base == req.rel {
        return ERR!("Base and rel must be different coins");
    }
    let base_coin_conf = coin_conf(&ctx, &req.base);
    if base_coin_conf.is_null() {
        return ERR!("Coin {} is not found in config", req.base);
    }
    if is_wallet_only_conf(&base_coin_conf) {
        return ERR!("Base Coin {} is wallet only", req.base);
    }
    let rel_coin_conf = coin_conf(&ctx, &req.rel);
    if rel_coin_conf.is_null() {
        return ERR!("Coin {} is not found in config", req.rel);
    }
    if is_wallet_only_conf(&rel_coin_conf) {
        return ERR!("Base Coin {} is wallet only", req.rel);
    }
    let ordermatch_ctx = try_s!(OrdermatchContext::from_ctx(&ctx));
    let request_orderbook = true;
    let base_ticker = ordermatch_ctx.orderbook_ticker_bypass(&req.base);
    let rel_ticker = ordermatch_ctx.orderbook_ticker_bypass(&req.rel);
    if base_ticker == rel_ticker && base_coin_conf["protocol"] == rel_coin_conf["protocol"] {
        return ERR!("Base and rel coins have the same orderbook tickers and protocols.");
    }

    try_s!(subscribe_to_orderbook_topic(&ctx, &base_ticker, &rel_ticker, request_orderbook).await);
    let orderbook = ordermatch_ctx.orderbook.lock();
    let my_pubsecp = try_s!(CryptoCtx::from_ctx(&ctx)).secp256k1_pubkey_hex();

    let mut asks = match orderbook.unordered.get(&(base_ticker.clone(), rel_ticker.clone())) {
        Some(uuids) => {
            let mut orderbook_entries = Vec::new();
            for uuid in uuids {
                let ask = orderbook.order_set.get(uuid).ok_or(ERRL!(
                    "Orderbook::unordered contains {:?} uuid that is not in Orderbook::order_set",
                    uuid
                ))?;
                let address_format = addr_format_from_protocol_info(&ask.base_protocol_info);
                let address = try_s!(address_by_coin_conf_and_pubkey_str(
                    &ctx,
                    &req.base,
                    &base_coin_conf,
                    &ask.pubkey,
                    address_format,
                ));
                let is_mine = my_pubsecp == ask.pubkey;
                orderbook_entries.push(ask.as_rpc_entry_ask(address, is_mine));
            }
            orderbook_entries
        },
        None => Vec::new(),
    };
    asks.sort_unstable_by(|ask1, ask2| ask1.price_rat.cmp(&ask2.price_rat));
    let (mut asks, total_asks_base_vol, total_asks_rel_vol) = build_aggregated_entries(asks);
    asks.reverse();

    let mut bids = match orderbook.unordered.get(&(rel_ticker, base_ticker)) {
        Some(uuids) => {
            let mut orderbook_entries = vec![];
            for uuid in uuids {
                let bid = orderbook.order_set.get(uuid).ok_or(ERRL!(
                    "Orderbook::unordered contains {:?} uuid that is not in Orderbook::order_set",
                    uuid
                ))?;
                let address_format = addr_format_from_protocol_info(&bid.base_protocol_info);
                let address = try_s!(address_by_coin_conf_and_pubkey_str(
                    &ctx,
                    &req.rel,
                    &rel_coin_conf,
                    &bid.pubkey,
                    address_format,
                ));
                let is_mine = my_pubsecp == bid.pubkey;
                orderbook_entries.push(bid.as_rpc_entry_bid(address, is_mine));
            }
            orderbook_entries
        },
        None => vec![],
    };
    bids.sort_unstable_by(|bid1, bid2| bid2.price_rat.cmp(&bid1.price_rat));
    let (bids, total_bids_base_vol, total_bids_rel_vol) = build_aggregated_entries(bids);

    let response = OrderbookResponse {
        num_asks: asks.len(),
        num_bids: bids.len(),
        ask_depth: 0,
        asks,
        base: req.base,
        bid_depth: 0,
        bids,
        netid: ctx.netid(),
        rel: req.rel,
        timestamp: now_ms() / 1000,
        total_asks_base: total_asks_base_vol.into(),
        total_asks_rel: total_asks_rel_vol.into(),
        total_bids_base: total_bids_base_vol.into(),
        total_bids_rel: total_bids_rel_vol.into(),
    };
    let response = try_s!(json::to_vec(&response));
    Ok(try_s!(Response::builder().body(response)))
}

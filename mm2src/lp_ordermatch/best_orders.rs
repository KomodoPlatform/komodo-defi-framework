use super::{OrderbookItemWithProof, OrdermatchContext, OrdermatchRequest};
use crate::mm2::lp_network::{request_any_relay, P2PRequest};
use coins::{address_by_coin_conf_and_pubkey_str, coin_conf};
use common::log;
use common::mm_ctx::MmArc;
use common::mm_number::MmNumber;
use http::Response;
use num_rational::BigRational;
use num_traits::Zero;
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BestOrdersAction {
    Buy,
    Sell,
}

#[derive(Debug, Deserialize)]
struct BestOrdersRequest {
    coin: String,
    action: BestOrdersAction,
    volume: MmNumber,
}

#[derive(Debug, Deserialize, Serialize)]
struct BestOrdersRes {
    orders: HashMap<String, Vec<OrderbookItemWithProof>>,
}

pub async fn process_best_orders_p2p_request(
    ctx: MmArc,
    coin: String,
    action: BestOrdersAction,
    required_volume: BigRational,
) -> Result<Option<Vec<u8>>, String> {
    let ordermatch_ctx = OrdermatchContext::from_ctx(&ctx).expect("ordermatch_ctx must exist at this point");
    let orderbook = ordermatch_ctx.orderbook.lock().await;
    let search_pairs_in = match action {
        BestOrdersAction::Buy => &orderbook.pairs_existing_for_base,
        BestOrdersAction::Sell => &orderbook.pairs_existing_for_rel,
    };
    let tickers = match search_pairs_in.get(&coin) {
        Some(tickers) => tickers,
        None => return Ok(None),
    };
    let mut result = HashMap::new();
    let pairs = tickers.iter().map(|ticker| match action {
        BestOrdersAction::Buy => (coin.clone(), ticker.clone()),
        BestOrdersAction::Sell => (ticker.clone(), coin.clone()),
    });
    for pair in pairs {
        let orders = match orderbook.ordered.get(&pair) {
            Some(orders) => orders,
            None => {
                log::warn!("No orders for pair {:?}", pair);
                continue;
            },
        };
        let mut best_orders = vec![];
        let mut collected_volume = BigRational::zero();
        for ordered in orders {
            match orderbook.order_set.get(&ordered.uuid) {
                Some(o) => {
                    let min_volume = match action {
                        BestOrdersAction::Buy => o.min_volume.clone(),
                        BestOrdersAction::Sell => &o.min_volume * &o.price,
                    };
                    if min_volume > required_volume {
                        log::debug!("Order {} min_vol {:?} > {:?}", o.uuid, min_volume, required_volume);
                        continue;
                    }

                    let max_volume = match action {
                        BestOrdersAction::Buy => o.max_volume.clone(),
                        BestOrdersAction::Sell => &o.max_volume * &o.price,
                    };
                    match orderbook.orderbook_item_with_proof(o.clone()) {
                        Ok(order_w_proof) => best_orders.push(order_w_proof),
                        Err(e) => {
                            log::error!("Error {:?} on proof generation for order {:?}", e, o);
                            continue;
                        },
                    };
                    collected_volume += max_volume;
                    if collected_volume >= required_volume {
                        break;
                    }
                },
                None => {
                    log::warn!("No order with uuid {:?}", ordered.uuid);
                    continue;
                },
            };
        }
        match action {
            BestOrdersAction::Buy => result.insert(pair.1, best_orders),
            BestOrdersAction::Sell => result.insert(pair.0, best_orders),
        };
    }
    let response = BestOrdersRes { orders: result };
    let encoded = rmp_serde::to_vec(&response).expect("rmp_serde::to_vec should not fail here");
    Ok(Some(encoded))
}

pub async fn best_orders_rpc(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: BestOrdersRequest = try_s!(json::from_value(req));
    let p2p_request = OrdermatchRequest::BestOrders {
        coin: req.coin,
        action: req.action,
        volume: req.volume.into(),
    };

    let best_orders_res =
        try_s!(request_any_relay::<BestOrdersRes>(ctx.clone(), P2PRequest::Ordermatch(p2p_request)).await);
    let mut response = HashMap::new();
    if let Some((p2p_response, peer_id)) = best_orders_res {
        log::debug!("Got best orders {:?} from peer {}", p2p_response, peer_id);
        for (coin, orders_w_proofs) in p2p_response.orders {
            let coin_conf = coin_conf(&ctx, &coin);
            if coin_conf.is_null() {
                log::warn!("Coin {} is not found in config", coin);
                continue;
            }
            for order_w_proof in orders_w_proofs {
                let order = order_w_proof.order;
                let address = match address_by_coin_conf_and_pubkey_str(&coin, &coin_conf, &order.pubkey) {
                    Ok(a) => a,
                    Err(e) => {
                        log::error!("Error {} getting coin {} address from pubkey {}", e, coin, order.pubkey);
                        continue;
                    },
                };
                let entry = match req.action {
                    BestOrdersAction::Buy => order.as_rpc_entry_ask(address, false),
                    BestOrdersAction::Sell => order.as_rpc_entry_bid(address, false),
                };
                response.entry(coin.clone()).or_insert_with(Vec::new).push(entry);
            }
        }
    }

    let res = json!({ "result": response });
    Response::builder()
        .body(json::to_vec(&res).expect("Serialization failed"))
        .map_err(|e| ERRL!("{}", e))
}

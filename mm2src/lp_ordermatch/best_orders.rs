use super::{addr_format_from_protocol_info, BaseRelProtocolInfo, OrderbookP2PItemWithProof, OrdermatchContext,
            OrdermatchRequest};
use crate::mm2::lp_network::{request_any_relay, P2PRequest};
use coins::{address_by_coin_conf_and_pubkey_str, coin_conf, is_wallet_only_conf, is_wallet_only_ticker};
use common::log;
use common::mm_ctx::MmArc;
use common::mm_number::MmNumber;
use http::Response;
use num_rational::BigRational;
use num_traits::Zero;
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;
use uuid::Uuid;

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
    orders: HashMap<String, Vec<OrderbookP2PItemWithProof>>,
    #[serde(default)]
    protocol_infos: HashMap<Uuid, BaseRelProtocolInfo>,
}

pub fn process_best_orders_p2p_request(
    ctx: MmArc,
    coin: String,
    action: BestOrdersAction,
    required_volume: BigRational,
) -> Result<Option<Vec<u8>>, String> {
    let ordermatch_ctx = OrdermatchContext::from_ctx(&ctx).expect("ordermatch_ctx must exist at this point");
    let orderbook = ordermatch_ctx.orderbook.lock();
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

    let mut protocol_infos = HashMap::new();

    for pair in pairs {
        let orders = match orderbook.ordered.get(&pair) {
            Some(orders) => orders,
            None => {
                log::debug!("No orders for pair {:?}", pair);
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
                    let order_w_proof = orderbook.orderbook_item_with_proof(o.clone());
                    protocol_infos.insert(order_w_proof.order.uuid, order_w_proof.order.base_rel_proto_info());
                    best_orders.push(order_w_proof.into());

                    collected_volume += max_volume;
                    if collected_volume >= required_volume {
                        break;
                    }
                },
                None => {
                    log::debug!("No order with uuid {:?}", ordered.uuid);
                    continue;
                },
            };
        }
        match action {
            BestOrdersAction::Buy => result.insert(pair.1, best_orders),
            BestOrdersAction::Sell => result.insert(pair.0, best_orders),
        };
    }
    let response = BestOrdersRes {
        orders: result,
        protocol_infos,
    };
    let encoded = rmp_serde::to_vec(&response).expect("rmp_serde::to_vec should not fail here");
    Ok(Some(encoded))
}

pub async fn best_orders_rpc(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: BestOrdersRequest = try_s!(json::from_value(req));
    let ordermatch_ctx = OrdermatchContext::from_ctx(&ctx).unwrap();
    if is_wallet_only_ticker(&ctx, &req.coin) {
        return ERR!("Coin {} is wallet only", &req.coin);
    }
    let p2p_request = OrdermatchRequest::BestOrders {
        coin: ordermatch_ctx.orderbook_ticker_bypass(&req.coin),
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
            if is_wallet_only_conf(&coin_conf) {
                log::warn!(
                    "Coin {} was removed from best orders because it's defined as wallet only in config",
                    coin
                );
                continue;
            }
            for order_w_proof in orders_w_proofs {
                let order = order_w_proof.order;
                let empty_proto_info = BaseRelProtocolInfo::default();
                let proto_infos = p2p_response
                    .protocol_infos
                    .get(&order.uuid)
                    .unwrap_or(&empty_proto_info);
                let addr_format = match req.action {
                    BestOrdersAction::Buy => addr_format_from_protocol_info(&proto_infos.rel),
                    BestOrdersAction::Sell => addr_format_from_protocol_info(&proto_infos.base),
                };
                let address =
                    match address_by_coin_conf_and_pubkey_str(&ctx, &coin, &coin_conf, &order.pubkey, addr_format) {
                        Ok(a) => a,
                        Err(e) => {
                            log::error!("Error {} getting coin {} address from pubkey {}", e, coin, order.pubkey);
                            continue;
                        },
                    };
                let entry = match req.action {
                    BestOrdersAction::Buy => order.as_rpc_best_orders_buy(address, false),
                    BestOrdersAction::Sell => order.as_rpc_best_orders_sell(address, false),
                };
                response.entry(coin.clone()).or_insert_with(Vec::new).push(entry);
            }
        }
    }

    let res = json!({ "result": response, "original_tickers": &ordermatch_ctx.original_tickers });
    Response::builder()
        .body(json::to_vec(&res).expect("Serialization failed"))
        .map_err(|e| ERRL!("{}", e))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod best_orders_test {
    use super::*;
    use crate::mm2::lp_ordermatch::ordermatch_tests::make_random_orders;
    use crate::mm2::lp_ordermatch::{OrderbookItem, TrieProof};
    use std::iter::FromIterator;

    #[test]
    fn check_best_orders_p2p_res_serde() {
        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
        struct BestOrderV1 {
            pubkey: String,
            base: String,
            rel: String,
            price: BigRational,
            max_volume: BigRational,
            min_volume: BigRational,
            uuid: Uuid,
            created_at: u64,
        }

        impl From<OrderbookItem> for BestOrderV1 {
            fn from(o: OrderbookItem) -> Self {
                BestOrderV1 {
                    pubkey: o.pubkey,
                    base: o.base,
                    rel: o.rel,
                    price: o.price,
                    max_volume: o.max_volume,
                    min_volume: o.min_volume,
                    uuid: o.uuid,
                    created_at: o.created_at,
                }
            }
        }

        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
        struct BestOrderWithProofV1 {
            /// Orderbook item
            order: BestOrderV1,
            /// Last pubkey message payload that contains most recent pair trie root
            last_message_payload: Vec<u8>,
            /// Proof confirming that orderbook item is in the pair trie
            proof: TrieProof,
        }

        #[derive(Debug, Deserialize, PartialEq, Serialize)]
        struct BestOrdersResV1 {
            orders: HashMap<String, Vec<BestOrderWithProofV1>>,
        }

        let orders = make_random_orders("".into(), &[1; 32], "RICK".into(), "MORTY".into(), 10);
        let orders: Vec<_> = orders
            .into_iter()
            .map(|order| BestOrderWithProofV1 {
                order: order.into(),
                last_message_payload: vec![],
                proof: vec![],
            })
            .collect();

        let old = BestOrdersResV1 {
            orders: HashMap::from_iter(std::iter::once(("RICK".into(), orders))),
        };

        let old_serialized = rmp_serde::to_vec(&old).unwrap();

        let mut new: BestOrdersRes = rmp_serde::from_read_ref(&old_serialized).unwrap();
        new.protocol_infos.insert(Uuid::new_v4(), BaseRelProtocolInfo {
            base: vec![1],
            rel: vec![2],
        });

        let new_serialized = rmp_serde::to_vec(&new).unwrap();

        let old_from_new: BestOrdersResV1 = rmp_serde::from_read_ref(&new_serialized).unwrap();
        assert_eq!(old, old_from_new);
    }
}

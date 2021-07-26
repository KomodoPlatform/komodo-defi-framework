use super::{addr_format_from_protocol_info, OrderbookItemWithProof, OrdermatchContext, OrdermatchRequest,
            OrdermatchRequestVersion};
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

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct BestOrdersRes {
    orders: HashMap<String, Vec<OrderbookItemWithProof>>,
}

pub async fn process_best_orders_p2p_request(
    ctx: MmArc,
    coin: String,
    action: BestOrdersAction,
    required_volume: BigRational,
    version: OrdermatchRequestVersion,
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
                    if version == OrdermatchRequestVersion::V1
                        && (o.base_protocol_info.is_some() || o.base_protocol_info.is_some())
                    {
                        log::debug!("Order {} address format is not supported by receiver", o.uuid);
                        continue;
                    }

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
                    best_orders.push(order_w_proof);

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
    if is_wallet_only_ticker(&ctx, &req.coin) {
        return ERR!("Coin {} is wallet only", &req.coin);
    }
    let p2p_request = OrdermatchRequest::BestOrders {
        coin: req.coin,
        action: req.action,
        volume: req.volume.into(),
        version: OrdermatchRequestVersion::V2,
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
                let addr_format = match req.action {
                    BestOrdersAction::Buy => addr_format_from_protocol_info(&order.rel_protocol_info),
                    BestOrdersAction::Sell => addr_format_from_protocol_info(&order.base_protocol_info),
                };
                let address = match address_by_coin_conf_and_pubkey_str(&coin, &coin_conf, &order.pubkey, addr_format) {
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

    let res = json!({ "result": response });
    Response::builder()
        .body(json::to_vec(&res).expect("Serialization failed"))
        .map_err(|e| ERRL!("{}", e))
}

#[cfg(test)]
mod best_orders_tests {
    use common::new_uuid;
    use keys::AddressFormat;
    use rmp_serde::decode::Error;
    use uuid::Uuid;

    use super::super::OrderbookItem;
    use super::*;

    #[test]
    fn check_best_orders_res_deserialize_to_old() {
        type TrieProof = Vec<Vec<u8>>;
        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
        struct OrderbookItemOld {
            pubkey: String,
            base: String,
            rel: String,
            price: BigRational,
            max_volume: BigRational,
            min_volume: BigRational,
            uuid: Uuid,
            created_at: u64,
        }

        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
        struct OrderbookItemWithProofOld {
            order: OrderbookItemOld,
            last_message_payload: Vec<u8>,
            proof: TrieProof,
        }

        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
        struct BestOrdersResOld {
            orders: HashMap<String, Vec<OrderbookItemWithProofOld>>,
        }

        let uuid = new_uuid();

        let mut best_order_res_new_orders = HashMap::new();
        best_order_res_new_orders.insert(String::from("RICK"), vec![OrderbookItemWithProof {
            order: OrderbookItem {
                pubkey: "03c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed".into(),
                base: "tBTC".into(),
                rel: "RICK".into(),
                price: BigRational::from_integer(1.into()),
                max_volume: BigRational::from_integer(2.into()),
                min_volume: BigRational::from_integer(1.into()),
                uuid,
                created_at: 1626959392,
                base_protocol_info: None,
                rel_protocol_info: None,
            },
            last_message_payload: vec![],
            proof: vec![],
        }]);
        let best_order_res_new = BestOrdersRes {
            orders: best_order_res_new_orders,
        };

        let mut best_order_res_old_orders = HashMap::new();
        best_order_res_old_orders.insert(String::from("RICK"), vec![OrderbookItemWithProofOld {
            order: OrderbookItemOld {
                pubkey: "03c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed".into(),
                base: "tBTC".into(),
                rel: "RICK".into(),
                price: BigRational::from_integer(1.into()),
                max_volume: BigRational::from_integer(2.into()),
                min_volume: BigRational::from_integer(1.into()),
                uuid,
                created_at: 1626959392,
            },
            last_message_payload: vec![],
            proof: vec![],
        }]);
        let best_order_res_old = BestOrdersResOld {
            orders: best_order_res_old_orders,
        };

        // new format should be deserialized to old when protocol_infos are None
        let serialized = rmp_serde::to_vec(&best_order_res_new).unwrap();
        let deserialized: BestOrdersResOld = rmp_serde::from_read_ref(serialized.as_slice()).unwrap();
        assert_eq!(best_order_res_old, deserialized);

        // old format should be deserialized to new with protocol_infos as None
        let serialized = rmp_serde::to_vec(&best_order_res_old).unwrap();
        let deserialized: BestOrdersRes = rmp_serde::from_read_ref(serialized.as_slice()).unwrap();
        assert_eq!(best_order_res_new, deserialized);

        let mut best_order_res_new_orders = HashMap::new();
        best_order_res_new_orders.insert(String::from("RICK"), vec![OrderbookItemWithProof {
            order: OrderbookItem {
                pubkey: "03c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed".into(),
                base: "tBTC".into(),
                rel: "RICK".into(),
                price: BigRational::from_integer(1.into()),
                max_volume: BigRational::from_integer(2.into()),
                min_volume: BigRational::from_integer(1.into()),
                uuid,
                created_at: 1626959392,
                base_protocol_info: Some(rmp_serde::to_vec(&AddressFormat::Segwit).unwrap()),
                rel_protocol_info: Some(rmp_serde::to_vec(&AddressFormat::Standard).unwrap()),
            },
            last_message_payload: vec![],
            proof: vec![],
        }]);
        let best_order_res_new = BestOrdersRes {
            orders: best_order_res_new_orders,
        };

        // old format can't be deserialized from new when protocol_infos are present
        let serialized = rmp_serde::to_vec(&best_order_res_new).unwrap();
        let deserialized: Result<BestOrdersResOld, Error> = rmp_serde::from_read_ref(serialized.as_slice());
        assert!(deserialized.is_err());
    }
}

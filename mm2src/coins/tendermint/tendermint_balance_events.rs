use async_trait::async_trait;
use common::{executor::{AbortSettings, SpawnAbortable},
             http_uri_to_ws_address, log};
use futures::channel::oneshot::{self, Receiver, Sender};
use futures_util::{SinkExt, StreamExt};
use jsonrpc_core::{Id as RpcId, Params as RpcParams, Value as RpcValue, Version as RpcVersion};
use mm2_core::mm_ctx::MmArc;
use mm2_event_stream::{behaviour::{EventBehaviour, EventInitStatus},
                       ErrorEventName, Event, EventName, EventStreamConfiguration};
use mm2_net::p2p::Keypair;
use mm2_number::BigDecimal;
use proxy_signature::RawMessage;
use std::collections::{HashMap, HashSet};

use super::TendermintCoin;
use crate::{tendermint::{rpc::PROXY_REQUEST_EXPIRATION_SEC, TendermintCommons},
            utxo::utxo_common::big_decimal_from_sat_unsigned,
            MarketCoinOps, MmCoin};

#[async_trait]
impl EventBehaviour for TendermintCoin {
    fn event_name() -> EventName { EventName::CoinBalance }

    fn error_event_name() -> ErrorEventName { ErrorEventName::CoinBalanceError }

    async fn handle(self, _interval: f64, tx: oneshot::Sender<EventInitStatus>) {
        fn generate_subscription_query(
            query_filter: String,
            proxy_sign_keypair: &Option<Keypair>,
            uri: &http::Uri,
        ) -> String {
            let mut params = serde_json::Map::with_capacity(1);
            params.insert("query".to_owned(), RpcValue::String(query_filter));

            let mut q = json!({
                "id": RpcId::Num(0),
                "jsonrpc": Some(RpcVersion::V2),
                "method": "subscribe".to_owned(),
                "params": RpcParams::Map(params),
            });

            const BODY_SIZE: usize = 0;
            if let Some(proxy_sign_keypair) = proxy_sign_keypair {
                if let Ok(proxy_sign) =
                    RawMessage::sign(proxy_sign_keypair, uri, BODY_SIZE, PROXY_REQUEST_EXPIRATION_SEC)
                {
                    q["proxy_sign"] = serde_json::to_value(proxy_sign).expect("This should never happen");
                }
            };

            serde_json::to_string(&q).expect("This should never happen")
        }

        let ctx = match MmArc::from_weak(&self.ctx) {
            Some(ctx) => ctx,
            None => {
                let msg = "MM context must have been initialized already.";
                tx.send(EventInitStatus::Failed(msg.to_owned()))
                    .expect("Receiver is dropped, which should never happen.");
                panic!("{}", msg);
            },
        };

        let account_id = self.account_id.to_string();
        let mut current_balances: HashMap<String, BigDecimal> = HashMap::new();

        tx.send(EventInitStatus::Success)
            .expect("Receiver is dropped, which should never happen.");

        loop {
            let client = match self.rpc_client().await {
                Ok(client) => client,
                Err(e) => {
                    log::error!("{e}");
                    continue;
                },
            };

            let receiver_q = generate_subscription_query(
                format!("coin_received.receiver = '{}'", account_id),
                client.proxy_sign_keypair(),
                &client.uri(),
            );
            let receiver_q = tokio_tungstenite_wasm::Message::Text(receiver_q);

            let spender_q = generate_subscription_query(
                format!("coin_spent.spender = '{}'", account_id),
                client.proxy_sign_keypair(),
                &client.uri(),
            );
            let spender_q = tokio_tungstenite_wasm::Message::Text(spender_q);

            let socket_address = format!("{}/{}", http_uri_to_ws_address(client.uri()), "websocket");

            let mut wsocket = match tokio_tungstenite_wasm::connect(&socket_address).await {
                Ok(ws) => ws,
                Err(e) => {
                    log::error!("Couldn't connect to '{socket_address}': {e}");
                    continue;
                },
            };

            // Filter received TX events
            if let Err(e) = wsocket.send(receiver_q.clone()).await {
                log::error!("{e}");
                continue;
            }

            // Filter spent TX events
            if let Err(e) = wsocket.send(spender_q.clone()).await {
                log::error!("{e}");
                continue;
            }

            while let Some(message) = wsocket.next().await {
                let msg = match message {
                    Ok(tokio_tungstenite_wasm::Message::Text(data)) => data.clone(),
                    Ok(tokio_tungstenite_wasm::Message::Close(_)) => break,
                    Err(err) => {
                        log::error!("Server returned an unknown message type - {err}");
                        break;
                    },
                    _ => continue,
                };

                // Here, we receive raw data from the socket.
                // To examine this data, you can use tools like wscat/websocat or visit
                // https://pastebin.pl/view/499cbf2c for sample data.
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&msg) {
                    let transfers: Vec<String> =
                        serde_json::from_value(json_val["result"]["events"]["transfer.amount"].clone())
                            .unwrap_or_default();

                    let denoms: HashSet<String> = transfers
                        .iter()
                        .map(|t| {
                            let amount: String = t.chars().take_while(|c| c.is_numeric()).collect();
                            let denom = &t[amount.len()..];
                            denom.to_owned()
                        })
                        .collect();

                    let mut balance_updates = vec![];
                    for denom in denoms {
                        if let Some((ticker, decimals)) = self.active_ticker_and_decimals_from_denom(&denom) {
                            let balance_denom = match self.account_balance_for_denom(&self.account_id, denom).await {
                                Ok(balance_denom) => balance_denom,
                                Err(e) => {
                                    log::error!("Failed getting balance for '{ticker}'. Error: {e}");
                                    let e = serde_json::to_value(e).expect("Serialization should't fail.");
                                    ctx.stream_channel_controller
                                        .broadcast(Event::new(
                                            format!("{}:{}", Self::error_event_name(), ticker),
                                            e.to_string(),
                                        ))
                                        .await;

                                    continue;
                                },
                            };

                            let balance_decimal = big_decimal_from_sat_unsigned(balance_denom, decimals);

                            // Only broadcast when balance is changed
                            let mut broadcast = false;
                            if let Some(balance) = current_balances.get_mut(&ticker) {
                                if *balance != balance_decimal {
                                    *balance = balance_decimal.clone();
                                    broadcast = true;
                                }
                            } else {
                                current_balances.insert(ticker.clone(), balance_decimal.clone());
                                broadcast = true;
                            }

                            if broadcast {
                                balance_updates.push(json!({
                                    "ticker": ticker,
                                    "balance": { "spendable": balance_decimal, "unspendable": BigDecimal::default() }
                                }));
                            }
                        }
                    }

                    if !balance_updates.is_empty() {
                        ctx.stream_channel_controller
                            .broadcast(Event::new(
                                Self::event_name().to_string(),
                                json!(balance_updates).to_string(),
                            ))
                            .await;
                    }
                }
            }
        }
    }

    async fn spawn_if_active(self, config: &EventStreamConfiguration) -> EventInitStatus {
        if let Some(event) = config.get_event(&Self::event_name()) {
            log::info!(
                "{} event is activated for {}. `stream_interval_seconds`({}) has no effect on this.",
                Self::event_name(),
                self.ticker(),
                event.stream_interval_seconds
            );

            let (tx, rx): (Sender<EventInitStatus>, Receiver<EventInitStatus>) = oneshot::channel();
            let fut = self.clone().handle(event.stream_interval_seconds, tx);
            let settings = AbortSettings::info_on_abort(format!(
                "{} event is stopped for {}.",
                Self::event_name(),
                self.ticker()
            ));
            self.spawner().spawn_with_settings(fut, settings);

            rx.await.unwrap_or_else(|e| {
                EventInitStatus::Failed(format!("Event initialization status must be received: {}", e))
            })
        } else {
            EventInitStatus::Inactive
        }
    }
}

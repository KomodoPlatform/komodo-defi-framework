use super::utxo_standard::UtxoStandardCoin;
use crate::utxo::rpc_clients::UtxoRpcClientEnum;
use crate::{utxo::{output_script,
                   rpc_clients::electrum_script_hash,
                   utxo_common::{address_balance, address_to_scripthash},
                   ScripthashNotification, UtxoCoinFields},
            CoinWithDerivationMethod, MarketCoinOps, MmCoin};
use async_trait::async_trait;
use common::{executor::{AbortSettings, SpawnAbortable},
             log};
use futures::channel::oneshot::{self, Receiver, Sender};
use futures_util::StreamExt;
use keys::Address;
use mm2_core::mm_ctx::MmArc;
use mm2_event_stream::{behaviour::{EventBehaviour, EventInitStatus},
                       ErrorEventName, Event, EventName, EventStreamConfiguration};
use std::collections::{BTreeMap, HashMap, HashSet};

macro_rules! try_or_continue {
    ($exp:expr) => {
        match $exp {
            Ok(t) => t,
            Err(e) => {
                log::error!("{}", e);
                continue;
            },
        }
    };
}

#[async_trait]
impl EventBehaviour for UtxoStandardCoin {
    fn event_name() -> EventName { EventName::CoinBalance }

    fn error_event_name() -> ErrorEventName { ErrorEventName::CoinBalanceError }

    async fn handle(self, _interval: f64, tx: oneshot::Sender<EventInitStatus>) {
        const RECEIVER_DROPPED_MSG: &str = "Receiver is dropped, which should never happen.";

        async fn subscribe_to_addresses(
            utxo: &UtxoCoinFields,
            addresses: HashSet<Address>,
        ) -> Result<BTreeMap<String, Address>, String> {
            match utxo.rpc_client.clone() {
                UtxoRpcClientEnum::Electrum(client) => {
                    // Collect the scrpithash for every address into a map.
                    let scripthash_to_address_map = addresses
                        .into_iter()
                        .map(|address| {
                            let scripthash = address_to_scripthash(&address).map_err(|e| e.to_string())?;
                            Ok((scripthash, address))
                        })
                        .collect::<Result<HashMap<String, Address>, String>>()?;
                    // Add these subscriptions to the connection manager. It will choose whatever connections
                    // it sees fit to subscribe each of these addresses to.
                    client
                        .connection_manager
                        .add_subscriptions(&scripthash_to_address_map)
                        .await;
                    // Convert the hashmap back to btreemap.
                    Ok(scripthash_to_address_map.into_iter().map(|(k, v)| (k, v)).collect())
                },
                UtxoRpcClientEnum::Native(_) => {
                    Err("Balance streaming is currently not supported for native client.".to_owned())
                },
            }
        }

        let ctx = match MmArc::from_weak(&self.as_ref().ctx) {
            Some(ctx) => ctx,
            None => {
                let msg = "MM context must have been initialized already.";
                tx.send(EventInitStatus::Failed(msg.to_owned()))
                    .expect(RECEIVER_DROPPED_MSG);
                panic!("{}", msg);
            },
        };

        let scripthash_notification_handler = match self.as_ref().scripthash_notification_handler.as_ref() {
            Some(t) => t,
            None => {
                let e = "Scripthash notification receiver can not be empty.";
                tx.send(EventInitStatus::Failed(e.to_string()))
                    .expect(RECEIVER_DROPPED_MSG);
                panic!("{}", e);
            },
        };

        tx.send(EventInitStatus::Success).expect(RECEIVER_DROPPED_MSG);

        let mut scripthash_to_address_map = BTreeMap::default();
        while let Some(message) = scripthash_notification_handler.lock().await.next().await {
            let notified_scripthash = match message {
                ScripthashNotification::Triggered(t) => t,
                ScripthashNotification::SubscribeToAddresses(addresses) => {
                    match subscribe_to_addresses(self.as_ref(), addresses).await {
                        Ok(map) => scripthash_to_address_map.extend(map),
                        Err(e) => {
                            log::error!("{e}");

                            ctx.stream_channel_controller
                                .broadcast(Event::new(
                                    format!("{}:{}", Self::error_event_name(), self.ticker()),
                                    json!({ "error": e }).to_string(),
                                ))
                                .await;
                        },
                    };

                    continue;
                },
            };

            let address = match scripthash_to_address_map.get(&notified_scripthash) {
                Some(t) => Some(t.clone()),
                None => try_or_continue!(self.all_addresses().await)
                    .into_iter()
                    .find_map(|addr| {
                        let script = match output_script(&addr) {
                            Ok(script) => script,
                            Err(e) => {
                                log::error!("{e}");
                                return None;
                            },
                        };
                        let script_hash = electrum_script_hash(&script);
                        let scripthash = hex::encode(script_hash);

                        if notified_scripthash == scripthash {
                            scripthash_to_address_map.insert(notified_scripthash.clone(), addr.clone());
                            Some(addr)
                        } else {
                            None
                        }
                    }),
            };

            let address = match address {
                Some(t) => t,
                None => {
                    log::debug!(
                        "Couldn't find the relevant address for {} scripthash.",
                        notified_scripthash
                    );
                    continue;
                },
            };

            let balance = match address_balance(&self, &address).await {
                Ok(t) => t,
                Err(e) => {
                    let ticker = self.ticker();
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

            let payload = json!({
                "ticker": self.ticker(),
                "address": address.to_string(),
                "balance": { "spendable": balance.spendable, "unspendable": balance.unspendable }
            });

            ctx.stream_channel_controller
                .broadcast(Event::new(
                    Self::event_name().to_string(),
                    json!(vec![payload]).to_string(),
                ))
                .await;
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

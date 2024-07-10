use async_trait::async_trait;
use common::{executor::{AbortSettings, SpawnAbortable, Timer},
             log, Future01CompatExt};
use futures::channel::oneshot::{self, Receiver, Sender};
use futures_util::StreamExt;
use keys::Address;
use mm2_core::mm_ctx::MmArc;
use mm2_event_stream::{behaviour::{EventBehaviour, EventInitStatus},
                       ErrorEventName, Event, EventName, EventStreamConfiguration};
use std::collections::{BTreeMap, HashSet};

use super::{utxo_standard::UtxoStandardCoin, UtxoArc};
use crate::{utxo::{output_script,
                   rpc_clients::electrum_script_hash,
                   utxo_common::{address_balance, address_to_scripthash},
                   ScripthashNotification, UtxoCoinFields},
            CoinWithDerivationMethod, MarketCoinOps, MmCoin};

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

pub struct UtxoBalanceEventStreamer {
    coin: UtxoStandardCoin,
}

impl UtxoBalanceEventStreamer {
    pub fn new(utxo_arc: UtxoArc) -> Self {
        Self {
            // We wrap the UtxoArc in a UtxoStandardCoin for easier method accessibility.
            // The UtxoArc might belong to a different coin type though.
            coin: UtxoStandardCoin::from(utxo_arc),
        }
    }
}

#[async_trait]
impl EventBehaviour for UtxoBalanceEventStreamer {
    fn event_name() -> EventName { EventName::CoinBalance }

    fn error_event_name() -> ErrorEventName { ErrorEventName::CoinBalanceError }

    // FIXME: Move `interval` to `self`.
    async fn handle(self, _interval: f64, tx: oneshot::Sender<EventInitStatus>) {
        const RECEIVER_DROPPED_MSG: &str = "Receiver is dropped, which should never happen.";
        let coin = self.coin;

        async fn subscribe_to_addresses(
            utxo: &UtxoCoinFields,
            addresses: HashSet<Address>,
        ) -> Result<BTreeMap<String, Address>, String> {
            const LOOP_INTERVAL: f64 = 0.5;

            let mut scripthash_to_address_map: BTreeMap<String, Address> = BTreeMap::new();
            for address in addresses {
                let scripthash = address_to_scripthash(&address).map_err(|e| e.to_string())?;

                scripthash_to_address_map.insert(scripthash.clone(), address);

                let mut attempt = 0;
                while let Err(e) = utxo
                    .rpc_client
                    .blockchain_scripthash_subscribe(scripthash.clone())
                    .compat()
                    .await
                {
                    if attempt == 5 {
                        return Err(e.to_string());
                    }

                    log::error!(
                        "Failed to subscribe {} scripthash ({attempt}/5 attempt). Error: {}",
                        scripthash,
                        e.to_string()
                    );

                    attempt += 1;
                    Timer::sleep(LOOP_INTERVAL).await;
                }
            }

            Ok(scripthash_to_address_map)
        }

        let ctx = match MmArc::from_weak(&coin.as_ref().ctx) {
            Some(ctx) => ctx,
            None => {
                let msg = "MM context must have been initialized already.";
                tx.send(EventInitStatus::Failed(msg.to_owned()))
                    .expect(RECEIVER_DROPPED_MSG);
                panic!("{}", msg);
            },
        };

        let scripthash_notification_handler = match coin.as_ref().scripthash_notification_handler.as_ref() {
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
                    match subscribe_to_addresses(coin.as_ref(), addresses).await {
                        Ok(map) => scripthash_to_address_map.extend(map),
                        Err(e) => {
                            log::error!("{e}");

                            ctx.stream_channel_controller
                                .broadcast(Event::new(
                                    format!("{}:{}", Self::error_event_name(), coin.ticker()),
                                    json!({ "error": e }).to_string(),
                                ))
                                .await;
                        },
                    };

                    continue;
                },
                ScripthashNotification::RefreshSubscriptions => {
                    let my_addresses = try_or_continue!(coin.all_addresses().await);
                    match subscribe_to_addresses(coin.as_ref(), my_addresses).await {
                        Ok(map) => scripthash_to_address_map = map,
                        Err(e) => {
                            log::error!("{e}");

                            ctx.stream_channel_controller
                                .broadcast(Event::new(
                                    format!("{}:{}", Self::error_event_name(), coin.ticker()),
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
                None => try_or_continue!(coin.all_addresses().await)
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

            let balance = match address_balance(&coin, &address).await {
                Ok(t) => t,
                Err(e) => {
                    let ticker = coin.ticker();
                    log::error!("Failed getting balance for '{ticker}'. Error: {e}");
                    let e = serde_json::to_value(e).expect("Serialization should't fail.");

                    // FIXME: Note that such an event isn't SSE-ed to any client since no body is listening to it.
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
                "ticker": coin.ticker(),
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
                self.coin.ticker(),
                event.stream_interval_seconds
            );

            let (tx, rx): (Sender<EventInitStatus>, Receiver<EventInitStatus>) = oneshot::channel();
            let settings = AbortSettings::info_on_abort(format!(
                "{} event is stopped for {}.",
                Self::event_name(),
                self.coin.ticker()
            ));
            self.coin
                .spawner()
                .spawn_with_settings(self.handle(event.stream_interval_seconds, tx), settings);

            rx.await.unwrap_or_else(|e| {
                EventInitStatus::Failed(format!("Event initialization status must be received: {}", e))
            })
        } else {
            EventInitStatus::Inactive
        }
    }
}

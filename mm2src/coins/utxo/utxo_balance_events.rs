use async_trait::async_trait;
use common::{executor::Timer, log, Future01CompatExt};
use futures::channel::oneshot;
use futures::StreamExt;
use keys::Address;
use mm2_event_stream::{Broadcaster, Event, EventStreamer, StreamHandlerInput};
use std::collections::{BTreeMap, HashSet};

use super::{utxo_standard::UtxoStandardCoin, UtxoArc};
use crate::{utxo::{output_script,
                   rpc_clients::electrum_script_hash,
                   utxo_common::{address_balance, address_to_scripthash},
                   ScripthashNotification, UtxoCoinFields},
            CoinWithDerivationMethod, MarketCoinOps};

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
impl EventStreamer for UtxoBalanceEventStreamer {
    type DataInType = ScripthashNotification;

    fn streamer_id(&self) -> String { format!("BALANCE:{}", self.coin.ticker()) }

    async fn handle(
        self,
        broadcaster: Broadcaster,
        ready_tx: oneshot::Sender<Result<(), String>>,
        mut data_rx: impl StreamHandlerInput<Self::DataInType>,
    ) {
        const RECEIVER_DROPPED_MSG: &str = "Receiver is dropped, which should never happen.";
        let streamer_id = self.streamer_id();
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

        if coin.as_ref().rpc_client.is_native() {
            let msg = "Native RPC client is not supported for UtxoBalanceEventStreamer.";
            ready_tx.send(Err(msg.to_string())).expect(RECEIVER_DROPPED_MSG);
            panic!("{}", msg);
        }

        ready_tx.send(Ok(())).expect(RECEIVER_DROPPED_MSG);

        let mut scripthash_to_address_map = BTreeMap::default();
        while let Some(message) = data_rx.next().await {
            let notified_scripthash = match message {
                ScripthashNotification::Triggered(t) => t,
                ScripthashNotification::SubscribeToAddresses(addresses) => {
                    match subscribe_to_addresses(coin.as_ref(), addresses).await {
                        Ok(map) => scripthash_to_address_map.extend(map),
                        Err(e) => {
                            log::error!("{e}");

                            broadcaster.broadcast(Event::err(streamer_id.clone(), json!({ "error": e })));
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

                            broadcaster.broadcast(Event::err(streamer_id.clone(), json!({ "error": e })));
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

                    broadcaster.broadcast(Event::err(streamer_id.clone(), e));

                    continue;
                },
            };

            let payload = json!({
                "ticker": coin.ticker(),
                "address": address.to_string(),
                "balance": { "spendable": balance.spendable, "unspendable": balance.unspendable }
            });

            broadcaster.broadcast(Event::new(streamer_id.clone(), json!(vec![payload])));
        }
    }
}

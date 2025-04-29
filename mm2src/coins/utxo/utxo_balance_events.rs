use super::{utxo_standard::UtxoStandardCoin, UtxoArc};

use crate::utxo::rpc_clients::UtxoRpcClientEnum;
use crate::{utxo::{output_script,
                   rpc_clients::electrum_script_hash,
                   utxo_common::{address_balance, address_to_scripthash},
                   ScripthashNotification, UtxoCoinFields},
            CoinWithDerivationMethod, MarketCoinOps};

use async_trait::async_trait;
use common::log;
use futures::channel::oneshot;
use futures::StreamExt;
use keys::Address;
use mm2_event_stream::{Broadcaster, Event, EventStreamer, StreamHandlerInput};
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

    pub fn derive_streamer_id(coin: &str) -> String { format!("BALANCE:{coin}") }
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
        let mut scripthash_to_address_map = BTreeMap::default();

        // Get all the addresses to subscribe to their balance updates.
        let all_addresses = match coin.all_addresses().await {
            Ok(addresses) => addresses,
            Err(e) => {
                let msg = format!("Failed to get all addresses: {e}");
                ready_tx.send(Err(msg.clone())).expect(RECEIVER_DROPPED_MSG);
                panic!("{}", msg);
            },
        };
        // FIXME: This might take some good LONG time in an HD wallet with many addresses.
        //        We better optimistically respond to the `ready_tx` to avoid blocking the enabler response to the RPC.
        match subscribe_to_addresses(coin.as_ref(), all_addresses).await {
            Ok(initial_tracking_list) => scripthash_to_address_map.extend(initial_tracking_list),
            Err(e) => {
                let msg = format!("Failed to subscribe to balance events: {e}");
                ready_tx.send(Err(msg.clone())).expect(RECEIVER_DROPPED_MSG);
                panic!("{}", msg);
            },
        }
        ready_tx.send(Ok(())).expect(RECEIVER_DROPPED_MSG);

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

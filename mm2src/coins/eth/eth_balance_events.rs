use async_trait::async_trait;
use common::{executor::{AbortSettings, SpawnAbortable, Timer},
             log, Future01CompatExt};
use ethereum_types::Address;
use futures::{channel::oneshot::{self, Receiver, Sender},
              stream::FuturesUnordered,
              StreamExt};
use instant::Instant;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;
use mm2_event_stream::{behaviour::{EventBehaviour, EventInitStatus},
                       ErrorEventName, Event, EventName, EventStreamConfiguration};
use mm2_number::BigDecimal;
use std::collections::{HashMap, HashSet};

use super::EthCoin;
use crate::{eth::{u256_to_big_decimal, Erc20TokenInfo},
            BalanceError, CoinWithDerivationMethod, MmCoin};

struct BalanceData {
    ticker: String,
    address: String,
    balance: BigDecimal,
}

struct BalanceFetchError {
    ticker: String,
    address: String,
    error: MmError<BalanceError>,
}

type BalanceResult = Result<BalanceData, BalanceFetchError>;

/// This implementation differs from others, as they immediately return
/// an error if any of the requests fails. This one completes all futures
/// and returns their results individually.
async fn get_all_balance_results_concurrently(coin: &EthCoin, addresses: HashSet<Address>) -> Vec<BalanceResult> {
    let mut tokens = coin.get_erc_tokens_infos();
    // Workaround for performance purposes.
    //
    // Unlike tokens, the platform coin length is constant (=1). Instead of creating a generic
    // type and mapping the platform coin and the entire token list (which can grow at any time), we map
    // the platform coin to Erc20TokenInfo so that we can use the token list right away without
    // additional mapping.
    tokens.insert(coin.ticker.clone(), Erc20TokenInfo {
        // This is a dummy value, since there is no token address for the platform coin.
        // In the fetch_balance function, we check if the token_ticker is equal to this
        // coin's ticker to avoid using token_address to fetch the balance
        // and to use address_balance instead.
        token_address: Address::default(),
        decimals: coin.decimals,
    });
    drop_mutability!(tokens);

    let mut all_jobs = FuturesUnordered::new();

    for address in addresses {
        let jobs = tokens.iter().map(|(token_ticker, info)| {
            let coin = coin.clone();
            let token_ticker = token_ticker.clone();
            let info = info.clone();
            async move { fetch_balance(&coin, address, token_ticker, &info).await }
        });

        all_jobs.extend(jobs);
    }

    all_jobs.collect().await
}

async fn fetch_balance(
    coin: &EthCoin,
    address: Address,
    token_ticker: String,
    info: &Erc20TokenInfo,
) -> Result<BalanceData, BalanceFetchError> {
    let (balance_as_u256, decimals) = if token_ticker == coin.ticker {
        (
            coin.address_balance(address)
                .compat()
                .await
                .map_err(|error| BalanceFetchError {
                    ticker: token_ticker.clone(),
                    address: address.to_string(),
                    error,
                })?,
            coin.decimals,
        )
    } else {
        (
            coin.get_token_balance(info.token_address)
                .await
                .map_err(|error| BalanceFetchError {
                    ticker: token_ticker.clone(),
                    address: address.to_string(),
                    error,
                })?,
            info.decimals,
        )
    };

    let balance_as_big_decimal = u256_to_big_decimal(balance_as_u256, decimals).map_err(|e| BalanceFetchError {
        ticker: token_ticker.clone(),
        address: address.to_string(),
        error: e.into(),
    })?;

    Ok(BalanceData {
        ticker: token_ticker,
        address: address.to_string(),
        balance: balance_as_big_decimal,
    })
}

#[async_trait]
impl EventBehaviour for EthCoin {
    fn event_name() -> EventName { EventName::CoinBalance }

    fn error_event_name() -> ErrorEventName { ErrorEventName::CoinBalanceError }

    async fn handle(self, interval: f64, tx: oneshot::Sender<EventInitStatus>) {
        const RECEIVER_DROPPED_MSG: &str = "Receiver is dropped, which should never happen.";

        async fn start_polling(coin: EthCoin, ctx: MmArc, interval: f64) {
            async fn sleep_remaining_time(interval: f64, now: Instant) {
                // If the interval is x seconds,
                // our goal is to broadcast changed balances every x seconds.
                // To achieve this, we need to subtract the time complexity of each iteration.
                // Given that an iteration already takes 80% of the interval,
                // this will lead to inconsistency in the events.
                let remaining_time = interval - now.elapsed().as_secs_f64();
                // Not worth to make a call for less than `0.1` durations
                if remaining_time >= 0.1 {
                    Timer::sleep(remaining_time).await;
                }
            }

            let mut cache: HashMap<String, HashMap<String, BigDecimal>> = HashMap::new();

            loop {
                let now = Instant::now();

                let addresses = match coin.all_addresses().await {
                    Ok(addresses) => addresses,
                    Err(e) => {
                        log::error!("Failed getting addresses for {}. Error: {}", coin.ticker, e);
                        let e = serde_json::to_value(e).expect("Serialization shouldn't fail.");
                        ctx.stream_channel_controller
                            .broadcast(Event::new(
                                format!("{}:{}", EthCoin::error_event_name(), coin.ticker),
                                e.to_string(),
                            ))
                            .await;
                        sleep_remaining_time(interval, now).await;
                        continue;
                    },
                };

                let mut balance_updates = vec![];
                for result in get_all_balance_results_concurrently(&coin, addresses).await {
                    match result {
                        Ok(res) => {
                            if Some(&res.balance) == cache.get(&res.ticker).and_then(|map| map.get(&res.address)) {
                                continue;
                            }

                            balance_updates.push(json!({
                                "ticker": res.ticker,
                                "address": res.address,
                                "balance": { "spendable": res.balance, "unspendable": BigDecimal::default() }
                            }));
                            cache
                                .entry(res.ticker.clone())
                                .or_insert_with(HashMap::new)
                                .insert(res.address, res.balance);
                        },
                        Err(err) => {
                            log::error!(
                                "Failed getting balance for '{}:{}' with {interval} interval. Error: {}",
                                err.ticker,
                                err.address,
                                err.error
                            );
                            let e = serde_json::to_value(err.error).expect("Serialization shouldn't fail.");
                            ctx.stream_channel_controller
                                .broadcast(Event::new(
                                    format!("{}:{}:{}", EthCoin::error_event_name(), err.ticker, err.address),
                                    e.to_string(),
                                ))
                                .await;
                        },
                    };
                }

                if !balance_updates.is_empty() {
                    ctx.stream_channel_controller
                        .broadcast(Event::new(
                            EthCoin::event_name().to_string(),
                            json!(balance_updates).to_string(),
                        ))
                        .await;
                }

                sleep_remaining_time(interval, now).await;
            }
        }

        let ctx = match MmArc::from_weak(&self.ctx) {
            Some(ctx) => ctx,
            None => {
                let msg = "MM context must have been initialized already.";
                tx.send(EventInitStatus::Failed(msg.to_owned()))
                    .expect(RECEIVER_DROPPED_MSG);
                panic!("{}", msg);
            },
        };

        tx.send(EventInitStatus::Success).expect(RECEIVER_DROPPED_MSG);

        start_polling(self, ctx, interval).await
    }

    async fn spawn_if_active(self, config: &EventStreamConfiguration) -> EventInitStatus {
        if let Some(event) = config.get_event(&Self::event_name()) {
            log::info!("{} event is activated for {}", Self::event_name(), self.ticker,);

            let (tx, rx): (Sender<EventInitStatus>, Receiver<EventInitStatus>) = oneshot::channel();
            let fut = self.clone().handle(event.stream_interval_seconds, tx);
            let settings =
                AbortSettings::info_on_abort(format!("{} event is stopped for {}.", Self::event_name(), self.ticker));
            self.spawner().spawn_with_settings(fut, settings);

            rx.await.unwrap_or_else(|e| {
                EventInitStatus::Failed(format!("Event initialization status must be received: {}", e))
            })
        } else {
            EventInitStatus::Inactive
        }
    }
}

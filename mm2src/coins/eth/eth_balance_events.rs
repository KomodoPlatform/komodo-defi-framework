use async_trait::async_trait;
use common::{executor::{AbortSettings, SpawnAbortable, Timer},
             log, Future01CompatExt};
use futures::{channel::oneshot::{self, Receiver, Sender},
              stream::FuturesUnordered,
              StreamExt};
use instant::Instant;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;
use mm2_event_stream::{behaviour::{EventBehaviour, EventInitStatus},
                       Event, EventStreamConfiguration};
use mm2_number::BigDecimal;
use std::collections::HashMap;

use super::EthCoin;
use crate::{eth::{u256_to_big_decimal, Erc20TokenInfo},
            BalanceError, MmCoin};

/// This implementation differs from others, as they immediately return
/// an error if any of the requests fails. This one completes all futures
/// and returns their results individually.
async fn get_all_balance_results_concurrently(
    coin: &EthCoin,
) -> Vec<Result<(String, BigDecimal), (String, MmError<BalanceError>)>> {
    let mut tokens = coin.get_erc_tokens_infos();

    // Workaround for performance purposes.
    //
    // Unlike tokens, the platform coin length is constant (=1). Instead of creating a generic
    // type and mapping the platform coin and the entire token list (which can grow at any time), we map
    // the platform coin to Erc20TokenInfo so that we can use the token list right away without
    // additional mapping.
    tokens.insert(coin.ticker.clone(), Erc20TokenInfo {
        token_address: coin.my_address,
        decimals: coin.decimals,
    });

    let jobs = tokens
        .into_iter()
        .map(|(token_ticker, info)| async move { fetch_balance(coin, token_ticker, &info).await })
        .collect::<FuturesUnordered<_>>();

    jobs.collect().await
}

async fn fetch_balance(
    coin: &EthCoin,
    token_ticker: String,
    info: &Erc20TokenInfo,
) -> Result<(String, BigDecimal), (String, MmError<BalanceError>)> {
    let (balance_as_u256, decimals) = if token_ticker == coin.ticker {
        (
            coin.address_balance(coin.my_address)
                .compat()
                .await
                .map_err(|e| (token_ticker.clone(), e))?,
            coin.decimals,
        )
    } else {
        (
            coin.get_token_balance_by_address(info.token_address)
                .await
                .map_err(|e| (token_ticker.clone(), e))?,
            info.decimals,
        )
    };

    let balance_as_big_decimal =
        u256_to_big_decimal(balance_as_u256, decimals).map_err(|e| (token_ticker.clone(), e.into()))?;

    Ok((token_ticker, balance_as_big_decimal))
}

#[async_trait]
impl EventBehaviour for EthCoin {
    const EVENT_NAME: &'static str = "COIN_BALANCE";
    const ERROR_EVENT_NAME: &'static str = "COIN_BALANCE_ERROR";

    async fn handle(self, interval: f64, tx: oneshot::Sender<EventInitStatus>) {
        const RECEIVER_DROPPED_MSG: &str = "Receiver is dropped, which should never happen.";

        async fn with_socket(_coin: EthCoin, _ctx: MmArc) { todo!() }

        async fn with_polling(coin: EthCoin, ctx: MmArc, interval: f64) {
            let mut cache: HashMap<String, BigDecimal> = HashMap::new();

            loop {
                let now = Instant::now();

                let mut balance_updates = vec![];
                for result in get_all_balance_results_concurrently(&coin).await {
                    match result {
                        Ok((ticker, balance)) => {
                            if Some(&balance) == cache.get(&ticker) {
                                continue;
                            }

                            balance_updates.push(json!({
                                "ticker": ticker,
                                "balance": { "spendable": balance, "unspendable": BigDecimal::default() }
                            }));
                            cache.insert(ticker.to_owned(), balance);
                        },
                        Err((ticker, e)) => {
                            log::error!("Failed getting balance for '{ticker}' with {interval} interval. Error: {e}");
                            let e = serde_json::to_value(e).expect("Serialization should't fail.");
                            ctx.stream_channel_controller
                                .broadcast(Event::new(
                                    format!("{}:{}", EthCoin::ERROR_EVENT_NAME, ticker),
                                    e.to_string(),
                                ))
                                .await;
                        },
                    };
                }

                if !balance_updates.is_empty() {
                    ctx.stream_channel_controller
                        .broadcast(Event::new(
                            EthCoin::EVENT_NAME.to_string(),
                            json!(balance_updates).to_string(),
                        ))
                        .await;
                }

                // If the interval is x seconds, our goal is to broadcast changed balances every x seconds.
                // To achieve this, we need to subtract the time complexity of each iteration.
                // Given that an iteration already takes 80% of the interval, this will lead to inconsistency
                // in the events.
                let remaining_time = interval - now.elapsed().as_secs_f64();
                // Not worth to make a call for less than `0.1` durations
                if remaining_time >= 0.1 {
                    Timer::sleep(remaining_time).await;
                }
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

        with_polling(self, ctx, interval).await
    }

    async fn spawn_if_active(self, config: &EventStreamConfiguration) -> EventInitStatus {
        if let Some(event) = config.get_event(Self::EVENT_NAME) {
            log::info!("{} event is activated for {}", Self::EVENT_NAME, self.ticker,);

            let (tx, rx): (Sender<EventInitStatus>, Receiver<EventInitStatus>) = oneshot::channel();
            let fut = self.clone().handle(event.stream_interval_seconds, tx);
            let settings =
                AbortSettings::info_on_abort(format!("{} event is stopped for {}.", Self::EVENT_NAME, self.ticker));
            self.spawner().spawn_with_settings(fut, settings);

            rx.await.unwrap_or_else(|e| {
                EventInitStatus::Failed(format!("Event initialization status must be received: {}", e))
            })
        } else {
            EventInitStatus::Inactive
        }
    }
}

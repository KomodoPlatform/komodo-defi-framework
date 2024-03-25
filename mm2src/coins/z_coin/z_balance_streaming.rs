use crate::common::Future01CompatExt;
use crate::hd_wallet::AsyncMutex;
use crate::z_coin::ZCoin;
use crate::{MarketCoinOps, MmCoin};

use async_trait::async_trait;
use common::executor::{AbortSettings, SpawnAbortable};
use common::log::{error, info};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use futures::channel::oneshot::{Receiver, Sender};
use futures_util::StreamExt;
use mm2_core::mm_ctx::MmArc;
use mm2_event_stream::behaviour::{EventBehaviour, EventInitStatus};
use mm2_event_stream::{Event, EventStreamConfiguration};
use std::sync::Arc;

pub type ZBalanceEventSender = UnboundedSender<()>;
pub type ZBalanceEventHandler = Arc<AsyncMutex<UnboundedReceiver<()>>>;

#[async_trait]
impl EventBehaviour for ZCoin {
    const EVENT_NAME: &'static str = "COIN_BALANCE";
    const ERROR_EVENT_NAME: &'static str = "COIN_BALANCE_ERROR";

    async fn handle(self, _interval: f64, tx: Sender<EventInitStatus>) {
        const RECEIVER_DROPPED_MSG: &str = "Receiver is dropped, which should never happen.";

        macro_rules! send_status_on_err {
            ($match: expr, $sender: tt, $msg: literal) => {
                match $match {
                    Some(t) => t,
                    None => {
                        $sender
                            .send(EventInitStatus::Failed($msg.to_owned()))
                            .expect(RECEIVER_DROPPED_MSG);
                        panic!("{}", $msg);
                    },
                }
            };
        }

        let ctx = send_status_on_err!(
            MmArc::from_weak(&self.as_ref().ctx),
            tx,
            "MM context must have been initialized already."
        );
        let z_balance_change_handler = send_status_on_err!(
            self.z_fields.z_balance_event_handler.as_ref(),
            tx,
            "Z balance change receiver can not be empty."
        );

        tx.send(EventInitStatus::Success).expect(RECEIVER_DROPPED_MSG);

        // Locks the balance change handler, iterates through received events, and updates balance changes accordingly.
        let mut bal = z_balance_change_handler.lock().await;
        while (bal.next().await).is_some() {
            match self.my_balance().compat().await {
                Ok(balance) => {
                    let payload = json!({
                        "ticker": self.ticker(),
                        "address": self.my_z_address_encoded(),
                        "balance": { "spendable": balance.spendable, "unspendable": balance.unspendable }
                    });

                    ctx.stream_channel_controller
                        .broadcast(Event::new(Self::EVENT_NAME.to_string(), payload.to_string()))
                        .await;
                },
                Err(err) => {
                    let ticker = self.ticker();
                    error!("Failed getting balance for '{ticker}'. Error: {err}");
                    let e = serde_json::to_value(err).expect("Serialization should't fail.");
                    return ctx
                        .stream_channel_controller
                        .broadcast(Event::new(
                            format!("{}:{}", Self::ERROR_EVENT_NAME, ticker),
                            e.to_string(),
                        ))
                        .await;
                },
            };
        }
    }

    async fn spawn_if_active(self, config: &EventStreamConfiguration) -> EventInitStatus {
        if let Some(event) = config.get_event(Self::EVENT_NAME) {
            info!(
                "{} event is activated for {} address {}. `stream_interval_seconds`({}) has no effect on this.",
                Self::EVENT_NAME,
                self.ticker(),
                self.my_z_address_encoded(),
                event.stream_interval_seconds
            );

            let (tx, rx): (Sender<EventInitStatus>, Receiver<EventInitStatus>) = oneshot::channel();
            let fut = self.clone().handle(event.stream_interval_seconds, tx);
            let settings =
                AbortSettings::info_on_abort(format!("{} event is stopped for {}.", Self::EVENT_NAME, self.ticker()));
            self.spawner().spawn_with_settings(fut, settings);

            rx.await.unwrap_or_else(|e| {
                EventInitStatus::Failed(format!("Event initialization status must be received: {}", e))
            })
        } else {
            EventInitStatus::Inactive
        }
    }
}

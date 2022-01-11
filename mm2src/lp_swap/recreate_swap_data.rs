use crate::mm2::lp_swap::maker_swap::{MakerSwapData, MakerSwapEvent, TakerNegotiationData, MAKER_ERROR_EVENTS,
                                      MAKER_SUCCESS_EVENTS};
use crate::mm2::lp_swap::taker_swap::{maker_payment_wait, MakerNegotiationData, TakerPaymentSpentData,
                                      TakerSavedEvent, TakerSwapData, TakerSwapEvent, TAKER_ERROR_EVENTS,
                                      TAKER_SUCCESS_EVENTS};
use crate::mm2::lp_swap::{MakerSavedEvent, MakerSavedSwap, SavedSwap, SwapError, TakerSavedSwap};
use coins::{lp_coinfind, MmCoinEnum};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{HttpStatusCode, StatusCode};
use derive_more::Display;
use rpc::v1::types::{H160 as H160Json, H256 as H256Json};

pub type RecreateSwapResult<T> = Result<T, MmError<RecreateSwapError>>;

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum RecreateSwapError {
    #[display(fmt = "Swap hasn't been started. Swap not recoverable")]
    SwapIsNotStarted,
    #[display(fmt = "Swap hasn't been negotiated. Swap not recoverable")]
    SwapIsNotNegotiated,
    #[display(fmt = "Expected '{}' event, found '{}'", expected, found)]
    UnexpectedEvent { expected: String, found: String },
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "'secret_hash' not found in swap data")]
    NoSecretHash,
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl HttpStatusCode for RecreateSwapError {
    fn status_code(&self) -> StatusCode { StatusCode::BAD_REQUEST }
}

impl RecreateSwapError {
    fn unexpected_event(found: String, expected: &str) -> RecreateSwapError {
        RecreateSwapError::UnexpectedEvent {
            expected: expected.to_owned(),
            found,
        }
    }
}

/// The input swap can be either tagged by `type` or not.
#[derive(Deserialize)]
#[serde(untagged)]
#[allow(clippy::enum_variant_names)]
pub enum InputSwap {
    SavedSwap(SavedSwap),
    MakerSavedSwap(MakerSavedSwap),
    TakerSavedSwap(TakerSavedSwap),
}

#[derive(Deserialize)]
pub struct RecreateSwapRequest {
    swap: InputSwap,
}

#[derive(Serialize)]
pub struct RecreateSwapResponse {
    swap: SavedSwap,
}

pub async fn recreate_swap_data(ctx: MmArc, args: RecreateSwapRequest) -> RecreateSwapResult<RecreateSwapResponse> {
    match args.swap {
        InputSwap::SavedSwap(SavedSwap::Maker(maker_swap)) | InputSwap::MakerSavedSwap(maker_swap) => {
            recreate_taker_swap(ctx, maker_swap)
                .await
                .map(SavedSwap::from)
                .map(|swap| RecreateSwapResponse { swap })
        },
        InputSwap::SavedSwap(SavedSwap::Taker(taker_swap)) | InputSwap::TakerSavedSwap(taker_swap) => {
            recreate_maker_swap(ctx, taker_swap)
                .map(SavedSwap::from)
                .map(|swap| RecreateSwapResponse { swap })
        },
    }
}

fn recreate_maker_swap(ctx: MmArc, taker_swap: TakerSavedSwap) -> RecreateSwapResult<MakerSavedSwap> {
    let mut maker_swap = MakerSavedSwap {
        uuid: taker_swap.uuid,
        my_order_uuid: taker_swap.my_order_uuid,
        events: Vec::new(),
        maker_amount: taker_swap.maker_amount,
        maker_coin: taker_swap.maker_coin,
        taker_amount: taker_swap.taker_amount,
        taker_coin: taker_swap.taker_coin,
        gui: ctx.gui().map(|s| s.to_owned()),
        mm_version: Some(ctx.mm_version.clone()),
        success_events: MAKER_SUCCESS_EVENTS.iter().map(|event| event.to_string()).collect(),
        error_events: MAKER_ERROR_EVENTS.iter().map(|event| event.to_string()).collect(),
    };

    let mut event_it = taker_swap.events.into_iter();

    let (started_event_timestamp, started_event) = {
        let TakerSavedEvent { event, timestamp } = event_it.next().or_mm_err(|| RecreateSwapError::SwapIsNotStarted)?;
        match event {
            TakerSwapEvent::Started(started) => (timestamp, started),
            event => return MmError::err(RecreateSwapError::unexpected_event(event.status_str(), "Started")),
        }
    };

    let (negotiated_event_timestamp, negotiated_event) = {
        let TakerSavedEvent { event, timestamp } =
            event_it.next().or_mm_err(|| RecreateSwapError::SwapIsNotNegotiated)?;
        match event {
            TakerSwapEvent::Negotiated(negotiated) => (timestamp, negotiated),
            event => return MmError::err(RecreateSwapError::unexpected_event(event.status_str(), "Negotiated")),
        }
    };

    // Generate `Started` event

    let mut taker_p2p_pubkey = [0; 32];
    taker_p2p_pubkey.copy_from_slice(&started_event.my_persistent_pub.0[1..33]);
    let maker_started_event = MakerSwapEvent::Started(MakerSwapData {
        taker_coin: started_event.taker_coin,
        maker_coin: started_event.maker_coin,
        taker: H256Json::from(taker_p2p_pubkey),
        // We could parse the `TakerSwapEvent::TakerPaymentSpent` event.
        // As for now, don't try to find the secret in the events since we can refund without it.
        secret: H256Json::default(),
        secret_hash: Some(negotiated_event.secret_hash),
        my_persistent_pub: negotiated_event.maker_pubkey,
        lock_duration: started_event.lock_duration,
        maker_amount: started_event.maker_amount,
        taker_amount: started_event.taker_amount,
        maker_payment_confirmations: started_event.maker_payment_confirmations,
        maker_payment_requires_nota: started_event.maker_payment_requires_nota,
        taker_payment_confirmations: started_event.taker_payment_confirmations,
        taker_payment_requires_nota: started_event.taker_payment_requires_nota,
        maker_payment_lock: negotiated_event.maker_payment_locktime,
        uuid: started_event.uuid,
        started_at: started_event.started_at,
        maker_coin_start_block: started_event.maker_coin_start_block,
        taker_coin_start_block: started_event.taker_coin_start_block,
        // Don't set the fee since the value is used when we calculate locked by other swaps amount only.
        maker_payment_trade_fee: None,
        // Don't set the fee since the value is used when we calculate locked by other swaps amount only.
        taker_payment_spend_trade_fee: None,
        maker_coin_swap_contract_address: negotiated_event.maker_coin_swap_contract_addr.clone(),
        taker_coin_swap_contract_address: negotiated_event.taker_coin_swap_contract_addr.clone(),
    });
    maker_swap.events.push(MakerSavedEvent {
        timestamp: started_event_timestamp,
        event: maker_started_event,
    });

    // Generate `Negotiated` event

    let maker_negotiated_event = MakerSwapEvent::Negotiated(TakerNegotiationData {
        taker_payment_locktime: started_event.taker_payment_lock,
        taker_pubkey: started_event.my_persistent_pub,
        maker_coin_swap_contract_addr: negotiated_event.maker_coin_swap_contract_addr,
        taker_coin_swap_contract_addr: negotiated_event.taker_coin_swap_contract_addr,
    });
    maker_swap.events.push(MakerSavedEvent {
        timestamp: negotiated_event_timestamp,
        event: maker_negotiated_event,
    });

    // Then we can continue to process success Taker events.
    let wait_refund_until = negotiated_event.maker_payment_locktime + 3700;
    maker_swap
        .events
        .extend(convert_taker_to_maker_events(event_it, wait_refund_until));

    Ok(maker_swap)
}

/// Converts `TakerSwapEvent` to `MakerSwapEvent`.
/// Please note that this method ignores the [`TakerSwapEvent::Started`] and [`TakerSwapEvent::Negotiated`] events
/// because they are used outside of this function to generate `MakerSwap` and the initial [`MakerSwapEvent::Started`] and [`MakerSwapEvent::Negotiated`] events.
fn convert_taker_to_maker_events(
    event_it: impl Iterator<Item = TakerSavedEvent>,
    wait_refund_until: u64,
) -> Vec<MakerSavedEvent> {
    let mut events = Vec::new();
    let mut taker_fee_ident = None;
    for TakerSavedEvent { event, timestamp } in event_it {
        macro_rules! push_event {
            ($maker_event:expr) => {
                events.push(MakerSavedEvent {
                    timestamp,
                    event: $maker_event,
                })
            };
        }

        // This is used only if an error occurs.
        let swap_error = SwapError {
            error: format!("Origin Taker error event: {:?}", event),
        };
        match event {
            // Even if we considered Taker fee as invalid, then we shouldn't have sent Maker payment.
            TakerSwapEvent::TakerFeeSent(tx_ident) => taker_fee_ident = Some(tx_ident),
            TakerSwapEvent::TakerFeeSendFailed(_) => {
                // Maker shouldn't send MakerPayment in this case.
                push_event!(MakerSwapEvent::TakerFeeValidateFailed(swap_error));
                push_event!(MakerSwapEvent::Finished);
                // Finish processing Taker events.
                return events;
            },
            TakerSwapEvent::MakerPaymentReceived(tx_ident) => {
                if let Some(taker_fee_ident) = taker_fee_ident.take() {
                    push_event!(MakerSwapEvent::TakerFeeValidated(taker_fee_ident));
                }
                push_event!(MakerSwapEvent::MakerPaymentSent(tx_ident))
            },
            TakerSwapEvent::MakerPaymentValidateFailed(_)
            | TakerSwapEvent::MakerPaymentWaitConfirmFailed(_)
            | TakerSwapEvent::TakerPaymentTransactionFailed(_)
            | TakerSwapEvent::TakerPaymentDataSendFailed(_)
            // We actually could confirm TakerPayment and spend it, but for now we don't know about the transaction.
            | TakerSwapEvent::TakerPaymentWaitConfirmFailed(_) => {
                // Maker shouldn't receive an info about TakerPayment.
                push_event!(MakerSwapEvent::TakerPaymentValidateFailed(swap_error));
                push_event!(MakerSwapEvent::MakerPaymentWaitRefundStarted {
                    wait_until: wait_refund_until
                });
                // Finish processing Taker events.
                return events;
            },
            TakerSwapEvent::TakerPaymentSent(tx_ident) => {
                push_event!(MakerSwapEvent::TakerPaymentReceived(tx_ident));
                // Please note we have not to push `TakerPaymentValidatedAndConfirmed` since we could actually decline it.
                push_event!(MakerSwapEvent::TakerPaymentWaitConfirmStarted);
            },
            TakerSwapEvent::TakerPaymentSpent(payment_spent_data) => {
                push_event!(MakerSwapEvent::TakerPaymentValidatedAndConfirmed);
                push_event!(MakerSwapEvent::TakerPaymentSpent(payment_spent_data.transaction));
                push_event!(MakerSwapEvent::TakerPaymentSpendConfirmStarted);
                // We can consider the spent transaction validated and confirmed since the taker found it on the blockchain.
                push_event!(MakerSwapEvent::TakerPaymentSpendConfirmed);
                push_event!(MakerSwapEvent::Finished);
                // Finish the function, because we spent TakerPayment
                return events;
            },
            TakerSwapEvent::TakerPaymentWaitForSpendFailed(_) => {
                // Maker hasn't spent TakerPayment for some reason.
                push_event!(MakerSwapEvent::TakerPaymentSpendFailed(swap_error));
                push_event!(MakerSwapEvent::MakerPaymentWaitRefundStarted {
                    wait_until: wait_refund_until
                });
                // Finish processing Taker events.
                return events;
            },
            TakerSwapEvent::Started(_)
            | TakerSwapEvent::StartFailed(_)
            | TakerSwapEvent::Negotiated(_)
            | TakerSwapEvent::NegotiateFailed(_)
            | TakerSwapEvent::MakerPaymentWaitConfirmStarted
            | TakerSwapEvent::MakerPaymentValidatedAndConfirmed
            | TakerSwapEvent::MakerPaymentSpent(_)
            | TakerSwapEvent::MakerPaymentSpendFailed(_)
            // We don't know the reason at the moment, so we rely on the errors handling above.
            | TakerSwapEvent::TakerPaymentWaitRefundStarted { .. }
            | TakerSwapEvent::TakerPaymentRefunded(_)
            | TakerSwapEvent::TakerPaymentRefundFailed(_)
            | TakerSwapEvent::Finished => {}
        }
    }
    events
}

async fn recreate_taker_swap(ctx: MmArc, maker_swap: MakerSavedSwap) -> RecreateSwapResult<TakerSavedSwap> {
    let mut taker_swap = TakerSavedSwap {
        uuid: maker_swap.uuid,
        my_order_uuid: Some(maker_swap.uuid),
        events: Vec::new(),
        maker_amount: maker_swap.maker_amount,
        maker_coin: maker_swap.maker_coin,
        taker_amount: maker_swap.taker_amount,
        taker_coin: maker_swap.taker_coin,
        gui: ctx.gui().map(|s| s.to_owned()),
        mm_version: Some(ctx.mm_version.clone()),
        success_events: TAKER_SUCCESS_EVENTS.iter().map(|event| event.to_string()).collect(),
        error_events: TAKER_ERROR_EVENTS.iter().map(|event| event.to_string()).collect(),
    };

    let mut event_it = maker_swap.events.into_iter();

    let (started_event_timestamp, started_event) = {
        let MakerSavedEvent { event, timestamp } = event_it.next().or_mm_err(|| RecreateSwapError::SwapIsNotStarted)?;
        match event {
            MakerSwapEvent::Started(started) => (timestamp, started),
            event => return MmError::err(RecreateSwapError::unexpected_event(event.status_str(), "Started")),
        }
    };

    let (negotiated_timestamp, negotiated_event) = {
        let MakerSavedEvent { event, timestamp } =
            event_it.next().or_mm_err(|| RecreateSwapError::SwapIsNotNegotiated)?;
        match event {
            MakerSwapEvent::Negotiated(negotiated) => (timestamp, negotiated),
            event => return MmError::err(RecreateSwapError::unexpected_event(event.status_str(), "Negotiated")),
        }
    };

    let mut maker_p2p_pubkey = [0; 32];
    maker_p2p_pubkey.copy_from_slice(&started_event.my_persistent_pub.0[1..33]);
    let taker_started_event = TakerSwapEvent::Started(TakerSwapData {
        taker_coin: started_event.taker_coin,
        maker_coin: started_event.maker_coin.clone(),
        maker: H256Json::from(maker_p2p_pubkey),
        my_persistent_pub: negotiated_event.taker_pubkey,
        lock_duration: started_event.lock_duration,
        maker_amount: started_event.maker_amount,
        taker_amount: started_event.taker_amount,
        maker_payment_confirmations: started_event.maker_payment_confirmations,
        maker_payment_requires_nota: started_event.maker_payment_requires_nota,
        taker_payment_confirmations: started_event.taker_payment_confirmations,
        taker_payment_requires_nota: started_event.taker_payment_requires_nota,
        taker_payment_lock: negotiated_event.taker_payment_locktime,
        uuid: started_event.uuid,
        started_at: started_event.started_at,
        maker_payment_wait: maker_payment_wait(started_event.started_at, started_event.lock_duration),
        maker_coin_start_block: started_event.maker_coin_start_block,
        taker_coin_start_block: started_event.taker_coin_start_block,
        // Don't set the fee since the value is used when we calculate locked by other swaps amount only.
        fee_to_send_taker_fee: None,
        // Don't set the fee since the value is used when we calculate locked by other swaps amount only.
        taker_payment_trade_fee: None,
        // Don't set the fee since the value is used when we calculate locked by other swaps amount only.
        maker_payment_spend_trade_fee: None,
        maker_coin_swap_contract_address: negotiated_event.maker_coin_swap_contract_addr.clone(),
        taker_coin_swap_contract_address: negotiated_event.taker_coin_swap_contract_addr.clone(),
    });
    taker_swap.events.push(TakerSavedEvent {
        timestamp: started_event_timestamp,
        event: taker_started_event,
    });

    let secret_hash = started_event
        .secret_hash
        .or_mm_err(|| RecreateSwapError::NoSecretHash)?;

    let taker_negotiated_event = TakerSwapEvent::Negotiated(MakerNegotiationData {
        maker_payment_locktime: started_event.maker_payment_lock,
        maker_pubkey: started_event.my_persistent_pub,
        secret_hash: secret_hash.clone(),
        maker_coin_swap_contract_addr: negotiated_event.maker_coin_swap_contract_addr,
        taker_coin_swap_contract_addr: negotiated_event.taker_coin_swap_contract_addr,
    });
    taker_swap.events.push(TakerSavedEvent {
        timestamp: negotiated_timestamp,
        event: taker_negotiated_event,
    });

    // Can be used to extract a secret from [`MakerSwapEvent::TakerPaymentSpent`].
    let maker_coin_ticker = started_event.maker_coin;
    let maker_coin = lp_coinfind(&ctx, &maker_coin_ticker)
        .await
        .map_to_mm(RecreateSwapError::Internal)?
        .or_mm_err(move || RecreateSwapError::NoSuchCoin {
            coin: maker_coin_ticker,
        })?;

    // Then we can continue to process success Maker events.
    let wait_refund_until = negotiated_event.taker_payment_locktime + 3700;
    taker_swap.events.extend(convert_maker_to_taker_events(
        event_it,
        maker_coin,
        secret_hash,
        wait_refund_until,
    ));

    Ok(taker_swap)
}

/// Converts `MakerSwapEvent` to `TakerSwapEvent`.
/// Please note that this method ignores the [`MakerSwapEvent::Started`] and [`MakerSwapEvent::Negotiated`] events
/// since they are used outside of this function to generate `TakerSwap` and the initial [`TakerSwapEvent::Started`] and [`TakerSwapEvent::Negotiated`] events.
///
/// The `maker_coin` and `secret_hash` function arguments are used to extract a secret from `TakerPaymentSpent`.
fn convert_maker_to_taker_events(
    event_it: impl Iterator<Item = MakerSavedEvent>,
    maker_coin: MmCoinEnum,
    secret_hash: H160Json,
    wait_refund_until: u64,
) -> Vec<TakerSavedEvent> {
    let mut events = Vec::new();
    for MakerSavedEvent { event, timestamp } in event_it {
        macro_rules! push_event {
            ($maker_event:expr) => {
                events.push(TakerSavedEvent {
                    timestamp,
                    event: $maker_event,
                })
            };
        }

        // This is used only if an error occurs.
        let swap_error = SwapError {
            error: format!("Origin Maker error event: {:?}", event),
        };
        match event {
            MakerSwapEvent::TakerFeeValidated(tx_ident) => push_event!(TakerSwapEvent::TakerFeeSent(tx_ident)),
            MakerSwapEvent::TakerFeeValidateFailed(_) => {
                push_event!(TakerSwapEvent::MakerPaymentValidateFailed(swap_error));
                push_event!(TakerSwapEvent::Finished);
                // Finish processing Maker events.
                return events;
            },
            MakerSwapEvent::MakerPaymentSent(tx_ident) => {
                push_event!(TakerSwapEvent::MakerPaymentReceived(tx_ident));
                // Please note we have not to push `MakerPaymentValidatedAndConfirmed` since we could actually decline it.
                push_event!(TakerSwapEvent::MakerPaymentWaitConfirmStarted);
            },
            MakerSwapEvent::MakerPaymentTransactionFailed(_)
            | MakerSwapEvent::MakerPaymentDataSendFailed(_)
            // We actually could confirm MakerPayment and send TakerPayment, but for now we don't know about the transaction.
            | MakerSwapEvent::MakerPaymentWaitConfirmFailed(_) => {
                push_event!(TakerSwapEvent::MakerPaymentValidateFailed(swap_error));
                push_event!(TakerSwapEvent::Finished);
                // Finish processing Maker events.
                return events;
            },
            MakerSwapEvent::TakerPaymentReceived(tx_ident) => {
                push_event!(TakerSwapEvent::MakerPaymentValidatedAndConfirmed);
                push_event!(TakerSwapEvent::TakerPaymentSent(tx_ident.clone()));
            },
            MakerSwapEvent::TakerPaymentValidateFailed(_)
            | MakerSwapEvent::TakerPaymentSpendFailed(_)
            | MakerSwapEvent::TakerPaymentWaitConfirmFailed(_) => {
                push_event!(TakerSwapEvent::TakerPaymentWaitForSpendFailed(swap_error));
                push_event!(TakerSwapEvent::TakerPaymentWaitRefundStarted { wait_until: wait_refund_until });
                // Finish processing Maker events.
                return events;
            },
            MakerSwapEvent::TakerPaymentSpent(tx_ident) => {
                let secret = match maker_coin.extract_secret(&secret_hash.0, &tx_ident.tx_hex) {
                    Ok(secret) => H256Json::from(secret.as_slice()),
                    Err(e) => {
                        push_event!(TakerSwapEvent::TakerPaymentWaitForSpendFailed(ERRL!("{}", e).into()));
                        push_event!(TakerSwapEvent::TakerPaymentWaitRefundStarted { wait_until: wait_refund_until });
                        return events;
                    },
                };
                push_event!(TakerSwapEvent::TakerPaymentSpent(TakerPaymentSpentData {
                    secret,
                    transaction: tx_ident,
                }));
                return events;
            },
            MakerSwapEvent::Started(_)
            | MakerSwapEvent::StartFailed(_)
            | MakerSwapEvent::Negotiated(_)
            | MakerSwapEvent::NegotiateFailed(_)
            | MakerSwapEvent::TakerPaymentWaitConfirmStarted
            | MakerSwapEvent::TakerPaymentValidatedAndConfirmed
            | MakerSwapEvent::TakerPaymentSpendConfirmStarted
            | MakerSwapEvent::TakerPaymentSpendConfirmed
            // I think we can't be sure if the spend transaction is declined by the network.
            | MakerSwapEvent::TakerPaymentSpendConfirmFailed(_)
            // We don't know the reason at the moment, so we rely on the errors handling above.
            | MakerSwapEvent::MakerPaymentWaitRefundStarted { .. }
            | MakerSwapEvent::MakerPaymentRefunded(_)
            | MakerSwapEvent::MakerPaymentRefundFailed(_)
            | MakerSwapEvent::Finished => {}
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use coins::{CoinsContext, SwapOps, TestCoin};
    use common::block_on;
    use common::mm_ctx::MmCtxBuilder;
    use mocktopus::mocking::{MockResult, Mockable};
    use serde_json as json;

    #[test]
    fn test_recreate_maker_swap() {
        let taker_saved_json = r#"{"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","my_order_uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","events":[{"timestamp":1638984440546,"event":{"type":"Started","data":{"taker_coin":"MORTY","maker_coin":"RICK","maker":"15d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","my_persistent_pub":"03b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","lock_duration":7800,"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","taker_amount":"1","maker_payment_confirmations":1,"maker_payment_requires_nota":false,"taker_payment_confirmations":1,"taker_payment_requires_nota":false,"taker_payment_lock":1638992240,"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","started_at":1638984440,"maker_payment_wait":1638987560,"maker_coin_start_block":1207822,"taker_coin_start_block":1222573,"fee_to_send_taker_fee":{"coin":"MORTY","amount":"0.00001","paid_from_trading_vol":false},"taker_payment_trade_fee":{"coin":"MORTY","amount":"0.00001","paid_from_trading_vol":false},"maker_payment_spend_trade_fee":{"coin":"RICK","amount":"0.00001","paid_from_trading_vol":true}}}},{"timestamp":1638984456603,"event":{"type":"Negotiated","data":{"maker_payment_locktime":1639000040,"maker_pubkey":"0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","secret_hash":"4da9e7080175e8e10842e0e161b33cd298cab30b","maker_coin_swap_contract_addr":null,"taker_coin_swap_contract_addr":null}}},{"timestamp":1638984456814,"event":{"type":"TakerFeeSent","data":{"tx_hex":"0400008085202f89016383e8aced2256378bb126a1ca1a41e2f344d9295f65b3ea4b99055c5eb4a6cb000000006a47304402201c7e661e0dbeb9b3eb6e4e9e3194010e5772227017772b2e48c1b8d48ed3b21f02201c2eda64e74455fa1878a5c221f25d22fe626abd0078a26a9fc0f829e0921639012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff02bcf60100000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac74c3e90b000000001976a91483762a373935ca241d557dfce89171d582b486de88ac08ebb061000000000000000000000000000000","tx_hash":"fcb49167c79e8e014143643b94878866f7e80b26c5a5dcf693010543da70b5bc"}}},{"timestamp":1638984457822,"event":{"type":"MakerPaymentReceived","data":{"tx_hex":"0400008085202f8901c41fdf6b9d8aea4b472f83e4fa0d99dfafc245e897d681fd2ca7df30707fbf48020000006b483045022100c7b294bd46cbf3b13530879a43c5cf67414047266d8b64c3c7263b5e75b989ba02201974f38d688b184bc44e628806c6ab2ac9092f394729d0ce838f14e1e76117c001210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ffffffff03a2296b050000000017a91491c45f69e1760c12a1f90fb2a811f6dfde35cc35870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30bac503d64000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac09ebb061000000000000000000000000000000","tx_hash":"6287e0d30951cd859bfb837eb1e5409f7596e75ffeb2e61fd6df1843bfd0203d"}}},{"timestamp":1638984457826,"event":{"type":"MakerPaymentWaitConfirmStarted"}},{"timestamp":1638984503611,"event":{"type":"MakerPaymentValidatedAndConfirmed"}},{"timestamp":1638984503974,"event":{"type":"TakerPaymentSent","data":{"tx_hex":"0400008085202f8901bcb570da43050193f6dca5c5260be8f7668887943b644341018e9ec76791b4fc010000006b483045022100fe6c90568a256b531bcd18321c15b3ce68c2d5d603768dea6aba68dcc170b801022076a34c006a92786bcdee6a1dcb46947fb49911e4f51ec27e880c3396d64d59b2012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff0300e1f5050000000017a9145a6125d597d2ce37bde9983d4d9d481335139bc4870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30b8cdef305000000001976a91483762a373935ca241d557dfce89171d582b486de88ac37ebb061000000000000000000000000000000","tx_hash":"667dea45dbd36e1f15ee8170e73e2afefd0a15d47d3bcd65655ca71d838445a2"}}},{"timestamp":1638984600390,"event":{"type":"TakerPaymentSpent","data":{"transaction":{"tx_hex":"0400008085202f8901a24584831da75c6565cd3b7dd4150afdfe2a3ee77081ee151f6ed3db45ea7d6600000000d74730440220422edb8ef5cd3991eb309c3a4fa5fe5d9ffe08d3d6b4b789c5587061d7993864022049cd082398f5a37a9e7411d56976e61bcce9162d0f5f1fb24e40bcf2f4ec0052012023a6bb64bc0ab2cc14cb84277d8d25134b814e5f999c66e578c9bba3c5e2d3a4004c6b63047009b161b1752103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddac6782012088a9144da9e7080175e8e10842e0e161b33cd298cab30b88210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ac68ffffffff0118ddf505000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac7009b161000000000000000000000000000000","tx_hash":"ab1eb5b65a302370af2607e0b64b60fc04360de33a87799bca1dcf337344b616"},"secret":"23a6bb64bc0ab2cc14cb84277d8d25134b814e5f999c66e578c9bba3c5e2d3a4"}}},{"timestamp":1638984600829,"event":{"type":"MakerPaymentSpent","data":{"tx_hex":"0400008085202f89013d20d0bf4318dfd61fe6b2fe5fe796759f40e5b17e83fb9b85cd5109d3e0876200000000d747304402200a57f752b760a8dcb932244dde0a46112a4d08bd5d31704c9138dc52b02a57e602204f0406dd354271e9850862e2e8c1feec57380e858952ab6cfedc8e7725196c94012023a6bb64bc0ab2cc14cb84277d8d25134b814e5f999c66e578c9bba3c5e2d3a4004c6b6304e827b161b175210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ac6782012088a9144da9e7080175e8e10842e0e161b33cd298cab30b882103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddac68ffffffff01ba256b05000000001976a91483762a373935ca241d557dfce89171d582b486de88ace827b161000000000000000000000000000000","tx_hash":"ca0721b69657c0ea2dcb848cc9e44e66d719ae10477097bf0fec57866a4f66aa"}}},{"timestamp":1638984600832,"event":{"type":"Finished"}}],"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","maker_coin":"RICK","taker_amount":"1","taker_coin":"MORTY","gui":"atomicDEX 0.5.1 iOS","mm_version":"1b065636a","success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","MakerPaymentWaitConfirmFailed","TakerPaymentTransactionFailed","TakerPaymentWaitConfirmFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentWaitRefundStarted","TakerPaymentRefunded","TakerPaymentRefundFailed"]}"#;
        let maker_expected_json = r#"{"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","my_order_uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","events":[{"timestamp":1638984440546,"event":{"type":"Started","data":{"taker_coin":"MORTY","maker_coin":"RICK","taker":"b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","secret":"0000000000000000000000000000000000000000000000000000000000000000","secret_hash":"4da9e7080175e8e10842e0e161b33cd298cab30b","my_persistent_pub":"0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","lock_duration":7800,"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","taker_amount":"1","maker_payment_confirmations":1,"maker_payment_requires_nota":false,"taker_payment_confirmations":1,"taker_payment_requires_nota":false,"maker_payment_lock":1639000040,"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","started_at":1638984440,"maker_coin_start_block":1207822,"taker_coin_start_block":1222573,"maker_payment_trade_fee":null,"taker_payment_spend_trade_fee":null}}},{"timestamp":1638984456603,"event":{"type":"Negotiated","data":{"taker_payment_locktime":1638992240,"taker_pubkey":"03b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","maker_coin_swap_contract_addr":null,"taker_coin_swap_contract_addr":null}}},{"timestamp":1638984457822,"event":{"type":"TakerFeeValidated","data":{"tx_hex":"0400008085202f89016383e8aced2256378bb126a1ca1a41e2f344d9295f65b3ea4b99055c5eb4a6cb000000006a47304402201c7e661e0dbeb9b3eb6e4e9e3194010e5772227017772b2e48c1b8d48ed3b21f02201c2eda64e74455fa1878a5c221f25d22fe626abd0078a26a9fc0f829e0921639012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff02bcf60100000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac74c3e90b000000001976a91483762a373935ca241d557dfce89171d582b486de88ac08ebb061000000000000000000000000000000","tx_hash":"fcb49167c79e8e014143643b94878866f7e80b26c5a5dcf693010543da70b5bc"}}},{"timestamp":1638984457822,"event":{"type":"MakerPaymentSent","data":{"tx_hex":"0400008085202f8901c41fdf6b9d8aea4b472f83e4fa0d99dfafc245e897d681fd2ca7df30707fbf48020000006b483045022100c7b294bd46cbf3b13530879a43c5cf67414047266d8b64c3c7263b5e75b989ba02201974f38d688b184bc44e628806c6ab2ac9092f394729d0ce838f14e1e76117c001210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ffffffff03a2296b050000000017a91491c45f69e1760c12a1f90fb2a811f6dfde35cc35870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30bac503d64000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac09ebb061000000000000000000000000000000","tx_hash":"6287e0d30951cd859bfb837eb1e5409f7596e75ffeb2e61fd6df1843bfd0203d"}}},{"timestamp":1638984503974,"event":{"type":"TakerPaymentReceived","data":{"tx_hex":"0400008085202f8901bcb570da43050193f6dca5c5260be8f7668887943b644341018e9ec76791b4fc010000006b483045022100fe6c90568a256b531bcd18321c15b3ce68c2d5d603768dea6aba68dcc170b801022076a34c006a92786bcdee6a1dcb46947fb49911e4f51ec27e880c3396d64d59b2012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff0300e1f5050000000017a9145a6125d597d2ce37bde9983d4d9d481335139bc4870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30b8cdef305000000001976a91483762a373935ca241d557dfce89171d582b486de88ac37ebb061000000000000000000000000000000","tx_hash":"667dea45dbd36e1f15ee8170e73e2afefd0a15d47d3bcd65655ca71d838445a2"}}},{"timestamp":1638984503974,"event":{"type":"TakerPaymentWaitConfirmStarted"}},{"timestamp":1638984600390,"event":{"type":"TakerPaymentValidatedAndConfirmed"}},{"timestamp":1638984600390,"event":{"type":"TakerPaymentSpent","data":{"tx_hex":"0400008085202f8901a24584831da75c6565cd3b7dd4150afdfe2a3ee77081ee151f6ed3db45ea7d6600000000d74730440220422edb8ef5cd3991eb309c3a4fa5fe5d9ffe08d3d6b4b789c5587061d7993864022049cd082398f5a37a9e7411d56976e61bcce9162d0f5f1fb24e40bcf2f4ec0052012023a6bb64bc0ab2cc14cb84277d8d25134b814e5f999c66e578c9bba3c5e2d3a4004c6b63047009b161b1752103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddac6782012088a9144da9e7080175e8e10842e0e161b33cd298cab30b88210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ac68ffffffff0118ddf505000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac7009b161000000000000000000000000000000","tx_hash":"ab1eb5b65a302370af2607e0b64b60fc04360de33a87799bca1dcf337344b616"}}},{"timestamp":1638984600390,"event":{"type":"TakerPaymentSpendConfirmStarted"}},{"timestamp":1638984600390,"event":{"type":"TakerPaymentSpendConfirmed"}},{"timestamp":1638984600390,"event":{"type":"Finished"}}],"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","maker_coin":"RICK","taker_amount":"1","taker_coin":"MORTY","gui":null,"mm_version":"","success_events":["Started","Negotiated","TakerFeeValidated","MakerPaymentSent","TakerPaymentReceived","TakerPaymentWaitConfirmStarted","TakerPaymentValidatedAndConfirmed","TakerPaymentSpent","TakerPaymentSpendConfirmStarted","TakerPaymentSpendConfirmed","Finished"],"error_events":["StartFailed","NegotiateFailed","TakerFeeValidateFailed","MakerPaymentTransactionFailed","MakerPaymentDataSendFailed","MakerPaymentWaitConfirmFailed","TakerPaymentValidateFailed","TakerPaymentWaitConfirmFailed","TakerPaymentSpendFailed","TakerPaymentSpendConfirmFailed","MakerPaymentWaitRefundStarted","MakerPaymentRefunded","MakerPaymentRefundFailed"]}"#;

        let taker_saved_swap: TakerSavedSwap = json::from_str(taker_saved_json).unwrap();
        let maker_expected_swap: MakerSavedSwap = json::from_str(maker_expected_json).unwrap();

        let ctx = MmCtxBuilder::default().into_mm_arc();

        let maker_actual_swap = recreate_maker_swap(ctx, taker_saved_swap).expect("!recreate_maker_swap");
        println!("{}", json::to_string(&maker_actual_swap).unwrap());
        assert_eq!(maker_actual_swap, maker_expected_swap);
    }

    #[test]
    fn test_recreate_maker_swap_maker_payment_wait_confirm_failed() {
        let taker_saved_json = r#"{"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","my_order_uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","events":[{"timestamp":1638984440546,"event":{"type":"Started","data":{"taker_coin":"MORTY","maker_coin":"RICK","maker":"15d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","my_persistent_pub":"03b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","lock_duration":7800,"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","taker_amount":"1","maker_payment_confirmations":1,"maker_payment_requires_nota":false,"taker_payment_confirmations":1,"taker_payment_requires_nota":false,"taker_payment_lock":1638992240,"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","started_at":1638984440,"maker_payment_wait":1638987560,"maker_coin_start_block":1207822,"taker_coin_start_block":1222573,"fee_to_send_taker_fee":{"coin":"MORTY","amount":"0.00001","paid_from_trading_vol":false},"taker_payment_trade_fee":{"coin":"MORTY","amount":"0.00001","paid_from_trading_vol":false},"maker_payment_spend_trade_fee":{"coin":"RICK","amount":"0.00001","paid_from_trading_vol":true}}}},{"timestamp":1638984456603,"event":{"type":"Negotiated","data":{"maker_payment_locktime":1639000040,"maker_pubkey":"0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","secret_hash":"4da9e7080175e8e10842e0e161b33cd298cab30b","maker_coin_swap_contract_addr":null,"taker_coin_swap_contract_addr":null}}},{"timestamp":1638984456814,"event":{"type":"TakerFeeSent","data":{"tx_hex":"0400008085202f89016383e8aced2256378bb126a1ca1a41e2f344d9295f65b3ea4b99055c5eb4a6cb000000006a47304402201c7e661e0dbeb9b3eb6e4e9e3194010e5772227017772b2e48c1b8d48ed3b21f02201c2eda64e74455fa1878a5c221f25d22fe626abd0078a26a9fc0f829e0921639012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff02bcf60100000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac74c3e90b000000001976a91483762a373935ca241d557dfce89171d582b486de88ac08ebb061000000000000000000000000000000","tx_hash":"fcb49167c79e8e014143643b94878866f7e80b26c5a5dcf693010543da70b5bc"}}},{"timestamp":1638984457822,"event":{"type":"MakerPaymentReceived","data":{"tx_hex":"0400008085202f8901c41fdf6b9d8aea4b472f83e4fa0d99dfafc245e897d681fd2ca7df30707fbf48020000006b483045022100c7b294bd46cbf3b13530879a43c5cf67414047266d8b64c3c7263b5e75b989ba02201974f38d688b184bc44e628806c6ab2ac9092f394729d0ce838f14e1e76117c001210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ffffffff03a2296b050000000017a91491c45f69e1760c12a1f90fb2a811f6dfde35cc35870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30bac503d64000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac09ebb061000000000000000000000000000000","tx_hash":"6287e0d30951cd859bfb837eb1e5409f7596e75ffeb2e61fd6df1843bfd0203d"}}},{"timestamp":1638984457826,"event":{"type":"MakerPaymentWaitConfirmStarted"}},{"timestamp":1638984503611,"event":{"type":"MakerPaymentWaitConfirmFailed","data":{"error":"An error"}}},{"timestamp":1638984503615,"event":{"type":"Finished"}}],"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","maker_coin":"RICK","taker_amount":"1","taker_coin":"MORTY","gui":"atomicDEX 0.5.1 iOS","mm_version":"1b065636a","success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","MakerPaymentWaitConfirmFailed","TakerPaymentTransactionFailed","TakerPaymentWaitConfirmFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentWaitRefundStarted","TakerPaymentRefunded","TakerPaymentRefundFailed"]}"#;
        let maker_expected_json = r#"{"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","my_order_uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","events":[{"timestamp":1638984440546,"event":{"type":"Started","data":{"taker_coin":"MORTY","maker_coin":"RICK","taker":"b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","secret":"0000000000000000000000000000000000000000000000000000000000000000","secret_hash":"4da9e7080175e8e10842e0e161b33cd298cab30b","my_persistent_pub":"0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","lock_duration":7800,"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","taker_amount":"1","maker_payment_confirmations":1,"maker_payment_requires_nota":false,"taker_payment_confirmations":1,"taker_payment_requires_nota":false,"maker_payment_lock":1639000040,"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","started_at":1638984440,"maker_coin_start_block":1207822,"taker_coin_start_block":1222573,"maker_payment_trade_fee":null,"taker_payment_spend_trade_fee":null}}},{"timestamp":1638984456603,"event":{"type":"Negotiated","data":{"taker_payment_locktime":1638992240,"taker_pubkey":"03b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","maker_coin_swap_contract_addr":null,"taker_coin_swap_contract_addr":null}}},{"timestamp":1638984457822,"event":{"type":"TakerFeeValidated","data":{"tx_hex":"0400008085202f89016383e8aced2256378bb126a1ca1a41e2f344d9295f65b3ea4b99055c5eb4a6cb000000006a47304402201c7e661e0dbeb9b3eb6e4e9e3194010e5772227017772b2e48c1b8d48ed3b21f02201c2eda64e74455fa1878a5c221f25d22fe626abd0078a26a9fc0f829e0921639012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff02bcf60100000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac74c3e90b000000001976a91483762a373935ca241d557dfce89171d582b486de88ac08ebb061000000000000000000000000000000","tx_hash":"fcb49167c79e8e014143643b94878866f7e80b26c5a5dcf693010543da70b5bc"}}},{"timestamp":1638984457822,"event":{"type":"MakerPaymentSent","data":{"tx_hex":"0400008085202f8901c41fdf6b9d8aea4b472f83e4fa0d99dfafc245e897d681fd2ca7df30707fbf48020000006b483045022100c7b294bd46cbf3b13530879a43c5cf67414047266d8b64c3c7263b5e75b989ba02201974f38d688b184bc44e628806c6ab2ac9092f394729d0ce838f14e1e76117c001210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ffffffff03a2296b050000000017a91491c45f69e1760c12a1f90fb2a811f6dfde35cc35870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30bac503d64000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac09ebb061000000000000000000000000000000","tx_hash":"6287e0d30951cd859bfb837eb1e5409f7596e75ffeb2e61fd6df1843bfd0203d"}}},{"timestamp":1638984503611,"event":{"type":"TakerPaymentValidateFailed","data":{"error":"Origin Taker error event: MakerPaymentWaitConfirmFailed(SwapError { error: \"An error\" })"}}},{"timestamp":1638984503611,"event":{"type":"MakerPaymentWaitRefundStarted","data":{"wait_until":1639003740}}}],"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","maker_coin":"RICK","taker_amount":"1","taker_coin":"MORTY","gui":null,"mm_version":"","success_events":["Started","Negotiated","TakerFeeValidated","MakerPaymentSent","TakerPaymentReceived","TakerPaymentWaitConfirmStarted","TakerPaymentValidatedAndConfirmed","TakerPaymentSpent","TakerPaymentSpendConfirmStarted","TakerPaymentSpendConfirmed","Finished"],"error_events":["StartFailed","NegotiateFailed","TakerFeeValidateFailed","MakerPaymentTransactionFailed","MakerPaymentDataSendFailed","MakerPaymentWaitConfirmFailed","TakerPaymentValidateFailed","TakerPaymentWaitConfirmFailed","TakerPaymentSpendFailed","TakerPaymentSpendConfirmFailed","MakerPaymentWaitRefundStarted","MakerPaymentRefunded","MakerPaymentRefundFailed"]}"#;

        let taker_saved_swap: TakerSavedSwap = json::from_str(taker_saved_json).unwrap();
        let maker_expected_swap: MakerSavedSwap = json::from_str(maker_expected_json).unwrap();

        let ctx = MmCtxBuilder::default().into_mm_arc();

        let maker_actual_swap = recreate_maker_swap(ctx, taker_saved_swap).expect("!recreate_maker_swap");
        println!("{}", json::to_string(&maker_actual_swap).unwrap());
        assert_eq!(maker_actual_swap, maker_expected_swap);
    }

    #[test]
    fn test_recreate_taker_swap() {
        TestCoin::extract_secret.mock_safe(|_coin, _secret_hash, _spend_tx| {
            let secret = hex::decode("23a6bb64bc0ab2cc14cb84277d8d25134b814e5f999c66e578c9bba3c5e2d3a4").unwrap();
            MockResult::Return(Ok(secret))
        });

        let maker_saved_json = r#"{"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","my_order_uuid":"15d007fa-9237-489c-82a7-df061deba95f","events":[{"timestamp":1638984440198,"event":{"type":"Started","data":{"taker_coin":"MORTY","maker_coin":"RICK","taker":"b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","secret":"0000000000000000000000000000000000000000000000000000000000000000","secret_hash":"4da9e7080175e8e10842e0e161b33cd298cab30b","my_persistent_pub":"0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","lock_duration":7800,"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","taker_amount":"1","maker_payment_confirmations":1,"maker_payment_requires_nota":false,"taker_payment_confirmations":1,"taker_payment_requires_nota":false,"maker_payment_lock":1639000040,"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","started_at":1638984440,"maker_coin_start_block":1207822,"taker_coin_start_block":1222573,"maker_payment_trade_fee":{"coin":"RICK","amount":"0.00001","paid_from_trading_vol":false},"taker_payment_spend_trade_fee":{"coin":"MORTY","amount":"0.00001","paid_from_trading_vol":true}}}},{"timestamp":1638984456204,"event":{"type":"Negotiated","data":{"taker_payment_locktime":1638992240,"taker_pubkey":"03b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","maker_coin_swap_contract_addr":null,"taker_coin_swap_contract_addr":null}}},{"timestamp":1638984457215,"event":{"type":"TakerFeeValidated","data":{"tx_hex":"0400008085202f89016383e8aced2256378bb126a1ca1a41e2f344d9295f65b3ea4b99055c5eb4a6cb000000006a47304402201c7e661e0dbeb9b3eb6e4e9e3194010e5772227017772b2e48c1b8d48ed3b21f02201c2eda64e74455fa1878a5c221f25d22fe626abd0078a26a9fc0f829e0921639012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff02bcf60100000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac74c3e90b000000001976a91483762a373935ca241d557dfce89171d582b486de88ac08ebb061000000000000000000000000000000","tx_hash":"fcb49167c79e8e014143643b94878866f7e80b26c5a5dcf693010543da70b5bc"}}},{"timestamp":1638984457230,"event":{"type":"MakerPaymentSent","data":{"tx_hex":"0400008085202f8901c41fdf6b9d8aea4b472f83e4fa0d99dfafc245e897d681fd2ca7df30707fbf48020000006b483045022100c7b294bd46cbf3b13530879a43c5cf67414047266d8b64c3c7263b5e75b989ba02201974f38d688b184bc44e628806c6ab2ac9092f394729d0ce838f14e1e76117c001210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ffffffff03a2296b050000000017a91491c45f69e1760c12a1f90fb2a811f6dfde35cc35870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30bac503d64000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac09ebb061000000000000000000000000000000","tx_hash":"6287e0d30951cd859bfb837eb1e5409f7596e75ffeb2e61fd6df1843bfd0203d"}}},{"timestamp":1638984504262,"event":{"type":"TakerPaymentReceived","data":{"tx_hex":"0400008085202f8901bcb570da43050193f6dca5c5260be8f7668887943b644341018e9ec76791b4fc010000006b483045022100fe6c90568a256b531bcd18321c15b3ce68c2d5d603768dea6aba68dcc170b801022076a34c006a92786bcdee6a1dcb46947fb49911e4f51ec27e880c3396d64d59b2012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff0300e1f5050000000017a9145a6125d597d2ce37bde9983d4d9d481335139bc4870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30b8cdef305000000001976a91483762a373935ca241d557dfce89171d582b486de88ac37ebb061000000000000000000000000000000","tx_hash":"667dea45dbd36e1f15ee8170e73e2afefd0a15d47d3bcd65655ca71d838445a2"}}},{"timestamp":1638984504263,"event":{"type":"TakerPaymentWaitConfirmStarted"}},{"timestamp":1638984594319,"event":{"type":"TakerPaymentValidatedAndConfirmed"}},{"timestamp":1638984594337,"event":{"type":"TakerPaymentSpent","data":{"tx_hex":"0400008085202f8901a24584831da75c6565cd3b7dd4150afdfe2a3ee77081ee151f6ed3db45ea7d6600000000d74730440220422edb8ef5cd3991eb309c3a4fa5fe5d9ffe08d3d6b4b789c5587061d7993864022049cd082398f5a37a9e7411d56976e61bcce9162d0f5f1fb24e40bcf2f4ec0052012023a6bb64bc0ab2cc14cb84277d8d25134b814e5f999c66e578c9bba3c5e2d3a4004c6b63047009b161b1752103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddac6782012088a9144da9e7080175e8e10842e0e161b33cd298cab30b88210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ac68ffffffff0118ddf505000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac7009b161000000000000000000000000000000","tx_hash":"ab1eb5b65a302370af2607e0b64b60fc04360de33a87799bca1dcf337344b616"}}},{"timestamp":1638984594338,"event":{"type":"TakerPaymentSpendConfirmStarted"}},{"timestamp":1638984699392,"event":{"type":"TakerPaymentSpendConfirmed"}},{"timestamp":1638984699393,"event":{"type":"Finished"}}],"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","maker_coin":"RICK","taker_amount":"1","taker_coin":"MORTY","gui":"mpm","mm_version":"213bfddd5","success_events":["Started","Negotiated","TakerFeeValidated","MakerPaymentSent","TakerPaymentReceived","TakerPaymentWaitConfirmStarted","TakerPaymentValidatedAndConfirmed","TakerPaymentSpent","TakerPaymentSpendConfirmStarted","TakerPaymentSpendConfirmed","Finished"],"error_events":["StartFailed","NegotiateFailed","TakerFeeValidateFailed","MakerPaymentTransactionFailed","MakerPaymentDataSendFailed","MakerPaymentWaitConfirmFailed","TakerPaymentValidateFailed","TakerPaymentWaitConfirmFailed","TakerPaymentSpendFailed","TakerPaymentSpendConfirmFailed","MakerPaymentWaitRefundStarted","MakerPaymentRefunded","MakerPaymentRefundFailed"]}"#;
        let taker_expected_json = r#"{"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","my_order_uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","events":[{"timestamp":1638984440198,"event":{"type":"Started","data":{"taker_coin":"MORTY","maker_coin":"RICK","maker":"15d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","my_persistent_pub":"03b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","lock_duration":7800,"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","taker_amount":"1","maker_payment_confirmations":1,"maker_payment_requires_nota":false,"taker_payment_confirmations":1,"taker_payment_requires_nota":false,"taker_payment_lock":1638992240,"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","started_at":1638984440,"maker_payment_wait":1638987560,"maker_coin_start_block":1207822,"taker_coin_start_block":1222573,"fee_to_send_taker_fee":null,"taker_payment_trade_fee":null,"maker_payment_spend_trade_fee":null}}},{"timestamp":1638984456204,"event":{"type":"Negotiated","data":{"maker_payment_locktime":1639000040,"maker_pubkey":"0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","secret_hash":"4da9e7080175e8e10842e0e161b33cd298cab30b","maker_coin_swap_contract_addr":null,"taker_coin_swap_contract_addr":null}}},{"timestamp":1638984457215,"event":{"type":"TakerFeeSent","data":{"tx_hex":"0400008085202f89016383e8aced2256378bb126a1ca1a41e2f344d9295f65b3ea4b99055c5eb4a6cb000000006a47304402201c7e661e0dbeb9b3eb6e4e9e3194010e5772227017772b2e48c1b8d48ed3b21f02201c2eda64e74455fa1878a5c221f25d22fe626abd0078a26a9fc0f829e0921639012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff02bcf60100000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac74c3e90b000000001976a91483762a373935ca241d557dfce89171d582b486de88ac08ebb061000000000000000000000000000000","tx_hash":"fcb49167c79e8e014143643b94878866f7e80b26c5a5dcf693010543da70b5bc"}}},{"timestamp":1638984457230,"event":{"type":"MakerPaymentReceived","data":{"tx_hex":"0400008085202f8901c41fdf6b9d8aea4b472f83e4fa0d99dfafc245e897d681fd2ca7df30707fbf48020000006b483045022100c7b294bd46cbf3b13530879a43c5cf67414047266d8b64c3c7263b5e75b989ba02201974f38d688b184bc44e628806c6ab2ac9092f394729d0ce838f14e1e76117c001210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ffffffff03a2296b050000000017a91491c45f69e1760c12a1f90fb2a811f6dfde35cc35870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30bac503d64000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac09ebb061000000000000000000000000000000","tx_hash":"6287e0d30951cd859bfb837eb1e5409f7596e75ffeb2e61fd6df1843bfd0203d"}}},{"timestamp":1638984457230,"event":{"type":"MakerPaymentWaitConfirmStarted"}},{"timestamp":1638984504262,"event":{"type":"MakerPaymentValidatedAndConfirmed"}},{"timestamp":1638984504262,"event":{"type":"TakerPaymentSent","data":{"tx_hex":"0400008085202f8901bcb570da43050193f6dca5c5260be8f7668887943b644341018e9ec76791b4fc010000006b483045022100fe6c90568a256b531bcd18321c15b3ce68c2d5d603768dea6aba68dcc170b801022076a34c006a92786bcdee6a1dcb46947fb49911e4f51ec27e880c3396d64d59b2012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff0300e1f5050000000017a9145a6125d597d2ce37bde9983d4d9d481335139bc4870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30b8cdef305000000001976a91483762a373935ca241d557dfce89171d582b486de88ac37ebb061000000000000000000000000000000","tx_hash":"667dea45dbd36e1f15ee8170e73e2afefd0a15d47d3bcd65655ca71d838445a2"}}},{"timestamp":1638984594337,"event":{"type":"TakerPaymentSpent","data":{"transaction":{"tx_hex":"0400008085202f8901a24584831da75c6565cd3b7dd4150afdfe2a3ee77081ee151f6ed3db45ea7d6600000000d74730440220422edb8ef5cd3991eb309c3a4fa5fe5d9ffe08d3d6b4b789c5587061d7993864022049cd082398f5a37a9e7411d56976e61bcce9162d0f5f1fb24e40bcf2f4ec0052012023a6bb64bc0ab2cc14cb84277d8d25134b814e5f999c66e578c9bba3c5e2d3a4004c6b63047009b161b1752103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddac6782012088a9144da9e7080175e8e10842e0e161b33cd298cab30b88210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ac68ffffffff0118ddf505000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac7009b161000000000000000000000000000000","tx_hash":"ab1eb5b65a302370af2607e0b64b60fc04360de33a87799bca1dcf337344b616"},"secret":"23a6bb64bc0ab2cc14cb84277d8d25134b814e5f999c66e578c9bba3c5e2d3a4"}}}],"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","maker_coin":"RICK","taker_amount":"1","taker_coin":"MORTY","gui":null,"mm_version":"","success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","MakerPaymentWaitConfirmFailed","TakerPaymentTransactionFailed","TakerPaymentWaitConfirmFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentWaitRefundStarted","TakerPaymentRefunded","TakerPaymentRefundFailed"]}"#;

        let maker_saved_swap: MakerSavedSwap = json::from_str(maker_saved_json).unwrap();
        let taker_expected_swap: TakerSavedSwap = json::from_str(taker_expected_json).unwrap();

        let ctx = MmCtxBuilder::default().into_mm_arc();
        let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
        block_on(coins_ctx.add_coin(MmCoinEnum::Test(TestCoin::new("RICK")))).unwrap();

        let taker_actual_swap = block_on(recreate_taker_swap(ctx, maker_saved_swap)).expect("!recreate_maker_swap");
        println!("{}", json::to_string(&taker_actual_swap).unwrap());
        assert_eq!(taker_actual_swap, taker_expected_swap);
    }

    #[test]
    fn test_recreate_taker_swap_taker_payment_wait_confirm_failed() {
        let maker_saved_json = r#"{"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","my_order_uuid":"15d007fa-9237-489c-82a7-df061deba95f","events":[{"timestamp":1638984440198,"event":{"type":"Started","data":{"taker_coin":"MORTY","maker_coin":"RICK","taker":"b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","secret":"0000000000000000000000000000000000000000000000000000000000000000","secret_hash":"4da9e7080175e8e10842e0e161b33cd298cab30b","my_persistent_pub":"0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","lock_duration":7800,"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","taker_amount":"1","maker_payment_confirmations":1,"maker_payment_requires_nota":false,"taker_payment_confirmations":1,"taker_payment_requires_nota":false,"maker_payment_lock":1639000040,"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","started_at":1638984440,"maker_coin_start_block":1207822,"taker_coin_start_block":1222573,"maker_payment_trade_fee":{"coin":"RICK","amount":"0.00001","paid_from_trading_vol":false},"taker_payment_spend_trade_fee":{"coin":"MORTY","amount":"0.00001","paid_from_trading_vol":true}}}},{"timestamp":1638984456204,"event":{"type":"Negotiated","data":{"taker_payment_locktime":1638992240,"taker_pubkey":"03b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","maker_coin_swap_contract_addr":null,"taker_coin_swap_contract_addr":null}}},{"timestamp":1638984457215,"event":{"type":"TakerFeeValidated","data":{"tx_hex":"0400008085202f89016383e8aced2256378bb126a1ca1a41e2f344d9295f65b3ea4b99055c5eb4a6cb000000006a47304402201c7e661e0dbeb9b3eb6e4e9e3194010e5772227017772b2e48c1b8d48ed3b21f02201c2eda64e74455fa1878a5c221f25d22fe626abd0078a26a9fc0f829e0921639012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff02bcf60100000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac74c3e90b000000001976a91483762a373935ca241d557dfce89171d582b486de88ac08ebb061000000000000000000000000000000","tx_hash":"fcb49167c79e8e014143643b94878866f7e80b26c5a5dcf693010543da70b5bc"}}},{"timestamp":1638984457230,"event":{"type":"MakerPaymentSent","data":{"tx_hex":"0400008085202f8901c41fdf6b9d8aea4b472f83e4fa0d99dfafc245e897d681fd2ca7df30707fbf48020000006b483045022100c7b294bd46cbf3b13530879a43c5cf67414047266d8b64c3c7263b5e75b989ba02201974f38d688b184bc44e628806c6ab2ac9092f394729d0ce838f14e1e76117c001210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ffffffff03a2296b050000000017a91491c45f69e1760c12a1f90fb2a811f6dfde35cc35870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30bac503d64000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac09ebb061000000000000000000000000000000","tx_hash":"6287e0d30951cd859bfb837eb1e5409f7596e75ffeb2e61fd6df1843bfd0203d"}}},{"timestamp":1638984504262,"event":{"type":"TakerPaymentReceived","data":{"tx_hex":"0400008085202f8901bcb570da43050193f6dca5c5260be8f7668887943b644341018e9ec76791b4fc010000006b483045022100fe6c90568a256b531bcd18321c15b3ce68c2d5d603768dea6aba68dcc170b801022076a34c006a92786bcdee6a1dcb46947fb49911e4f51ec27e880c3396d64d59b2012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff0300e1f5050000000017a9145a6125d597d2ce37bde9983d4d9d481335139bc4870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30b8cdef305000000001976a91483762a373935ca241d557dfce89171d582b486de88ac37ebb061000000000000000000000000000000","tx_hash":"667dea45dbd36e1f15ee8170e73e2afefd0a15d47d3bcd65655ca71d838445a2"}}},{"timestamp":1638984504263,"event":{"type":"TakerPaymentWaitConfirmStarted"}},{"timestamp":1638984594319,"event":{"type":"TakerPaymentWaitConfirmFailed","data":{"error":"An error"}}},{"timestamp":1638984594338,"event":{"type":"MakerPaymentWaitRefundStarted","data":{"wait_until":1639003740}}},{"timestamp":1639003740392,"event":{"type":"MakerPaymentRefunded","data":{"tx_hex":"0400008085202f8901bcb570da43050193f6dca5c5260be8f7668887943b644341018e9ec76791b4fc010000006b483045022100fe6c90568a256b531bcd18321c15b3ce68c2d5d603768dea6aba68dcc170b801022076a34c006a92786bcdee6a1dcb46947fb49911e4f51ec27e880c3396d64d59b2012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff0300e1f5050000000017a9145a6125d597d2ce37bde9983d4d9d481335139bc4870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30b8cdef305000000001976a91483762a373935ca241d557dfce89171d582b486de88ac37ebb061000000000000000000000000000000","tx_hash":"667dea45dbd36e1f15ee8170e73e2afefd0a15d47d3bcd65655ca71d838445a2"}}},{"timestamp":1639003740398,"event":{"type":"Finished"}}],"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","maker_coin":"RICK","taker_amount":"1","taker_coin":"MORTY","gui":"mpm","mm_version":"213bfddd5","success_events":["Started","Negotiated","TakerFeeValidated","MakerPaymentSent","TakerPaymentReceived","TakerPaymentWaitConfirmStarted","TakerPaymentValidatedAndConfirmed","TakerPaymentSpent","TakerPaymentSpendConfirmStarted","TakerPaymentSpendConfirmed","Finished"],"error_events":["StartFailed","NegotiateFailed","TakerFeeValidateFailed","MakerPaymentTransactionFailed","MakerPaymentDataSendFailed","MakerPaymentWaitConfirmFailed","TakerPaymentValidateFailed","TakerPaymentWaitConfirmFailed","TakerPaymentSpendFailed","TakerPaymentSpendConfirmFailed","MakerPaymentWaitRefundStarted","MakerPaymentRefunded","MakerPaymentRefundFailed"]}"#;
        let taker_expected_json = r#"{"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","my_order_uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","events":[{"timestamp":1638984440198,"event":{"type":"Started","data":{"taker_coin":"MORTY","maker_coin":"RICK","maker":"15d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","my_persistent_pub":"03b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","lock_duration":7800,"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","taker_amount":"1","maker_payment_confirmations":1,"maker_payment_requires_nota":false,"taker_payment_confirmations":1,"taker_payment_requires_nota":false,"taker_payment_lock":1638992240,"uuid":"f87fa9ce-0820-4675-b85d-db18c7bc9fb4","started_at":1638984440,"maker_payment_wait":1638987560,"maker_coin_start_block":1207822,"taker_coin_start_block":1222573,"fee_to_send_taker_fee":null,"taker_payment_trade_fee":null,"maker_payment_spend_trade_fee":null}}},{"timestamp":1638984456204,"event":{"type":"Negotiated","data":{"maker_payment_locktime":1639000040,"maker_pubkey":"0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732","secret_hash":"4da9e7080175e8e10842e0e161b33cd298cab30b","maker_coin_swap_contract_addr":null,"taker_coin_swap_contract_addr":null}}},{"timestamp":1638984457215,"event":{"type":"TakerFeeSent","data":{"tx_hex":"0400008085202f89016383e8aced2256378bb126a1ca1a41e2f344d9295f65b3ea4b99055c5eb4a6cb000000006a47304402201c7e661e0dbeb9b3eb6e4e9e3194010e5772227017772b2e48c1b8d48ed3b21f02201c2eda64e74455fa1878a5c221f25d22fe626abd0078a26a9fc0f829e0921639012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff02bcf60100000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac74c3e90b000000001976a91483762a373935ca241d557dfce89171d582b486de88ac08ebb061000000000000000000000000000000","tx_hash":"fcb49167c79e8e014143643b94878866f7e80b26c5a5dcf693010543da70b5bc"}}},{"timestamp":1638984457230,"event":{"type":"MakerPaymentReceived","data":{"tx_hex":"0400008085202f8901c41fdf6b9d8aea4b472f83e4fa0d99dfafc245e897d681fd2ca7df30707fbf48020000006b483045022100c7b294bd46cbf3b13530879a43c5cf67414047266d8b64c3c7263b5e75b989ba02201974f38d688b184bc44e628806c6ab2ac9092f394729d0ce838f14e1e76117c001210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ffffffff03a2296b050000000017a91491c45f69e1760c12a1f90fb2a811f6dfde35cc35870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30bac503d64000000001976a9141462c3dd3f936d595c9af55978003b27c250441f88ac09ebb061000000000000000000000000000000","tx_hash":"6287e0d30951cd859bfb837eb1e5409f7596e75ffeb2e61fd6df1843bfd0203d"}}},{"timestamp":1638984457230,"event":{"type":"MakerPaymentWaitConfirmStarted"}},{"timestamp":1638984504262,"event":{"type":"MakerPaymentValidatedAndConfirmed"}},{"timestamp":1638984504262,"event":{"type":"TakerPaymentSent","data":{"tx_hex":"0400008085202f8901bcb570da43050193f6dca5c5260be8f7668887943b644341018e9ec76791b4fc010000006b483045022100fe6c90568a256b531bcd18321c15b3ce68c2d5d603768dea6aba68dcc170b801022076a34c006a92786bcdee6a1dcb46947fb49911e4f51ec27e880c3396d64d59b2012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff0300e1f5050000000017a9145a6125d597d2ce37bde9983d4d9d481335139bc4870000000000000000166a144da9e7080175e8e10842e0e161b33cd298cab30b8cdef305000000001976a91483762a373935ca241d557dfce89171d582b486de88ac37ebb061000000000000000000000000000000","tx_hash":"667dea45dbd36e1f15ee8170e73e2afefd0a15d47d3bcd65655ca71d838445a2"}}},{"timestamp":1638984594319,"event":{"type":"TakerPaymentWaitForSpendFailed","data":{"error":"Origin Maker error event: TakerPaymentWaitConfirmFailed(SwapError { error: \"An error\" })"}}},{"timestamp":1638984594319,"event":{"type":"TakerPaymentWaitRefundStarted","data":{"wait_until":1638995940}}}],"maker_amount":"0.9090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909090909091","maker_coin":"RICK","taker_amount":"1","taker_coin":"MORTY","gui":null,"mm_version":"","success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","MakerPaymentWaitConfirmFailed","TakerPaymentTransactionFailed","TakerPaymentWaitConfirmFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentWaitRefundStarted","TakerPaymentRefunded","TakerPaymentRefundFailed"]}"#;

        let maker_saved_swap: MakerSavedSwap = json::from_str(maker_saved_json).unwrap();
        let taker_expected_swap: TakerSavedSwap = json::from_str(taker_expected_json).unwrap();

        let ctx = MmCtxBuilder::default().into_mm_arc();
        let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
        block_on(coins_ctx.add_coin(MmCoinEnum::Test(TestCoin::new("RICK")))).unwrap();

        let taker_actual_swap = block_on(recreate_taker_swap(ctx, maker_saved_swap)).expect("!recreate_maker_swap");
        println!("{}", json::to_string(&taker_actual_swap).unwrap());
        assert_eq!(taker_actual_swap, taker_expected_swap);
    }
}

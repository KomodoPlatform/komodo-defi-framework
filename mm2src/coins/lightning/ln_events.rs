use super::*;
use crate::lightning::ln_db::{DBChannelDetails, HTLCStatus, LightningDB, PaymentType};
use crate::lightning::ln_errors::{SaveChannelClosingError, SaveChannelClosingResult};
use crate::lightning::ln_sql::SqliteLightningDB;
use bitcoin::blockdata::script::Script;
use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::encode::serialize_hex;
use common::executor::{spawn, Timer};
use common::log::{error, info};
use common::{now_ms, spawn_abortable, AbortOnDropHandle};
use core::time::Duration;
use lightning::chain::chaininterface::{ConfirmationTarget, FeeEstimator};
use lightning::chain::keysinterface::SpendableOutputDescriptor;
use lightning::util::events::{Event, EventHandler, PaymentPurpose};
use parking_lot::Mutex as PaMutex;
use rand::Rng;
use script::{Builder, SignatureVersion};
use secp256k1::Secp256k1;
use std::convert::TryFrom;
use std::sync::Arc;
use utxo_signer::with_key_pair::sign_tx;

const TRY_LOOP_INTERVAL: f64 = 60.;

pub struct LightningEventHandler {
    platform: Arc<Platform>,
    channel_manager: Arc<ChannelManager>,
    keys_manager: Arc<KeysManager>,
    db: SqliteLightningDB,
    abort_handlers: Arc<PaMutex<Vec<AbortOnDropHandle>>>,
}

impl EventHandler for LightningEventHandler {
    fn handle_event(&self, event: &Event) {
        match event {
            Event::FundingGenerationReady {
                temporary_channel_id,
                channel_value_satoshis,
                output_script,
                user_channel_id,
            } => self.handle_funding_generation_ready(
                *temporary_channel_id,
                *channel_value_satoshis,
                output_script,
                *user_channel_id,
            ),

            Event::PaymentReceived {
                payment_hash,
                amt,
                purpose,
            } => self.handle_payment_received(*payment_hash, *amt, purpose),

            Event::PaymentSent {
                payment_preimage,
                payment_hash,
                fee_paid_msat,
                ..
            } => self.handle_payment_sent(*payment_preimage, *payment_hash, *fee_paid_msat),

            Event::PaymentFailed { payment_hash, .. } => self.handle_payment_failed(*payment_hash),

            Event::PendingHTLCsForwardable { time_forwardable } => self.handle_pending_htlcs_forwards(*time_forwardable),

            Event::SpendableOutputs { outputs } => self.handle_spendable_outputs(outputs),

            // Todo: an RPC for total amount earned
            Event::PaymentForwarded { fee_earned_msat, claim_from_onchain_tx } => info!(
                "Received a fee of {} milli-satoshis for a successfully forwarded payment through our {} lightning node. Was the forwarded HTLC claimed by our counterparty via an on-chain transaction?: {}",
                fee_earned_msat.unwrap_or_default(),
                self.platform.coin.ticker(),
                claim_from_onchain_tx,
            ),

            Event::ChannelClosed {
                channel_id,
                user_channel_id,
                reason,
            } => self.handle_channel_closed(*channel_id, *user_channel_id, reason.to_string()),

            // Todo: Add spent UTXOs to RecentlySpentOutPoints if it's not discarded
            Event::DiscardFunding { channel_id, transaction } => info!(
                "Discarding funding tx: {} for channel {}",
                transaction.txid().to_string(),
                hex::encode(channel_id),
            ),

            // Handling updating channel penalties after successfully routing a payment along a path is done by the InvoicePayer.
            Event::PaymentPathSuccessful {
                payment_id,
                payment_hash,
                path,
            } => info!(
                "Payment path: {:?}, successful for payment hash: {}, payment id: {}",
                path.iter().map(|hop| hop.pubkey.to_string()).collect::<Vec<_>>(),
                payment_hash.map(|h| hex::encode(h.0)).unwrap_or_default(),
                hex::encode(payment_id.0)
            ),

            // Handling updating channel penalties after a payment fails to route through a channel is done by the InvoicePayer.
            // Also abandoning or retrying a payment is handled by the InvoicePayer. 
            Event::PaymentPathFailed {
                payment_hash,
                rejected_by_dest,
                all_paths_failed,
                path,
                ..
            } => info!(
                "Payment path: {:?}, failed for payment hash: {}, Was rejected by destination?: {}, All paths failed?: {}",
                path.iter().map(|hop| hop.pubkey.to_string()).collect::<Vec<_>>(),
                hex::encode(payment_hash.0),
                rejected_by_dest,
                all_paths_failed,
            ),

            Event::OpenChannelRequest {
                temporary_channel_id,
                counterparty_node_id,
                funding_satoshis,
                push_msat,
                channel_type: _,
            } => self.handle_open_channel_request(*temporary_channel_id, *counterparty_node_id, *funding_satoshis, *push_msat),
        }
    }
}

pub async fn init_events_abort_handlers(
    platform: Arc<Platform>,
    db: SqliteLightningDB,
) -> EnableLightningResult<Arc<PaMutex<Vec<AbortOnDropHandle>>>> {
    let abort_handlers = Arc::new(PaMutex::new(Vec::new()));
    let closed_channels_without_closing_tx = db.get_closed_channels_with_no_closing_tx().await?;
    for channel_details in closed_channels_without_closing_tx {
        let platform = platform.clone();
        let db = db.clone();
        let user_channel_id = channel_details.rpc_id;
        let abort_handler = spawn_abortable(async move {
            if let Ok(closing_tx_hash) = platform
                .get_channel_closing_tx(channel_details)
                .await
                .error_log_passthrough()
            {
                if let Err(e) = db.add_closing_tx_to_db(user_channel_id, closing_tx_hash).await {
                    log::error!(
                        "Unable to update channel {} closing details in DB: {}",
                        user_channel_id,
                        e
                    );
                }
            }
        });
        abort_handlers.lock().push(abort_handler);
    }
    Ok(abort_handlers)
}

// Generates the raw funding transaction with one output equal to the channel value.
fn sign_funding_transaction(
    user_channel_id: u64,
    output_script: &Script,
    platform: Arc<Platform>,
) -> OpenChannelResult<Transaction> {
    let coin = &platform.coin;
    let mut unsigned = {
        let unsigned_funding_txs = platform.unsigned_funding_txs.lock();
        unsigned_funding_txs
            .get(&user_channel_id)
            .ok_or_else(|| {
                OpenChannelError::InternalError(format!(
                    "Unsigned funding tx not found for internal channel id: {}",
                    user_channel_id
                ))
            })?
            .clone()
    };
    unsigned.outputs[0].script_pubkey = output_script.to_bytes().into();

    let my_address = coin.as_ref().derivation_method.iguana_or_err()?;
    let key_pair = coin.as_ref().priv_key_policy.key_pair_or_err()?;

    let prev_script = Builder::build_p2pkh(&my_address.hash);
    let signed = sign_tx(
        unsigned,
        key_pair,
        prev_script,
        SignatureVersion::WitnessV0,
        coin.as_ref().conf.fork_id,
    )?;

    Transaction::try_from(signed).map_to_mm(|e| OpenChannelError::ConvertTxErr(e.to_string()))
}

async fn save_channel_closing_details(
    db: SqliteLightningDB,
    platform: Arc<Platform>,
    user_channel_id: u64,
    reason: String,
) -> SaveChannelClosingResult<()> {
    db.update_channel_to_closed(user_channel_id as i64, reason, (now_ms() / 1000) as i64)
        .await?;

    let channel_details = db
        .get_channel_from_db(user_channel_id)
        .await?
        .ok_or_else(|| MmError::new(SaveChannelClosingError::ChannelNotFound(user_channel_id)))?;

    let closing_tx_hash = platform.get_channel_closing_tx(channel_details).await?;

    db.add_closing_tx_to_db(user_channel_id as i64, closing_tx_hash).await?;

    Ok(())
}

async fn add_claiming_tx_to_db_loop(
    db: SqliteLightningDB,
    closing_txid: String,
    claiming_txid: String,
    claimed_balance: f64,
) {
    while let Err(e) = db
        .add_claiming_tx_to_db(closing_txid.clone(), claiming_txid.clone(), claimed_balance)
        .await
    {
        error!("error {}", e);
        Timer::sleep(TRY_LOOP_INTERVAL).await;
    }
}

impl LightningEventHandler {
    pub fn new(
        platform: Arc<Platform>,
        channel_manager: Arc<ChannelManager>,
        keys_manager: Arc<KeysManager>,
        db: SqliteLightningDB,
        abort_handlers: Arc<PaMutex<Vec<AbortOnDropHandle>>>,
    ) -> Self {
        LightningEventHandler {
            platform,
            channel_manager,
            keys_manager,
            db,
            abort_handlers,
        }
    }

    fn handle_funding_generation_ready(
        &self,
        temporary_channel_id: [u8; 32],
        channel_value_satoshis: u64,
        output_script: &Script,
        user_channel_id: u64,
    ) {
        info!(
            "Handling FundingGenerationReady event for internal channel id: {}",
            user_channel_id
        );
        let funding_tx = match sign_funding_transaction(user_channel_id, output_script, self.platform.clone()) {
            Ok(tx) => tx,
            Err(e) => {
                error!(
                    "Error generating funding transaction for internal channel id {}: {}",
                    user_channel_id,
                    e.to_string()
                );
                return;
            },
        };
        let funding_txid = funding_tx.txid();
        // Give the funding transaction back to LDK for opening the channel.
        if let Err(e) = self
            .channel_manager
            .funding_transaction_generated(&temporary_channel_id, funding_tx)
        {
            error!("{:?}", e);
            return;
        }
        let platform = self.platform.clone();
        let db = self.db.clone();
        spawn(async move {
            let best_block_height = platform.best_block_height();
            db.add_funding_tx_to_db(
                user_channel_id as i64,
                funding_txid.to_string(),
                channel_value_satoshis as i64,
                best_block_height as i64,
            )
            .await
            .error_log();
        });
    }

    fn handle_payment_received(&self, payment_hash: PaymentHash, amt: u64, purpose: &PaymentPurpose) {
        info!(
            "Handling PaymentReceived event for payment_hash: {}",
            hex::encode(payment_hash.0)
        );
        let (payment_preimage, payment_secret) = match purpose {
            PaymentPurpose::InvoicePayment {
                payment_preimage,
                payment_secret,
            } => match payment_preimage {
                Some(preimage) => (*preimage, Some(*payment_secret)),
                None => return,
            },
            PaymentPurpose::SpontaneousPayment(preimage) => (*preimage, None),
        };
        let status = match self.channel_manager.claim_funds(payment_preimage) {
            true => {
                info!(
                    "Received an amount of {} millisatoshis for payment hash {}",
                    amt,
                    hex::encode(payment_hash.0)
                );
                HTLCStatus::Succeeded
            },
            false => HTLCStatus::Failed,
        };
        let db = self.db.clone();
        match purpose {
            PaymentPurpose::InvoicePayment { .. } => spawn(async move {
                if let Ok(Some(mut payment_info)) = db.get_payment_from_db(payment_hash).await.error_log_passthrough() {
                    payment_info.preimage = Some(payment_preimage);
                    payment_info.status = HTLCStatus::Succeeded;
                    payment_info.amt_msat = Some(amt as i64);
                    payment_info.last_updated = (now_ms() / 1000) as i64;
                    if let Err(e) = db.add_or_update_payment_in_db(payment_info).await {
                        error!("Unable to update payment information in DB: {}", e);
                    }
                }
            }),
            PaymentPurpose::SpontaneousPayment(_) => {
                let payment_info = DBPaymentInfo {
                    payment_hash,
                    payment_type: PaymentType::InboundPayment,
                    description: "".into(),
                    preimage: Some(payment_preimage),
                    secret: payment_secret,
                    amt_msat: Some(amt as i64),
                    fee_paid_msat: None,
                    status,
                    created_at: (now_ms() / 1000) as i64,
                    last_updated: (now_ms() / 1000) as i64,
                };
                spawn(async move {
                    if let Err(e) = db.add_or_update_payment_in_db(payment_info).await {
                        error!("Unable to update payment information in DB: {}", e);
                    }
                });
            },
        }
    }

    fn handle_payment_sent(
        &self,
        payment_preimage: PaymentPreimage,
        payment_hash: PaymentHash,
        fee_paid_msat: Option<u64>,
    ) {
        info!(
            "Handling PaymentSent event for payment_hash: {}",
            hex::encode(payment_hash.0)
        );
        let db = self.db.clone();
        spawn(async move {
            if let Ok(Some(mut payment_info)) = db.get_payment_from_db(payment_hash).await.error_log_passthrough() {
                payment_info.preimage = Some(payment_preimage);
                payment_info.status = HTLCStatus::Succeeded;
                payment_info.fee_paid_msat = fee_paid_msat.map(|f| f as i64);
                payment_info.last_updated = (now_ms() / 1000) as i64;
                let amt_msat = payment_info.amt_msat;
                if let Err(e) = db.add_or_update_payment_in_db(payment_info).await {
                    error!("Unable to update payment information in DB: {}", e);
                }
                info!(
                    "Successfully sent payment of {} millisatoshis with payment hash {}",
                    amt_msat.unwrap_or_default(),
                    hex::encode(payment_hash.0)
                );
            }
        });
    }

    fn handle_channel_closed(&self, channel_id: [u8; 32], user_channel_id: u64, reason: String) {
        info!(
            "Channel: {} closed for the following reason: {}",
            hex::encode(channel_id),
            reason
        );
        let db = self.db.clone();
        let platform = self.platform.clone();
        let abort_handler = spawn_abortable(async move {
            if let Err(e) = save_channel_closing_details(db, platform, user_channel_id, reason).await {
                // This is the case when a channel is closed before funding is broadcasted due to the counterparty disconnecting or other incompatibility issue.
                if e != SaveChannelClosingError::FundingTxNull.into() {
                    error!(
                        "Unable to update channel {} closing details in DB: {}",
                        user_channel_id, e
                    );
                }
            }
        });
        self.abort_handlers.lock().push(abort_handler);
    }

    fn handle_payment_failed(&self, payment_hash: PaymentHash) {
        info!(
            "Handling PaymentFailed event for payment_hash: {}",
            hex::encode(payment_hash.0)
        );
        let db = self.db.clone();
        spawn(async move {
            if let Ok(Some(mut payment_info)) = db.get_payment_from_db(payment_hash).await.error_log_passthrough() {
                payment_info.status = HTLCStatus::Failed;
                payment_info.last_updated = (now_ms() / 1000) as i64;
                if let Err(e) = db.add_or_update_payment_in_db(payment_info).await {
                    error!("Unable to update payment information in DB: {}", e);
                }
            }
        });
    }

    fn handle_pending_htlcs_forwards(&self, time_forwardable: Duration) {
        info!("Handling PendingHTLCsForwardable event!");
        let min_wait_time = time_forwardable.as_millis() as u32;
        let channel_manager = self.channel_manager.clone();
        spawn(async move {
            let millis_to_sleep = rand::thread_rng().gen_range(min_wait_time, min_wait_time * 5);
            Timer::sleep_ms(millis_to_sleep).await;
            channel_manager.process_pending_htlc_forwards();
        });
    }

    fn handle_spendable_outputs(&self, outputs: &[SpendableOutputDescriptor]) {
        info!("Handling SpendableOutputs event!");
        let platform_coin = &self.platform.coin;
        // Todo: add support for Hardware wallets for funding transactions and spending spendable outputs (channel closing transactions)
        let my_address = match platform_coin.as_ref().derivation_method.iguana_or_err() {
            Ok(addr) => addr,
            Err(e) => {
                error!("{}", e);
                return;
            },
        };
        let change_destination_script = Builder::build_witness_script(&my_address.hash).to_bytes().take().into();
        let feerate_sat_per_1000_weight = self.platform.get_est_sat_per_1000_weight(ConfirmationTarget::Normal);
        let output_descriptors = &outputs.iter().collect::<Vec<_>>();
        let claiming_tx = match self.keys_manager.spend_spendable_outputs(
            output_descriptors,
            Vec::new(),
            change_destination_script,
            feerate_sat_per_1000_weight,
            &Secp256k1::new(),
        ) {
            Ok(tx) => tx,
            Err(_) => {
                error!("Error spending spendable outputs");
                return;
            },
        };

        let claiming_txid = claiming_tx.txid();
        let tx_hex = serialize_hex(&claiming_tx);
        if let Err(e) = tokio::task::block_in_place(move || self.platform.coin.send_raw_tx(&tx_hex).wait()) {
            // TODO: broadcast transaction through p2p network in this case
            error!(
                "Broadcasting of the claiming transaction {} failed: {}",
                claiming_txid, e
            );
            return;
        }

        let claiming_tx_inputs_value = outputs.iter().fold(0, |sum, output| match output {
            SpendableOutputDescriptor::StaticOutput { output, .. } => sum + output.value,
            SpendableOutputDescriptor::DelayedPaymentOutput(descriptor) => sum + descriptor.output.value,
            SpendableOutputDescriptor::StaticPaymentOutput(descriptor) => sum + descriptor.output.value,
        });
        let claiming_tx_outputs_value = claiming_tx.output.iter().fold(0, |sum, txout| sum + txout.value);
        if claiming_tx_inputs_value < claiming_tx_outputs_value {
            error!(
                "Claiming transaction input value {} can't be less than outputs value {}!",
                claiming_tx_inputs_value, claiming_tx_outputs_value
            );
            return;
        }
        let claiming_tx_fee = claiming_tx_inputs_value - claiming_tx_outputs_value;
        let claiming_tx_fee_per_channel = (claiming_tx_fee as f64) / (outputs.len() as f64);

        for output in outputs {
            let (closing_txid, claimed_balance) = match output {
                SpendableOutputDescriptor::StaticOutput { outpoint, output } => {
                    (outpoint.txid.to_string(), output.value)
                },
                SpendableOutputDescriptor::DelayedPaymentOutput(descriptor) => {
                    (descriptor.outpoint.txid.to_string(), descriptor.output.value)
                },
                SpendableOutputDescriptor::StaticPaymentOutput(descriptor) => {
                    (descriptor.outpoint.txid.to_string(), descriptor.output.value)
                },
            };
            let db = self.db.clone();

            // This doesn't need to be respawned on restart unlike add_closing_tx_to_db since Event::SpendableOutputs will be re-fired on restart
            // if the spending_tx is not broadcasted.
            let abort_handler = spawn_abortable(add_claiming_tx_to_db_loop(
                db,
                closing_txid,
                claiming_txid.to_string(),
                (claimed_balance as f64) - claiming_tx_fee_per_channel,
            ));
            self.abort_handlers.lock().push(abort_handler);
        }
    }

    fn handle_open_channel_request(
        &self,
        temporary_channel_id: [u8; 32],
        counterparty_node_id: PublicKey,
        funding_satoshis: u64,
        push_msat: u64,
    ) {
        info!(
            "Handling OpenChannelRequest from node: {} with funding value: {} and starting balance: {}",
            counterparty_node_id, funding_satoshis, push_msat,
        );

        let db = self.db.clone();
        let channel_manager = self.channel_manager.clone();
        let platform = self.platform.clone();
        spawn(async move {
            if let Ok(last_channel_rpc_id) = db.get_last_channel_rpc_id().await.error_log_passthrough() {
                let user_channel_id = last_channel_rpc_id as u64 + 1;
                if channel_manager
                    .accept_inbound_channel(&temporary_channel_id, user_channel_id)
                    .is_ok()
                {
                    let is_public = match channel_manager
                        .list_channels()
                        .into_iter()
                        .find(|chan| chan.user_channel_id == user_channel_id)
                    {
                        Some(details) => details.is_public,
                        None => {
                            error!(
                                "Inbound channel {} details should be found by list_channels!",
                                user_channel_id
                            );
                            return;
                        },
                    };

                    let pending_channel_details = DBChannelDetails::new(
                        user_channel_id,
                        temporary_channel_id,
                        counterparty_node_id,
                        false,
                        is_public,
                    );
                    if let Err(e) = db.add_channel_to_db(pending_channel_details).await {
                        error!("Unable to add new inbound channel {} to db: {}", user_channel_id, e);
                    }

                    while let Some(details) = channel_manager
                        .list_channels()
                        .into_iter()
                        .find(|chan| chan.user_channel_id == user_channel_id)
                    {
                        if let Some(funding_tx) = details.funding_txo {
                            let best_block_height = platform.best_block_height();
                            db.add_funding_tx_to_db(
                                user_channel_id as i64,
                                funding_tx.txid.to_string(),
                                funding_satoshis as i64,
                                best_block_height as i64,
                            )
                            .await
                            .error_log();
                            break;
                        }

                        Timer::sleep(TRY_LOOP_INTERVAL).await;
                    }
                }
            }
        });
    }
}

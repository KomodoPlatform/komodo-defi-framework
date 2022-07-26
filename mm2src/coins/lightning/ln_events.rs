use super::*;
use crate::lightning::ln_errors::{SaveChannelClosingError, SaveChannelClosingResult};
use bitcoin::blockdata::script::Script;
use bitcoin::blockdata::transaction::Transaction;
use common::executor::{spawn, Timer};
use common::log::{error, info};
use common::now_ms;
use core::time::Duration;
use lightning::chain::chaininterface::{BroadcasterInterface, ConfirmationTarget, FeeEstimator};
use lightning::chain::keysinterface::SpendableOutputDescriptor;
use lightning::util::events::{Event, EventHandler, PaymentPurpose};
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
    persister: Arc<LightningPersister>,
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
            } => {
                info!(
                    "Handling OpenChannelRequest from node: {} with funding value: {} and starting balance: {}",
                    counterparty_node_id,
                    funding_satoshis,
                    push_msat,
                );
                if self.channel_manager.accept_inbound_channel(temporary_channel_id, 0).is_ok() {
                    // Todo: once the rust-lightning PR for user_channel_id in accept_inbound_channel is released
                    // use user_channel_id to get the funding tx here once the funding tx is available.
                }
            },
        }
    }
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
    persister: Arc<LightningPersister>,
    platform: Arc<Platform>,
    user_channel_id: u64,
    reason: String,
) -> SaveChannelClosingResult<()> {
    persister
        .update_channel_to_closed(user_channel_id, reason, now_ms() / 1000)
        .await?;

    let channel_details = persister
        .get_channel_from_db(user_channel_id)
        .await?
        .ok_or_else(|| MmError::new(SaveChannelClosingError::ChannelNotFound(user_channel_id)))?;

    let closing_tx_hash = platform.get_channel_closing_tx(channel_details).await?;

    persister.add_closing_tx_to_db(user_channel_id, closing_tx_hash).await?;

    Ok(())
}

impl LightningEventHandler {
    pub fn new(
        platform: Arc<Platform>,
        channel_manager: Arc<ChannelManager>,
        keys_manager: Arc<KeysManager>,
        persister: Arc<LightningPersister>,
    ) -> Self {
        LightningEventHandler {
            platform,
            channel_manager,
            keys_manager,
            persister,
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
        let persister = self.persister.clone();
        spawn(async move {
            let best_block_height = platform.best_block_height();
            persister
                .add_funding_tx_to_db(
                    user_channel_id,
                    funding_txid.to_string(),
                    channel_value_satoshis,
                    best_block_height,
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
        let persister = self.persister.clone();
        match purpose {
            PaymentPurpose::InvoicePayment { .. } => spawn(async move {
                if let Ok(Some(mut payment_info)) = persister
                    .get_payment_from_db(payment_hash)
                    .await
                    .error_log_passthrough()
                {
                    payment_info.preimage = Some(payment_preimage);
                    payment_info.status = HTLCStatus::Succeeded;
                    payment_info.amt_msat = Some(amt);
                    payment_info.last_updated = now_ms() / 1000;
                    if let Err(e) = persister.add_or_update_payment_in_db(payment_info).await {
                        error!("Unable to update payment information in DB: {}", e);
                    }
                }
            }),
            PaymentPurpose::SpontaneousPayment(_) => {
                let payment_info = PaymentInfo {
                    payment_hash,
                    payment_type: PaymentType::InboundPayment,
                    description: "".into(),
                    preimage: Some(payment_preimage),
                    secret: payment_secret,
                    amt_msat: Some(amt),
                    fee_paid_msat: None,
                    status,
                    created_at: now_ms() / 1000,
                    last_updated: now_ms() / 1000,
                };
                spawn(async move {
                    if let Err(e) = persister.add_or_update_payment_in_db(payment_info).await {
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
        let persister = self.persister.clone();
        spawn(async move {
            if let Ok(Some(mut payment_info)) = persister
                .get_payment_from_db(payment_hash)
                .await
                .error_log_passthrough()
            {
                payment_info.preimage = Some(payment_preimage);
                payment_info.status = HTLCStatus::Succeeded;
                payment_info.fee_paid_msat = fee_paid_msat;
                payment_info.last_updated = now_ms() / 1000;
                let amt_msat = payment_info.amt_msat;
                if let Err(e) = persister.add_or_update_payment_in_db(payment_info).await {
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
        let persister = self.persister.clone();
        let platform = self.platform.clone();
        // Todo: Handle inbound channels closure case after updating to latest version of rust-lightning
        // as it has a new OpenChannelRequest event where we can give an inbound channel a user_channel_id
        // other than 0 in sql
        if user_channel_id != 0 {
            spawn(async move {
                if let Err(e) = save_channel_closing_details(persister, platform, user_channel_id, reason).await {
                    error!(
                        "Unable to update channel {} closing details in DB: {}",
                        user_channel_id, e
                    );
                }
            });
        }
    }

    fn handle_payment_failed(&self, payment_hash: PaymentHash) {
        info!(
            "Handling PaymentFailed event for payment_hash: {}",
            hex::encode(payment_hash.0)
        );
        let persister = self.persister.clone();
        spawn(async move {
            if let Ok(Some(mut payment_info)) = persister
                .get_payment_from_db(payment_hash)
                .await
                .error_log_passthrough()
            {
                payment_info.status = HTLCStatus::Failed;
                payment_info.last_updated = now_ms() / 1000;
                if let Err(e) = persister.add_or_update_payment_in_db(payment_info).await {
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
        let spending_tx = match self.keys_manager.spend_spendable_outputs(
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

        let claiming_tx_inputs_value = outputs.iter().fold(0, |sum, output| match output {
            SpendableOutputDescriptor::StaticOutput { output, .. } => sum + output.value,
            SpendableOutputDescriptor::DelayedPaymentOutput(descriptor) => sum + descriptor.output.value,
            SpendableOutputDescriptor::StaticPaymentOutput(descriptor) => sum + descriptor.output.value,
        });
        let claiming_tx_outputs_value = spending_tx.output.iter().fold(0, |sum, txout| sum + txout.value);
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
            let claiming_txid = spending_tx.txid().to_string();
            let persister = self.persister.clone();
            spawn(async move {
                ok_or_retry_after_sleep!(
                    persister
                        .add_claiming_tx_to_db(
                            closing_txid.clone(),
                            claiming_txid.clone(),
                            (claimed_balance as f64) - claiming_tx_fee_per_channel,
                        )
                        .await,
                    TRY_LOOP_INTERVAL
                );
            });

            self.platform.broadcast_transaction(&spending_tx);
        }
    }
}

pub mod ln_conf;
pub(crate) mod ln_db;
pub mod ln_errors;
pub mod ln_events;
mod ln_filesystem_persister;
pub mod ln_p2p;
pub mod ln_platform;
pub(crate) mod ln_serialization;
mod ln_sql;
pub mod ln_storage;
pub mod ln_utils;

use crate::coin_errors::MyAddressError;
use crate::lightning::ln_utils::{filter_channels, pay_invoice_with_max_total_cltv_expiry_delta, PaymentError};
use crate::utxo::rpc_clients::UtxoRpcClientEnum;
use crate::utxo::utxo_common::{big_decimal_from_sat, big_decimal_from_sat_unsigned};
use crate::utxo::{sat_from_big_decimal, utxo_common, BlockchainNetwork};
use crate::{BalanceFut, CheckIfMyPaymentSentArgs, CoinBalance, CoinFutSpawner, FeeApproxStage, FoundSwapTxSpend,
            HistorySyncState, MarketCoinOps, MmCoin, NegotiateSwapContractAddrErr, PaymentInstructions,
            PaymentInstructionsErr, RawTransactionError, RawTransactionFut, RawTransactionRequest,
            SearchForSwapTxSpendInput, SendMakerPaymentArgs, SendMakerRefundsPaymentArgs,
            SendMakerSpendsTakerPaymentArgs, SendSpendPaymentArgs, SendTakerPaymentArgs, SendTakerRefundsPaymentArgs,
            SendTakerSpendsMakerPaymentArgs, SignatureError, SignatureResult, SwapOps, TradeFee, TradePreimageFut,
            TradePreimageResult, TradePreimageValue, Transaction, TransactionEnum, TransactionErr, TransactionFut,
            TxMarshalingErr, UnexpectedDerivationMethod, UtxoStandardCoin, ValidateAddressResult, ValidateFeeArgs,
            ValidateInstructionsErr, ValidateOtherPubKeyErr, ValidatePaymentError, ValidatePaymentFut,
            ValidatePaymentInput, VerificationError, VerificationResult, WatcherOps, WatcherSearchForSwapTxSpendInput,
            WatcherValidatePaymentInput, WatcherValidateTakerFeeInput, WithdrawError, WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use bitcoin::bech32::ToBase32;
use bitcoin::hashes::Hash;
use bitcoin_hashes::sha256::Hash as Sha256;
use bitcrypto::ChecksumType;
use bitcrypto::{dhash256, ripemd160};
use common::executor::Timer;
use common::executor::{AbortableSystem, AbortedError};
use common::log::{info, LogOnError, LogState};
use common::{async_blocking, get_local_duration_since_epoch, log, now_ms, PagingOptionsEnum};
use db_common::sqlite::rusqlite::Error as SqlError;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::{hash::H256, CompactSignature, KeyPair, Private, Public};
use lightning::chain::keysinterface::{KeysInterface, KeysManager, Recipient};
use lightning::ln::channelmanager::{ChannelDetails, MIN_FINAL_CLTV_EXPIRY};
use lightning::ln::{PaymentHash, PaymentPreimage};
use lightning_background_processor::BackgroundProcessor;
use lightning_invoice::utils::DefaultRouter;
use lightning_invoice::{payment, CreationError, InvoiceBuilder, SignOrCreationError};
use lightning_invoice::{Invoice, InvoiceDescription};
use ln_conf::{LightningCoinConf, PlatformCoinConfirmationTargets};
use ln_db::{DBChannelDetails, HTLCStatus, LightningDB, PaymentInfo, PaymentType};
use ln_errors::{EnableLightningError, EnableLightningResult};
use ln_events::LightningEventHandler;
use ln_filesystem_persister::LightningFilesystemPersister;
use ln_p2p::PeerManager;
use ln_platform::Platform;
use ln_serialization::{ChannelDetailsForRPC, PublicKeyForRPC};
use ln_sql::SqliteLightningDB;
use ln_storage::{NetworkGraph, NodesAddressesMapShared, Scorer, TrustedNodesShared};
use ln_utils::{ChainMonitor, ChannelManager, Router};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_net::ip_addr::myipaddr;
use mm2_number::{BigDecimal, MmNumber};
use parking_lot::Mutex as PaMutex;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use script::TransactionInputSigner;
use secp256k1v22::PublicKey;
use serde::Deserialize;
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};
use std::convert::TryInto;
use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

pub const DEFAULT_INVOICE_EXPIRY: u32 = 3600;

pub type InvoicePayer<E> = payment::InvoicePayer<Arc<ChannelManager>, Router, Arc<Scorer>, Arc<LogState>, E>;

#[derive(Clone)]
pub struct LightningCoin {
    pub platform: Arc<Platform>,
    pub conf: LightningCoinConf,
    /// The lightning node background processor that takes care of tasks that need to happen periodically.
    pub background_processor: Arc<BackgroundProcessor>,
    /// The lightning node peer manager that takes care of connecting to peers, etc..
    pub peer_manager: Arc<PeerManager>,
    /// The lightning node channel manager which keeps track of the number of open channels and sends messages to the appropriate
    /// channel, also tracks HTLC preimages and forwards onion packets appropriately.
    pub channel_manager: Arc<ChannelManager>,
    /// The lightning node chain monitor that takes care of monitoring the chain for transactions of interest.
    pub chain_monitor: Arc<ChainMonitor>,
    /// The lightning node keys manager that takes care of signing invoices.
    pub keys_manager: Arc<KeysManager>,
    /// The lightning node invoice payer.
    pub invoice_payer: Arc<InvoicePayer<Arc<LightningEventHandler>>>,
    /// The lightning node persister that takes care of writing/reading data from storage.
    pub persister: Arc<LightningFilesystemPersister>,
    /// The lightning node db struct that takes care of reading/writing data from/to db.
    pub db: SqliteLightningDB,
    /// The mutex storing the addresses of the nodes that the lightning node has open channels with,
    /// these addresses are used for reconnecting.
    pub open_channels_nodes: NodesAddressesMapShared,
    /// The mutex storing the public keys of the nodes that our lightning node trusts to allow 0 confirmation
    /// inbound channels from.
    pub trusted_nodes: TrustedNodesShared,
    /// The lightning node router that takes care of finding routes for payments.
    // Todo: this should be removed once pay_invoice_with_max_total_cltv_expiry_delta similar functionality is implemented in rust-lightning
    pub router: Arc<Router>,
    /// The lightning node scorer that takes care of scoring routes. Given the uncertainty of channel liquidity balances,
    /// the scorer stores the probabilities that a route is successful based on knowledge learned from successful and unsuccessful attempts.
    // Todo: this should be removed once pay_invoice_with_max_total_cltv_expiry_delta similar functionality is implemented in rust-lightning
    pub scorer: Arc<Scorer>,
}

impl fmt::Debug for LightningCoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "LightningCoin {{ conf: {:?} }}", self.conf) }
}

#[derive(Deserialize)]
pub struct OpenChannelsFilter {
    pub channel_id: Option<H256Json>,
    pub counterparty_node_id: Option<PublicKeyForRPC>,
    pub funding_tx: Option<H256Json>,
    pub from_funding_value_sats: Option<u64>,
    pub to_funding_value_sats: Option<u64>,
    pub is_outbound: Option<bool>,
    pub from_balance_msat: Option<u64>,
    pub to_balance_msat: Option<u64>,
    pub from_outbound_capacity_msat: Option<u64>,
    pub to_outbound_capacity_msat: Option<u64>,
    pub from_inbound_capacity_msat: Option<u64>,
    pub to_inbound_capacity_msat: Option<u64>,
    pub is_ready: Option<bool>,
    pub is_usable: Option<bool>,
    pub is_public: Option<bool>,
}

pub(crate) struct GetOpenChannelsResult {
    pub channels: Vec<ChannelDetailsForRPC>,
    pub skipped: usize,
    pub total: usize,
}

impl Transaction for PaymentHash {
    fn tx_hex(&self) -> Vec<u8> { self.0.to_vec() }

    fn tx_hash(&self) -> BytesJson { self.0.to_vec().into() }
}

impl LightningCoin {
    pub fn platform_coin(&self) -> &UtxoStandardCoin { &self.platform.coin }

    #[inline]
    fn my_node_id(&self) -> String { self.channel_manager.get_our_node_id().to_string() }

    pub(crate) async fn list_channels(&self) -> Vec<ChannelDetails> {
        let channel_manager = self.channel_manager.clone();
        async_blocking(move || channel_manager.list_channels()).await
    }

    async fn get_balance_msat(&self) -> (u64, u64) {
        self.list_channels()
            .await
            .iter()
            .fold((0, 0), |(spendable, unspendable), chan| {
                if chan.is_usable {
                    (
                        spendable + chan.outbound_capacity_msat,
                        unspendable + chan.balance_msat - chan.outbound_capacity_msat,
                    )
                } else {
                    (spendable, unspendable + chan.balance_msat)
                }
            })
    }

    pub(crate) async fn get_channel_by_rpc_id(&self, rpc_id: u64) -> Option<ChannelDetails> {
        self.list_channels()
            .await
            .into_iter()
            .find(|chan| chan.user_channel_id == rpc_id)
    }

    pub(crate) async fn pay_invoice(
        &self,
        invoice: Invoice,
        max_total_cltv_expiry_delta: Option<u32>,
    ) -> Result<PaymentInfo, MmError<PaymentError>> {
        let payment_hash = PaymentHash((invoice.payment_hash()).into_inner());
        let payment_type = PaymentType::OutboundPayment {
            destination: *invoice.payee_pub_key().unwrap_or(&invoice.recover_payee_pub_key()),
        };
        let description = match invoice.description() {
            InvoiceDescription::Direct(d) => d.to_string(),
            InvoiceDescription::Hash(h) => hex::encode(h.0.into_inner()),
        };
        let payment_secret = Some(*invoice.payment_secret());
        let amt_msat = invoice.amount_milli_satoshis().map(|a| a as i64);

        let selfi = self.clone();
        match max_total_cltv_expiry_delta {
            Some(total_cltv) => {
                async_blocking(move || {
                    pay_invoice_with_max_total_cltv_expiry_delta(
                        selfi.channel_manager,
                        selfi.router,
                        selfi.scorer,
                        &invoice,
                        total_cltv,
                    )
                })
                .await?
            },
            None => async_blocking(move || selfi.invoice_payer.pay_invoice(&invoice)).await?,
        };

        let payment_info = PaymentInfo {
            payment_hash,
            payment_type,
            description,
            preimage: None,
            secret: payment_secret,
            amt_msat,
            fee_paid_msat: None,
            status: HTLCStatus::Pending,
            created_at: (now_ms() / 1000) as i64,
            last_updated: (now_ms() / 1000) as i64,
        };
        self.db.add_or_update_payment_in_db(payment_info.clone()).await?;
        Ok(payment_info)
    }

    pub(crate) async fn keysend(
        &self,
        destination: PublicKey,
        amount_msat: u64,
        final_cltv_expiry_delta: u32,
    ) -> Result<PaymentInfo, MmError<PaymentError>> {
        if final_cltv_expiry_delta < MIN_FINAL_CLTV_EXPIRY {
            return MmError::err(PaymentError::CLTVExpiry(final_cltv_expiry_delta, MIN_FINAL_CLTV_EXPIRY));
        }
        let payment_preimage = PaymentPreimage(self.keys_manager.get_secure_random_bytes());

        let selfi = self.clone();
        async_blocking(move || {
            selfi
                .invoice_payer
                .pay_pubkey(destination, payment_preimage, amount_msat, final_cltv_expiry_delta)
                .map_to_mm(|e| PaymentError::Keysend(format!("{:?}", e)))
        })
        .await?;

        let payment_hash = PaymentHash(Sha256::hash(&payment_preimage.0).into_inner());
        let payment_type = PaymentType::OutboundPayment { destination };
        let payment_info = PaymentInfo {
            payment_hash,
            payment_type,
            description: "".into(),
            preimage: Some(payment_preimage),
            secret: None,
            amt_msat: Some(amount_msat as i64),
            fee_paid_msat: None,
            status: HTLCStatus::Pending,
            created_at: (now_ms() / 1000) as i64,
            last_updated: (now_ms() / 1000) as i64,
        };
        self.db.add_or_update_payment_in_db(payment_info.clone()).await?;

        Ok(payment_info)
    }

    pub(crate) async fn get_open_channels_by_filter(
        &self,
        filter: Option<OpenChannelsFilter>,
        paging: PagingOptionsEnum<u64>,
        limit: usize,
    ) -> GetOpenChannelsResult {
        fn apply_open_channel_filter(channel_details: &ChannelDetailsForRPC, filter: &OpenChannelsFilter) -> bool {
            // Checking if channel_id is some and not equal
            if filter.channel_id.is_some() && Some(&channel_details.channel_id) != filter.channel_id.as_ref() {
                return false;
            }

            // Checking if counterparty_node_id is some and not equal
            if filter.counterparty_node_id.is_some()
                && Some(&channel_details.counterparty_node_id) != filter.counterparty_node_id.as_ref()
            {
                return false;
            }

            // Checking if funding_tx is some and not equal
            if filter.funding_tx.is_some() && channel_details.funding_tx != filter.funding_tx {
                return false;
            }

            // Checking if from_funding_value_sats is some and more than funding_tx_value_sats
            if filter.from_funding_value_sats.is_some()
                && Some(&channel_details.funding_tx_value_sats) < filter.from_funding_value_sats.as_ref()
            {
                return false;
            }

            // Checking if to_funding_value_sats is some and less than funding_tx_value_sats
            if filter.to_funding_value_sats.is_some()
                && Some(&channel_details.funding_tx_value_sats) > filter.to_funding_value_sats.as_ref()
            {
                return false;
            }

            // Checking if is_outbound is some and not equal
            if filter.is_outbound.is_some() && Some(&channel_details.is_outbound) != filter.is_outbound.as_ref() {
                return false;
            }

            // Checking if from_balance_msat is some and more than balance_msat
            if filter.from_balance_msat.is_some()
                && Some(&channel_details.balance_msat) < filter.from_balance_msat.as_ref()
            {
                return false;
            }

            // Checking if to_balance_msat is some and less than balance_msat
            if filter.to_balance_msat.is_some() && Some(&channel_details.balance_msat) > filter.to_balance_msat.as_ref()
            {
                return false;
            }

            // Checking if from_outbound_capacity_msat is some and more than outbound_capacity_msat
            if filter.from_outbound_capacity_msat.is_some()
                && Some(&channel_details.outbound_capacity_msat) < filter.from_outbound_capacity_msat.as_ref()
            {
                return false;
            }

            // Checking if to_outbound_capacity_msat is some and less than outbound_capacity_msat
            if filter.to_outbound_capacity_msat.is_some()
                && Some(&channel_details.outbound_capacity_msat) > filter.to_outbound_capacity_msat.as_ref()
            {
                return false;
            }

            // Checking if from_inbound_capacity_msat is some and more than outbound_capacity_msat
            if filter.from_inbound_capacity_msat.is_some()
                && Some(&channel_details.inbound_capacity_msat) < filter.from_inbound_capacity_msat.as_ref()
            {
                return false;
            }

            // Checking if to_inbound_capacity_msat is some and less than inbound_capacity_msat
            if filter.to_inbound_capacity_msat.is_some()
                && Some(&channel_details.inbound_capacity_msat) > filter.to_inbound_capacity_msat.as_ref()
            {
                return false;
            }

            // Checking if is_ready is some and not equal
            if filter.is_ready.is_some() && Some(&channel_details.is_ready) != filter.is_ready.as_ref() {
                return false;
            }

            // Checking if is_usable is some and not equal
            if filter.is_usable.is_some() && Some(&channel_details.is_usable) != filter.is_usable.as_ref() {
                return false;
            }

            // Checking if is_public is some and not equal
            if filter.is_public.is_some() && Some(&channel_details.is_public) != filter.is_public.as_ref() {
                return false;
            }

            // All checks pass
            true
        }

        let mut total_open_channels: Vec<ChannelDetailsForRPC> =
            self.list_channels().await.into_iter().map(From::from).collect();

        total_open_channels.sort_by(|a, b| a.rpc_channel_id.cmp(&b.rpc_channel_id));
        drop_mutability!(total_open_channels);

        let open_channels_filtered = if let Some(ref f) = filter {
            total_open_channels
                .into_iter()
                .filter(|chan| apply_open_channel_filter(chan, f))
                .collect()
        } else {
            total_open_channels
        };

        let offset = match paging {
            PagingOptionsEnum::PageNumber(page) => (page.get() - 1) * limit,
            PagingOptionsEnum::FromId(rpc_id) => open_channels_filtered
                .iter()
                .position(|x| x.rpc_channel_id == rpc_id)
                .map(|pos| pos + 1)
                .unwrap_or_default(),
        };

        let total = open_channels_filtered.len();

        let channels = if offset + limit <= total {
            open_channels_filtered[offset..offset + limit].to_vec()
        } else {
            open_channels_filtered[offset..].to_vec()
        };

        GetOpenChannelsResult {
            channels,
            skipped: offset,
            total,
        }
    }

    async fn create_invoice_for_hash(
        &self,
        payment_hash: PaymentHash,
        amt_msat: Option<u64>,
        description: String,
        min_final_cltv_expiry: u64,
        invoice_expiry_delta_secs: u32,
    ) -> Result<Invoice, MmError<SignOrCreationError<()>>> {
        let open_channels_nodes = self.open_channels_nodes.lock().clone();
        for (node_pubkey, node_addr) in open_channels_nodes {
            ln_p2p::connect_to_ln_node(node_pubkey, node_addr, self.peer_manager.clone())
                .await
                .error_log_with_msg(&format!(
                    "Channel with node: {} can't be used for invoice routing hints due to connection error.",
                    node_pubkey
                ));
        }

        // `create_inbound_payment` only returns an error if the amount is greater than the total bitcoin
        // supply.
        let payment_secret = self
            .channel_manager
            .create_inbound_payment_for_hash(payment_hash, amt_msat, invoice_expiry_delta_secs)
            .map_to_mm(|()| SignOrCreationError::CreationError(CreationError::InvalidAmount))?;
        let our_node_pubkey = self.channel_manager.get_our_node_id();
        // Todo: Check if it's better to use UTC instead of local time for invoice generations
        let duration = get_local_duration_since_epoch().expect("for the foreseeable future this shouldn't happen");

        let mut invoice = InvoiceBuilder::new(self.platform.network.clone().into())
            .description(description)
            .duration_since_epoch(duration)
            .payee_pub_key(our_node_pubkey)
            .payment_hash(Hash::from_inner(payment_hash.0))
            .payment_secret(payment_secret)
            .basic_mpp()
            // Todo: This should be validated by the other side, right now this is not validated by rust-lightning and the PaymentReceived event doesn't include the final cltv of the payment for us to validate it
            // Todo: This needs a PR opened to rust-lightning, I already contacted them about it and there is an issue opened for it https://github.com/lightningdevkit/rust-lightning/issues/1850
            .min_final_cltv_expiry(min_final_cltv_expiry)
            .expiry_time(core::time::Duration::from_secs(invoice_expiry_delta_secs.into()));
        if let Some(amt) = amt_msat {
            invoice = invoice.amount_milli_satoshis(amt);
        }

        let route_hints = filter_channels(self.channel_manager.list_usable_channels(), amt_msat);
        for hint in route_hints {
            invoice = invoice.private_route(hint);
        }

        let raw_invoice = match invoice.build_raw() {
            Ok(inv) => inv,
            Err(e) => return MmError::err(SignOrCreationError::CreationError(e)),
        };
        let hrp_str = raw_invoice.hrp.to_string();
        let hrp_bytes = hrp_str.as_bytes();
        let data_without_signature = raw_invoice.data.to_base32();
        let signed_raw_invoice = raw_invoice.sign(|_| {
            self.keys_manager
                .sign_invoice(hrp_bytes, &data_without_signature, Recipient::Node)
        });
        match signed_raw_invoice {
            Ok(inv) => Ok(Invoice::from_signed(inv).map_err(|_| SignOrCreationError::SignError(()))?),
            Err(e) => MmError::err(SignOrCreationError::SignError(e)),
        }
    }

    fn estimate_blocks_from_duration(&self, duration: u64) -> u64 { duration / self.platform.avg_block_time }

    async fn swap_payment_instructions(
        &self,
        secret_hash: &[u8],
        amount: &BigDecimal,
        expires_in: u64,
        min_final_cltv_expiry: u64,
    ) -> Result<Vec<u8>, MmError<PaymentInstructionsErr>> {
        // lightning decimals should be 11 in config since the smallest divisible unit in lightning coin is msat
        let amt_msat = sat_from_big_decimal(amount, self.decimals())?;
        let payment_hash =
            payment_hash_from_slice(secret_hash).map_to_mm(|e| PaymentInstructionsErr::InternalError(e.to_string()))?;
        // note: No description is provided in the invoice to reduce the payload
        let invoice = self
            .create_invoice_for_hash(
                payment_hash,
                Some(amt_msat),
                "".into(),
                min_final_cltv_expiry,
                expires_in.try_into().expect("expires_in shouldn't exceed u32::MAX"),
            )
            .await
            .map_err(|e| PaymentInstructionsErr::LightningInvoiceErr(e.to_string()))?;
        Ok(invoice.to_string().into_bytes())
    }

    fn validate_swap_instructions(
        &self,
        instructions: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        min_final_cltv_expiry: u64,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        let invoice = Invoice::from_str(&String::from_utf8_lossy(instructions))?;
        if invoice.payment_hash().as_inner() != secret_hash
            && ripemd160(invoice.payment_hash().as_inner()).as_slice() != secret_hash
        {
            return MmError::err(ValidateInstructionsErr::ValidateLightningInvoiceErr(
                "Invalid invoice payment hash!".into(),
            ));
        }

        let invoice_amount = invoice
            .amount_milli_satoshis()
            .or_mm_err(|| ValidateInstructionsErr::ValidateLightningInvoiceErr("No invoice amount!".into()))?;
        if big_decimal_from_sat(invoice_amount as i64, self.decimals()) != amount {
            return MmError::err(ValidateInstructionsErr::ValidateLightningInvoiceErr(
                "Invalid invoice amount!".into(),
            ));
        }

        if invoice.min_final_cltv_expiry() != min_final_cltv_expiry {
            return MmError::err(ValidateInstructionsErr::ValidateLightningInvoiceErr(
                "Invalid invoice min_final_cltv_expiry!".into(),
            ));
        }

        Ok(PaymentInstructions::Lightning(invoice))
    }

    fn spend_swap_payment(&self, spend_payment_args: SendSpendPaymentArgs<'_>) -> TransactionFut {
        let payment_hash = try_tx_fus!(payment_hash_from_slice(spend_payment_args.other_payment_tx));
        let mut preimage = [b' '; 32];
        preimage.copy_from_slice(spend_payment_args.secret);

        let coin = self.clone();
        let fut = async move {
            let payment_preimage = PaymentPreimage(preimage);
            coin.channel_manager.claim_funds(payment_preimage);
            coin.db
                .update_payment_preimage_in_db(payment_hash, payment_preimage)
                .await
                .error_log_with_msg(&format!(
                    "Unable to update payment {} information in DB with preimage: {}!",
                    hex::encode(payment_hash.0),
                    hex::encode(preimage)
                ));
            Ok(TransactionEnum::LightningPayment(payment_hash))
        };
        Box::new(fut.boxed().compat())
    }

    fn validate_swap_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        let payment_hash = try_f!(payment_hash_from_slice(&input.payment_tx)
            .map_to_mm(|e| ValidatePaymentError::TxDeserializationError(e.to_string())));
        let payment_hex = hex::encode(payment_hash.0);

        let amt_msat = try_f!(sat_from_big_decimal(&input.amount, self.decimals()));

        let coin = self.clone();
        let fut = async move {
            match coin.db.get_payment_from_db(payment_hash).await {
                Ok(Some(mut payment)) => {
                    let amount_received = payment.amt_msat;
                    // Note: locktime doesn't need to be validated since min_final_cltv_expiry should be validated in rust-lightning after fixing the below issue
                    // https://github.com/lightningdevkit/rust-lightning/issues/1850
                    // Also, PaymentReceived won't be fired if amount_received < the amount requested in the invoice, this check is probably not needed.
                    // But keeping it just in case any changes happen in rust-lightning
                    if amount_received != Some(amt_msat as i64) {
                        // Free the htlc to allow for this inbound liquidity to be used for other inbound payments
                        coin.channel_manager.fail_htlc_backwards(&payment_hash);
                        payment.status = HTLCStatus::Failed;
                        drop_mutability!(payment);
                        coin.db
                            .add_or_update_payment_in_db(payment)
                            .await
                            .error_log_with_msg("Unable to update payment information in DB!");
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Provided payment {} amount {:?} doesn't match required amount {}",
                            payment_hex, amount_received, amt_msat
                        )));
                    }
                    Ok(())
                },
                Ok(None) => MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "Payment {} is not in the database when it should be!",
                    payment_hex
                ))),
                Err(e) => MmError::err(ValidatePaymentError::InternalError(format!(
                    "Unable to retrieve payment {} from the database error: {}",
                    payment_hex, e
                ))),
            }
        };
        Box::new(fut.boxed().compat())
    }
}

#[async_trait]
// Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
impl SwapOps for LightningCoin {
    // Todo: This uses dummy data for now for the sake of swap P.O.C., this should be implemented probably after agreeing on how fees will work for lightning
    fn send_taker_fee(&self, _fee_addr: &[u8], _amount: BigDecimal, _uuid: &[u8]) -> TransactionFut {
        let fut = async move { Ok(TransactionEnum::LightningPayment(PaymentHash([1; 32]))) };
        Box::new(fut.boxed().compat())
    }

    fn send_maker_payment(&self, maker_payment_args: SendMakerPaymentArgs<'_>) -> TransactionFut {
        let PaymentInstructions::Lightning(invoice) = try_tx_fus!(maker_payment_args
            .payment_instructions
            .clone()
            .ok_or("payment_instructions can't be None"));
        let coin = self.clone();
        let fut = async move {
            // No need for max_total_cltv_expiry_delta for lightning maker payment since the maker is the side that reveals the secret/preimage
            let payment = try_tx_s!(coin.pay_invoice(invoice, None).await);
            Ok(payment.payment_hash.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_payment(&self, taker_payment_args: SendTakerPaymentArgs<'_>) -> TransactionFut {
        let PaymentInstructions::Lightning(invoice) = try_tx_fus!(taker_payment_args
            .payment_instructions
            .clone()
            .ok_or("payment_instructions can't be None"));
        let max_total_cltv_expiry_delta = self
            .estimate_blocks_from_duration(taker_payment_args.time_lock_duration)
            .try_into()
            .expect("max_total_cltv_expiry_delta shouldn't exceed u32::MAX");
        let coin = self.clone();
        let fut = async move {
            // Todo: The path/s used is already logged when PaymentPathSuccessful/PaymentPathFailed events are fired, it might be better to save it to the DB and retrieve it with the payment info.
            let payment = try_tx_s!(coin.pay_invoice(invoice, Some(max_total_cltv_expiry_delta)).await);
            Ok(payment.payment_hash.into())
        };
        Box::new(fut.boxed().compat())
    }

    #[inline]
    fn send_maker_spends_taker_payment(
        &self,
        maker_spends_payment_args: SendMakerSpendsTakerPaymentArgs<'_>,
    ) -> TransactionFut {
        self.spend_swap_payment(maker_spends_payment_args)
    }

    #[inline]
    fn send_taker_spends_maker_payment(
        &self,
        taker_spends_payment_args: SendTakerSpendsMakerPaymentArgs<'_>,
    ) -> TransactionFut {
        self.spend_swap_payment(taker_spends_payment_args)
    }

    fn send_taker_refunds_payment(
        &self,
        _taker_refunds_payment_args: SendTakerRefundsPaymentArgs<'_>,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_refunds_payment(
        &self,
        _maker_refunds_payment_args: SendMakerRefundsPaymentArgs<'_>,
    ) -> TransactionFut {
        unimplemented!()
    }

    // Todo: This validates the dummy fee for now for the sake of swap P.O.C., this should be implemented probably after agreeing on how fees will work for lightning
    fn validate_fee(
        &self,
        _validate_fee_args: ValidateFeeArgs<'_>,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        Box::new(futures01::future::ok(()))
    }

    #[inline]
    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        self.validate_swap_payment(input)
    }

    #[inline]
    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        self.validate_swap_payment(input)
    }

    // Todo: This is None for now for the sake of swap P.O.C., this should be implemented probably in next PRs and should be tested across restarts
    fn check_if_my_payment_sent(
        &self,
        _if_my_payment_spent_args: CheckIfMyPaymentSentArgs<'_>,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        Box::new(futures01::future::ok(None))
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        _: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        _: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    fn check_tx_signed_by_pub(&self, _tx: &[u8], _expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>> {
        unimplemented!();
    }

    async fn extract_secret(&self, _secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        let payment_hash = payment_hash_from_slice(spend_tx).map_err(|e| e.to_string())?;
        let payment_hex = hex::encode(payment_hash.0);

        match self.db.get_payment_from_db(payment_hash).await {
            Ok(Some(payment)) => match payment.preimage {
                Some(preimage) => Ok(preimage.0.to_vec()),
                None => ERR!("Preimage for payment {} should be found on the database", payment_hex),
            },
            Ok(None) => ERR!("Payment {} is not in the database when it should be!", payment_hex),
            Err(e) => ERR!(
                "Unable to retrieve payment {} from the database error: {}",
                payment_hex,
                e
            ),
        }
    }

    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        Ok(None)
    }

    // Todo: This can be changed if private swaps were to be implemented for lightning
    #[inline]
    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair {
        utxo_common::derive_htlc_key_pair(self.platform.coin.as_ref(), swap_unique_data)
    }

    #[inline]
    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> {
        utxo_common::validate_other_pubkey(raw_pubkey)
    }

    async fn maker_payment_instructions(
        &self,
        secret_hash: &[u8],
        amount: &BigDecimal,
        maker_lock_duration: u64,
        expires_in: u64,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        let min_final_cltv_expiry = self.estimate_blocks_from_duration(maker_lock_duration);
        self.swap_payment_instructions(secret_hash, amount, expires_in, min_final_cltv_expiry)
            .await
            .map(Some)
    }

    #[inline]
    async fn taker_payment_instructions(
        &self,
        secret_hash: &[u8],
        amount: &BigDecimal,
        expires_in: u64,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        self.swap_payment_instructions(secret_hash, amount, expires_in, MIN_FINAL_CLTV_EXPIRY as u64)
            .await
            .map(Some)
    }

    fn validate_maker_payment_instructions(
        &self,
        instructions: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        maker_lock_duration: u64,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        let min_final_cltv_expiry = self.estimate_blocks_from_duration(maker_lock_duration);
        self.validate_swap_instructions(instructions, secret_hash, amount, min_final_cltv_expiry)
    }

    #[inline]
    fn validate_taker_payment_instructions(
        &self,
        instructions: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        self.validate_swap_instructions(instructions, secret_hash, amount, MIN_FINAL_CLTV_EXPIRY as u64)
    }

    // Watchers cannot be used for lightning swaps for now
    // Todo: Check if watchers can work in some cases with lightning and implement it if it's possible, the watcher will not be able to retrieve the preimage since it's retrieved through the lightning network
    // Todo: The watcher can retrieve the preimage only if he is running a lightning node and is part of the nodes that routed the taker payment which is a very low probability event that shouldn't be considered
    fn is_supported_by_watchers(&self) -> bool { false }

    fn maker_locktime_multiplier(&self) -> f64 { 1.5 }
}

#[derive(Debug, Display)]
pub enum PaymentHashFromSliceErr {
    #[display(fmt = "Invalid data length of {}", _0)]
    InvalidLength(usize),
}

fn payment_hash_from_slice(data: &[u8]) -> Result<PaymentHash, PaymentHashFromSliceErr> {
    let len = data.len();
    if len != 32 {
        return Err(PaymentHashFromSliceErr::InvalidLength(len));
    }
    let mut hash = [b' '; 32];
    hash.copy_from_slice(data);
    Ok(PaymentHash(hash))
}

#[async_trait]
impl WatcherOps for LightningCoin {
    fn create_maker_payment_spend_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_maker_payment_spend_preimage(&self, _preimage: &[u8], _secret: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn create_taker_payment_refund_preimage(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_payment_refund_preimage(&self, _taker_refunds_payment: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn watcher_validate_taker_fee(&self, _input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        _input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!();
    }
}

impl MarketCoinOps for LightningCoin {
    fn ticker(&self) -> &str { &self.conf.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { Ok(self.my_node_id()) }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> { unimplemented!() }

    fn sign_message_hash(&self, message: &str) -> Option<[u8; 32]> {
        let mut _message_prefix = self.conf.sign_message_prefix.clone()?;
        let prefixed_message = format!("{}{}", _message_prefix, message);
        Some(dhash256(prefixed_message.as_bytes()).take())
    }

    fn sign_message(&self, message: &str) -> SignatureResult<String> {
        let message_hash = self.sign_message_hash(message).ok_or(SignatureError::PrefixNotFound)?;
        let secret_key = self
            .keys_manager
            .get_node_secret(Recipient::Node)
            .map_err(|_| SignatureError::InternalError("Error accessing node keys".to_string()))?;
        let private = Private {
            prefix: 239,
            secret: H256::from(*secret_key.as_ref()),
            compressed: true,
            checksum_type: ChecksumType::DSHA256,
        };
        let signature = private.sign_compact(&H256::from(message_hash))?;
        Ok(zbase32::encode_full_bytes(&signature))
    }

    fn verify_message(&self, signature: &str, message: &str, pubkey: &str) -> VerificationResult<bool> {
        let message_hash = self
            .sign_message_hash(message)
            .ok_or(VerificationError::PrefixNotFound)?;
        let signature = CompactSignature::from(
            zbase32::decode_full_bytes_str(signature)
                .map_err(|e| VerificationError::SignatureDecodingError(e.to_string()))?,
        );
        let recovered_pubkey = Public::recover_compact(&H256::from(message_hash), &signature)?;
        Ok(recovered_pubkey.to_string() == pubkey)
    }

    // Todo: max_inbound_in_flight_htlc_percent should be taken in consideration too for max allowed amount, this can be considered the spendable balance,
    // Todo: but it's better to refactor the CoinBalance struct to add more info. We can make it 100% in the config for now until this is implemented.
    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let decimals = self.decimals();
        let fut = async move {
            let (spendable_msat, unspendable_msat) = coin.get_balance_msat().await;
            Ok(CoinBalance {
                spendable: big_decimal_from_sat_unsigned(spendable_msat, decimals),
                unspendable: big_decimal_from_sat_unsigned(unspendable_msat, decimals),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> {
        Box::new(self.platform_coin().my_balance().map(|res| res.spendable))
    }

    fn platform_ticker(&self) -> &str { self.platform_coin().ticker() }

    fn send_raw_tx(&self, _tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        Box::new(futures01::future::err(
            MmError::new(
                "send_raw_tx is not supported for lightning, please use send_payment method instead.".to_string(),
            )
            .to_string(),
        ))
    }

    fn send_raw_tx_bytes(&self, _tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        Box::new(futures01::future::err(
            MmError::new(
                "send_raw_tx is not supported for lightning, please use send_payment method instead.".to_string(),
            )
            .to_string(),
        ))
    }

    // Todo: Add waiting for confirmations logic for the case of if the channel is closed and the htlc can be claimed on-chain
    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        _confirmations: u64,
        _requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let payment_hash = try_f!(payment_hash_from_slice(tx).map_err(|e| e.to_string()));
        let payment_hex = hex::encode(payment_hash.0);

        let coin = self.clone();
        let fut = async move {
            loop {
                if now_ms() / 1000 > wait_until {
                    return ERR!(
                        "Waited too long until {} for payment {} to be received",
                        wait_until,
                        payment_hex
                    );
                }

                match coin.db.get_payment_from_db(payment_hash).await {
                    Ok(Some(payment)) => {
                        match payment.payment_type {
                            PaymentType::OutboundPayment { .. } => match payment.status {
                                HTLCStatus::Pending | HTLCStatus::Succeeded => return Ok(()),
                                HTLCStatus::Received => {
                                    return ERR!(
                                        "Payment {} has an invalid status of {} in the db",
                                        payment_hex,
                                        payment.status
                                    )
                                },
                                // Todo: PaymentFailed event is fired after 5 retries, maybe timeout should be used instead.
                                // Todo: Still this doesn't prevent failure if there are no routes
                                // Todo: JIT channels/routing can be used to solve this issue https://github.com/lightningdevkit/rust-lightning/pull/1835 but it requires some trust.
                                HTLCStatus::Failed => return ERR!("Lightning swap payment {} failed", payment_hex),
                            },
                            PaymentType::InboundPayment => match payment.status {
                                HTLCStatus::Received | HTLCStatus::Succeeded => return Ok(()),
                                HTLCStatus::Pending => info!("Payment {} not received yet!", payment_hex),
                                HTLCStatus::Failed => return ERR!("Lightning swap payment {} failed", payment_hex),
                            },
                        }
                    },
                    Ok(None) => info!("Payment {} not received yet!", payment_hex),
                    Err(e) => return ERR!("Error getting payment {} from db: {}", payment_hex, e),
                }

                // note: When sleeping for only 1 second the test_send_payment_and_swaps unit test took 20 seconds to complete instead of 37 seconds when WAIT_CONFIRM_INTERVAL (15 seconds) is used
                // Todo: In next sprints, should add a mutex for lightning swap payments to avoid overloading the shared db connection with requests when the sleep time is reduced and multiple swaps are ran together
                // Todo: The aim is to make lightning swap payments as fast as possible. Running swap payments statuses should be loaded from db on restarts in this case.
                Timer::sleep(check_every as f64).await;
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn wait_for_htlc_tx_spend(
        &self,
        transaction: &[u8],
        _secret_hash: &[u8],
        wait_until: u64,
        _from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
        _check_every: f64,
    ) -> TransactionFut {
        let payment_hash = try_tx_fus!(payment_hash_from_slice(transaction));
        let payment_hex = hex::encode(payment_hash.0);

        let coin = self.clone();
        let fut = async move {
            loop {
                if now_ms() / 1000 > wait_until {
                    return Err(TransactionErr::Plain(format!(
                        "Waited too long until {} for payment {} to be spent",
                        wait_until, payment_hex
                    )));
                }

                match coin.db.get_payment_from_db(payment_hash).await {
                    Ok(Some(payment)) => match payment.status {
                        HTLCStatus::Pending => (),
                        HTLCStatus::Received => {
                            return Err(TransactionErr::Plain(format!(
                                "Payment {} has an invalid status of {} in the db",
                                payment_hex, payment.status
                            )))
                        },
                        HTLCStatus::Succeeded => return Ok(TransactionEnum::LightningPayment(payment_hash)),
                        HTLCStatus::Failed => {
                            return Err(TransactionErr::Plain(format!(
                                "Lightning swap payment {} failed",
                                payment_hex
                            )))
                        },
                    },
                    Ok(None) => {
                        return Err(TransactionErr::Plain(format!(
                            "Payment {} not found in DB",
                            payment_hex
                        )))
                    },
                    Err(e) => {
                        return Err(TransactionErr::Plain(format!(
                            "Error getting payment {} from db: {}",
                            payment_hex, e
                        )))
                    },
                }

                // note: When sleeping for only 1 second the test_send_payment_and_swaps unit test took 20 seconds to complete instead of 37 seconds when sleeping for 10 seconds
                // Todo: In next sprints, should add a mutex for lightning swap payments to avoid overloading the shared db connection with requests when the sleep time is reduced and multiple swaps are ran together.
                // Todo: The aim is to make lightning swap payments as fast as possible, more sleep time can be allowed for maker payment since it waits for the secret to be revealed on another chain first.
                // Todo: Running swap payments statuses should be loaded from db on restarts in this case.
                Timer::sleep(10.).await;
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        Ok(TransactionEnum::LightningPayment(
            payment_hash_from_slice(bytes).map_to_mm(|e| TxMarshalingErr::InvalidInput(e.to_string()))?,
        ))
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> { Box::new(futures01::future::ok(0)) }

    fn display_priv_key(&self) -> Result<String, String> {
        Ok(self
            .keys_manager
            .get_node_secret(Recipient::Node)
            .map_err(|_| "Unsupported recipient".to_string())?
            .display_secret()
            .to_string())
    }

    // Todo: min_tx_amount should depend on inbound_htlc_minimum_msat of the channel/s the payment will be sent through, 1 satoshi is used for for now (1000 of the base unit of lightning which is msat)
    fn min_tx_amount(&self) -> BigDecimal { big_decimal_from_sat(1000, self.decimals()) }

    // Todo: Equals to min_tx_amount for now (1 satoshi), should change this later
    fn min_trading_vol(&self) -> MmNumber { self.min_tx_amount().into() }
}

#[async_trait]
impl MmCoin for LightningCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.platform.abortable_system) }

    fn get_raw_transaction(&self, _req: RawTransactionRequest) -> RawTransactionFut {
        let fut = async move {
            MmError::err(RawTransactionError::InternalError(
                "get_raw_transaction method is not supported for lightning, please use get_payment_details method instead.".into(),
            ))
        };
        Box::new(fut.boxed().compat())
    }

    fn get_tx_hex_by_hash(&self, _tx_hash: Vec<u8>) -> RawTransactionFut {
        let fut = async move {
            MmError::err(RawTransactionError::InternalError(
                "get_tx_hex_by_hash method is not supported for lightning.".into(),
            ))
        };
        Box::new(fut.boxed().compat())
    }

    fn withdraw(&self, _req: WithdrawRequest) -> WithdrawFut {
        let fut = async move {
            MmError::err(WithdrawError::InternalError(
                "withdraw method is not supported for lightning, please use generate_invoice method instead.".into(),
            ))
        };
        Box::new(fut.boxed().compat())
    }

    fn decimals(&self) -> u8 { self.conf.decimals }

    fn convert_to_address(&self, _from: &str, _to_address_format: Json) -> Result<String, String> {
        Err(MmError::new("Address conversion is not available for LightningCoin".to_string()).to_string())
    }

    fn validate_address(&self, address: &str) -> ValidateAddressResult {
        match PublicKey::from_str(address) {
            Ok(_) => ValidateAddressResult {
                is_valid: true,
                reason: None,
            },
            Err(e) => ValidateAddressResult {
                is_valid: false,
                reason: Some(format!("Error {} on parsing node public key", e)),
            },
        }
    }

    // Todo: Implement this when implementing payments history for lightning
    fn process_history_loop(&self, _ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { unimplemented!() }

    // Todo: Implement this when implementing payments history for lightning
    fn history_sync_status(&self) -> HistorySyncState { unimplemented!() }

    // Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { unimplemented!() }

    // Todo: This uses dummy data for now for the sake of swap P.O.C., this should be implemented probably after agreeing on how fees will work for lightning
    async fn get_sender_trade_fee(
        &self,
        _value: TradePreimageValue,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        Ok(TradeFee {
            coin: self.ticker().to_owned(),
            amount: Default::default(),
            paid_from_trading_vol: false,
        })
    }

    // Todo: This uses dummy data for now for the sake of swap P.O.C., this should be implemented probably after agreeing on how fees will work for lightning
    fn get_receiver_trade_fee(&self, _send_amount: BigDecimal, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        Box::new(futures01::future::ok(TradeFee {
            coin: self.ticker().to_owned(),
            amount: Default::default(),
            paid_from_trading_vol: false,
        }))
    }

    // Todo: This uses dummy data for now for the sake of swap P.O.C., this should be implemented probably after agreeing on how fees will work for lightning
    async fn get_fee_to_send_taker_fee(
        &self,
        _dex_fee_amount: BigDecimal,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        Ok(TradeFee {
            coin: self.ticker().to_owned(),
            amount: Default::default(),
            paid_from_trading_vol: false,
        })
    }

    // Lightning payments are either pending, successful or failed. Once a payment succeeds there is no need to for confirmations
    // unlike onchain transactions.
    fn required_confirmations(&self) -> u64 { 0 }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, _confirmations: u64) {}

    fn set_requires_notarization(&self, _requires_nota: bool) {}

    fn swap_contract_address(&self) -> Option<BytesJson> { None }

    fn fallback_swap_contract(&self) -> Option<BytesJson> { None }

    fn mature_confirmations(&self) -> Option<u32> { None }

    // Todo: This uses default data for now for the sake of swap P.O.C., this should be implemented probably when implementing order matching if it's needed
    fn coin_protocol_info(&self) -> Vec<u8> { Vec::new() }

    // Todo: This uses default data for now for the sake of swap P.O.C., this should be implemented probably when implementing order matching if it's needed
    fn is_coin_protocol_supported(&self, _info: &Option<Vec<u8>>) -> bool { true }

    fn on_disabled(&self) -> Result<(), AbortedError> { AbortableSystem::abort_all(&self.platform.abortable_system) }

    fn on_token_deactivated(&self, _ticker: &str) {}
}

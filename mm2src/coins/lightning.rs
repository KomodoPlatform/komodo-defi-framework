pub mod ln_conf;
pub(crate) mod ln_db;
pub mod ln_errors;
mod ln_events;
mod ln_filesystem_persister;
pub(crate) mod ln_p2p;
mod ln_platform;
pub(crate) mod ln_serialization;
mod ln_sql;
pub(crate) mod ln_storage;
mod ln_utils;

use super::DerivationMethod;
use crate::lightning::ln_utils::filter_channels;
use crate::utxo::rpc_clients::UtxoRpcClientEnum;
use crate::utxo::utxo_common::{big_decimal_from_sat, big_decimal_from_sat_unsigned};
use crate::utxo::{sat_from_big_decimal, utxo_common, BlockchainNetwork};
use crate::{BalanceFut, CoinBalance, CoinFutSpawner, FeeApproxStage, FoundSwapTxSpend, HistorySyncState,
            MarketCoinOps, MmCoin, MyAddressError, NegotiateSwapContractAddrErr, PaymentInstructions,
            PaymentInstructionsErr, RawTransactionError, RawTransactionFut, RawTransactionRequest,
            SearchForSwapTxSpendInput, SignatureError, SignatureResult, SwapOps, TradeFee, TradePreimageFut,
            TradePreimageResult, TradePreimageValue, Transaction, TransactionEnum, TransactionErr, TransactionFut,
            TxMarshalingErr, UnexpectedDerivationMethod, UtxoStandardCoin, ValidateAddressResult,
            ValidateInstructionsErr, ValidateOtherPubKeyErr, ValidatePaymentError, ValidatePaymentFut,
            ValidatePaymentInput, VerificationError, VerificationResult, WatcherValidatePaymentInput, WithdrawError,
            WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use bitcoin::bech32::ToBase32;
use bitcoin::hashes::Hash;
use bitcoin_hashes::sha256::Hash as Sha256;
use bitcrypto::ChecksumType;
use bitcrypto::{dhash256, ripemd160};
use common::executor::{SpawnFuture, Timer};
use common::log::{info, LogOnError, LogState};
use common::{async_blocking, get_local_duration_since_epoch, log, now_ms, PagingOptionsEnum};
use db_common::sqlite::rusqlite::Error as SqlError;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::{hash::H256, CompactSignature, KeyPair, Private, Public};
use lightning::chain::keysinterface::{KeysInterface, KeysManager, Recipient};
use lightning::chain::Access;
use lightning::ln::channelmanager::{ChannelDetails, MIN_FINAL_CLTV_EXPIRY};
use lightning::ln::{PaymentHash, PaymentPreimage};
use lightning::routing::gossip;
use lightning_background_processor::{BackgroundProcessor, GossipSync};
use lightning_invoice::utils::DefaultRouter;
use lightning_invoice::{payment, CreationError, InvoiceBuilder, SignOrCreationError};
use lightning_invoice::{Invoice, InvoiceDescription};
use ln_conf::{LightningCoinConf, LightningProtocolConf, PlatformCoinConfirmationTargets};
use ln_db::{DBChannelDetails, HTLCStatus, LightningDB, PaymentInfo, PaymentType};
use ln_errors::{EnableLightningError, EnableLightningResult};
use ln_events::{init_abortable_events, LightningEventHandler};
use ln_filesystem_persister::LightningFilesystemPersister;
use ln_p2p::PeerManager;
use ln_platform::Platform;
use ln_serialization::{ChannelDetailsForRPC, PublicKeyForRPC};
use ln_sql::SqliteLightningDB;
use ln_storage::{LightningStorage, NetworkGraph, NodesAddressesMapShared, Scorer, TrustedNodesShared};
use ln_utils::{ChainMonitor, ChannelManager};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_net::ip_addr::myipaddr;
use mm2_number::{BigDecimal, MmNumber};
use parking_lot::Mutex as PaMutex;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use script::TransactionInputSigner;
use secp256k1v22::PublicKey;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

pub const DEFAULT_INVOICE_EXPIRY: u32 = 3600;

type Router = DefaultRouter<Arc<NetworkGraph>, Arc<LogState>>;
type InvoicePayer<E> = payment::InvoicePayer<Arc<ChannelManager>, Router, Arc<Scorer>, Arc<LogState>, E>;

#[derive(Clone)]
pub struct LightningCoin {
    pub platform: Arc<Platform>,
    pub conf: LightningCoinConf,
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

#[derive(Debug, Display)]
pub(crate) enum PaymentError {
    #[display(fmt = "Final cltv expiry delta {} is below the required minimum of {}", _0, _1)]
    CLTVExpiry(u32, u32),
    #[display(fmt = "Error paying invoice: {}", _0)]
    Invoice(String),
    #[display(fmt = "Keysend error: {}", _0)]
    Keysend(String),
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
}

impl From<SqlError> for PaymentError {
    fn from(err: SqlError) -> PaymentError { PaymentError::DbError(err.to_string()) }
}

impl Transaction for PaymentHash {
    fn tx_hex(&self) -> Option<Vec<u8>> { None }

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

    pub(crate) async fn pay_invoice(&self, invoice: Invoice) -> Result<PaymentInfo, MmError<PaymentError>> {
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
        async_blocking(move || {
            selfi
                .invoice_payer
                .pay_invoice(&invoice)
                .map_to_mm(|e| PaymentError::Invoice(format!("{:?}", e)))
        })
        .await?;

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
            // Todo: This will probably be important in locktime calculations in the next PRs and should be validated by the other side
            .min_final_cltv_expiry(MIN_FINAL_CLTV_EXPIRY.into())
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
}

#[async_trait]
// Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
impl SwapOps for LightningCoin {
    // Todo: This uses dummy data for now for the sake of swap P.O.C., this should be implemented probably after agreeing on how fees will work for lightning
    fn send_taker_fee(&self, _fee_addr: &[u8], _amount: BigDecimal, _uuid: &[u8]) -> TransactionFut {
        let fut = async move { Ok(TransactionEnum::LightningPayment(PaymentHash([1; 32]))) };
        Box::new(fut.boxed().compat())
    }

    fn send_maker_payment(
        &self,
        _time_lock: u32,
        _taker_pub: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
        payment_instructions: &Option<PaymentInstructions>,
    ) -> TransactionFut {
        let PaymentInstructions::Lightning(invoice) =
            try_tx_fus!(payment_instructions.clone().ok_or("payment_instructions can't be None"));
        let coin = self.clone();
        let fut = async move {
            let payment = try_tx_s!(coin.pay_invoice(invoice).await);
            Ok(payment.payment_hash.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_payment(
        &self,
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
        payment_instructions: &Option<PaymentInstructions>,
    ) -> TransactionFut {
        let PaymentInstructions::Lightning(invoice) =
            try_tx_fus!(payment_instructions.clone().ok_or("payment_instructions can't be None"));
        let coin = self.clone();
        let fut = async move {
            let payment = try_tx_s!(coin.pay_invoice(invoice).await);
            Ok(payment.payment_hash.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        secret: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let payment_hash = try_tx_fus!(payment_hash_from_slice(taker_payment_tx));
        let mut preimage = [b' '; 32];
        preimage.copy_from_slice(secret);

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

    fn create_taker_spends_maker_payment_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_spends_maker_payment(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_spends_maker_payment_preimage(&self, _preimage: &[u8], _secret: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_refunds_payment(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_refunds_payment(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    // Todo: This validates the dummy fee for now for the sake of swap P.O.C., this should be implemented probably after agreeing on how fees will work for lightning
    fn validate_fee(
        &self,
        _fee_tx: &TransactionEnum,
        _expected_sender: &[u8],
        _fee_addr: &[u8],
        _amount: &BigDecimal,
        _min_block_number: u64,
        _uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        Box::new(futures01::future::ok(()))
    }

    fn validate_maker_payment(&self, _input: ValidatePaymentInput) -> ValidatePaymentFut<()> { unimplemented!() }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        let payment_hash = try_f!(payment_hash_from_slice(&input.payment_tx)
            .map_to_mm(|e| ValidatePaymentError::TxDeserializationError(e.to_string())));
        let payment_hex = hex::encode(payment_hash.0);

        let amt_msat = try_f!(sat_from_big_decimal(&input.amount, self.decimals()));

        let coin = self.clone();
        let fut = async move {
            match coin.db.get_payment_from_db(payment_hash).await {
                Ok(Some(mut payment)) => {
                    let amount_sent = payment.amt_msat;
                    // Todo: Add more validations if needed, locktime is probably the most important
                    if amount_sent != Some(amt_msat as i64) {
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
                            payment_hex, amount_sent, amt_msat
                        )));
                    }
                    Ok(())
                },
                Ok(None) => MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "Payment {} should be found on the database",
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

    fn watcher_validate_taker_payment(
        &self,
        _input: WatcherValidatePaymentInput,
    ) -> Box<dyn Future<Item = (), Error = MmError<ValidatePaymentError>> + Send> {
        unimplemented!();
    }

    // Todo: This is None for now for the sake of swap P.O.C., this should be implemented probably in next PRs and should be tested across restarts
    fn check_if_my_payment_sent(
        &self,
        _time_lock: u32,
        _other_pub: &[u8],
        _secret_hash: &[u8],
        _search_from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
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

    async fn extract_secret(&self, _secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        let payment_hash = payment_hash_from_slice(spend_tx).map_err(|e| e.to_string())?;
        let payment_hex = hex::encode(payment_hash.0);

        return match self.db.get_payment_from_db(payment_hash).await {
            Ok(Some(payment)) => match payment.preimage {
                Some(preimage) => Ok(preimage.0.to_vec()),
                None => ERR!("Preimage for payment {} should be found on the database", payment_hex),
            },
            Ok(None) => ERR!("Payment {} should be found on the database", payment_hex),
            Err(e) => ERR!(
                "Unable to retrieve payment {} from the database error: {}",
                payment_hex,
                e
            ),
        };
    }

    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        Ok(None)
    }

    // Todo: This can be changed if private swaps were to be implemented for lightning
    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair {
        utxo_common::derive_htlc_key_pair(self.platform.coin.as_ref(), swap_unique_data)
    }

    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> {
        utxo_common::validate_other_pubkey(raw_pubkey)
    }

    async fn payment_instructions(
        &self,
        secret_hash: &[u8],
        amount: &BigDecimal,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        // lightning decimals should be 11 in config since the smallest divisible unit in lightning coin is msat
        let amt_msat = sat_from_big_decimal(amount, self.decimals())?;
        let payment_hash =
            payment_hash_from_slice(secret_hash).map_to_mm(|e| PaymentInstructionsErr::InternalError(e.to_string()))?;

        // note: No description is provided in the invoice to reduce the payload
        // Todo: The invoice expiry should probably be the same as maker_payment_wait/wait_taker_payment
        let invoice = self
            .create_invoice_for_hash(payment_hash, Some(amt_msat), "".into(), DEFAULT_INVOICE_EXPIRY)
            .await
            .map_err(|e| PaymentInstructionsErr::LightningInvoiceErr(e.to_string()))?;
        Ok(Some(invoice.to_string().into_bytes()))
    }

    fn validate_instructions(
        &self,
        instructions: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        let invoice = Invoice::from_str(&String::from_utf8_lossy(instructions))?;
        if invoice.payment_hash().as_inner() != secret_hash
            && ripemd160(invoice.payment_hash().as_inner()).as_slice() != secret_hash
        {
            return Err(
                ValidateInstructionsErr::ValidateLightningInvoiceErr("Invalid invoice payment hash!".into()).into(),
            );
        }
        let invoice_amount = invoice
            .amount_milli_satoshis()
            .ok_or_else(|| ValidateInstructionsErr::ValidateLightningInvoiceErr("No invoice amount!".into()))?;
        if big_decimal_from_sat(invoice_amount as i64, self.decimals()) != amount {
            return Err(ValidateInstructionsErr::ValidateLightningInvoiceErr("Invalid invoice amount!".into()).into());
        }
        // Todo: continue validation here by comparing locktime, etc..
        Ok(PaymentInstructions::Lightning(invoice))
    }
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
        Ok(zbase32::encode_full_bytes(&*signature))
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
                    Ok(Some(_)) => {
                        // Todo: This should check for different payment statuses depending on where wait_for_confirmations is called,
                        // Todo: which might lead to breaking wait_for_confirmations to 3 functions (wait_for_payment_sent_confirmations, wait_for_payment_received_confirmations, wait_for_payment_spent_confirmations)
                        return Ok(());
                    },
                    Ok(None) => info!("Payment {} not received yet!", payment_hex),
                    Err(e) => {
                        return ERR!(
                            "Error getting payment {} from db: {}, retrying in {} seconds",
                            payment_hex,
                            e,
                            check_every
                        )
                    },
                }

                // note: When sleeping for only 1 second the test_send_payment_and_swaps unit test took 20 seconds to complete instead of 37 seconds when WAIT_CONFIRM_INTERVAL (15 seconds) is used
                // Todo: In next sprints, should add a mutex for lightning swap payments to avoid overloading the shared db connection with requests when the sleep time is reduced and multiple swaps are ran together
                // Todo: The aim is to make lightning swap payments as fast as possible. Running swap payments statuses should be loaded from db on restarts in this case.
                Timer::sleep(check_every as f64).await;
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn wait_for_tx_spend(
        &self,
        transaction: &[u8],
        wait_until: u64,
        _from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
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
                    Ok(Some(payment)) => {
                        match payment.status {
                            HTLCStatus::Pending => (),
                            HTLCStatus::Received => {
                                return Err(TransactionErr::Plain(format!(
                                    "Payment {} has an invalid status of {} in the db",
                                    payment_hex, payment.status
                                )))
                            },
                            HTLCStatus::Succeeded => return Ok(TransactionEnum::LightningPayment(payment_hash)),
                            // Todo: Retry payment multiple times returning an error only if all paths failed or other permenant error, should also keep locktime in mind when using different paths with different CLTVs
                            HTLCStatus::Failed => {
                                return Err(TransactionErr::Plain(format!(
                                    "Lightning swap payment {} failed",
                                    payment_hex
                                )))
                            },
                        }
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
    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
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

    fn mature_confirmations(&self) -> Option<u32> { None }

    // Todo: This uses default data for now for the sake of swap P.O.C., this should be implemented probably when implementing order matching if it's needed
    fn coin_protocol_info(&self) -> Vec<u8> { Vec::new() }

    // Todo: This uses default data for now for the sake of swap P.O.C., this should be implemented probably when implementing order matching if it's needed
    fn is_coin_protocol_supported(&self, _info: &Option<Vec<u8>>) -> bool { true }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LightningParams {
    // The listening port for the p2p LN node
    pub listening_port: u16,
    // Printable human-readable string to describe this node to other users.
    pub node_name: [u8; 32],
    // Node's RGB color. This is used for showing the node in a network graph with the desired color.
    pub node_color: [u8; 3],
    // Invoice Payer is initialized while starting the lightning node, and it requires the number of payment retries that
    // it should do before considering a payment failed or partially failed. If not provided the number of retries will be 5
    // as this is a good default value.
    pub payment_retries: Option<usize>,
    // Node's backup path for channels and other data that requires backup.
    pub backup_path: Option<String>,
}

pub async fn start_lightning(
    ctx: &MmArc,
    platform_coin: UtxoStandardCoin,
    protocol_conf: LightningProtocolConf,
    conf: LightningCoinConf,
    params: LightningParams,
) -> EnableLightningResult<LightningCoin> {
    // Todo: add support for Hardware wallets for funding transactions and spending spendable outputs (channel closing transactions)
    if let DerivationMethod::HDWallet(_) = platform_coin.as_ref().derivation_method {
        return MmError::err(EnableLightningError::UnsupportedMode(
            "'start_lightning'".into(),
            "iguana".into(),
        ));
    }

    let platform = Arc::new(Platform::new(
        platform_coin.clone(),
        protocol_conf.network.clone(),
        protocol_conf.confirmation_targets,
    ));
    platform.set_latest_fees().await?;

    // Initialize the Logger
    let logger = ctx.log.0.clone();

    // Initialize Persister
    let persister = ln_utils::init_persister(ctx, conf.ticker.clone(), params.backup_path).await?;

    // Initialize the KeysManager
    let keys_manager = ln_utils::init_keys_manager(ctx)?;

    // Initialize the P2PGossipSync. This is used for providing routes to send payments over
    let network_graph = Arc::new(
        persister
            .get_network_graph(protocol_conf.network.into(), logger.clone())
            .await?,
    );

    let gossip_sync = Arc::new(gossip::P2PGossipSync::new(
        network_graph.clone(),
        None::<Arc<dyn Access + Send + Sync>>,
        logger.clone(),
    ));

    // Initialize DB
    let db = ln_utils::init_db(ctx, conf.ticker.clone()).await?;

    // Initialize the ChannelManager
    let (chain_monitor, channel_manager) = ln_utils::init_channel_manager(
        platform.clone(),
        logger.clone(),
        persister.clone(),
        db.clone(),
        keys_manager.clone(),
        conf.clone().into(),
    )
    .await?;

    // Initialize the PeerManager
    let peer_manager = ln_p2p::init_peer_manager(
        ctx.clone(),
        &platform,
        params.listening_port,
        channel_manager.clone(),
        gossip_sync.clone(),
        keys_manager
            .get_node_secret(Recipient::Node)
            .map_to_mm(|_| EnableLightningError::UnsupportedMode("'start_lightning'".into(), "local node".into()))?,
        logger.clone(),
    )
    .await?;

    let trusted_nodes = Arc::new(PaMutex::new(persister.get_trusted_nodes().await?));

    init_abortable_events(platform.clone(), db.clone()).await?;

    // Initialize the event handler
    let event_handler = Arc::new(ln_events::LightningEventHandler::new(
        platform.clone(),
        channel_manager.clone(),
        keys_manager.clone(),
        db.clone(),
        trusted_nodes.clone(),
    ));

    // Initialize routing Scorer
    let scorer = Arc::new(persister.get_scorer(network_graph.clone(), logger.clone()).await?);

    // Create InvoicePayer
    // random_seed_bytes are additional random seed to improve privacy by adding a random CLTV expiry offset to each path's final hop.
    // This helps obscure the intended recipient from adversarial intermediate hops. The seed is also used to randomize candidate paths during route selection.
    // TODO: random_seed_bytes should be taken in consideration when implementing swaps because they change the payment lock-time.
    // https://github.com/lightningdevkit/rust-lightning/issues/158
    // https://github.com/lightningdevkit/rust-lightning/pull/1286
    // https://github.com/lightningdevkit/rust-lightning/pull/1359
    let router = DefaultRouter::new(network_graph, logger.clone(), keys_manager.get_secure_random_bytes());
    let invoice_payer = Arc::new(InvoicePayer::new(
        channel_manager.clone(),
        router,
        scorer.clone(),
        logger.clone(),
        event_handler,
        // Todo: Add option for choosing payment::Retry::Timeout instead of Attempts in LightningParams
        payment::Retry::Attempts(params.payment_retries.unwrap_or(5)),
    ));

    // Start Background Processing. Runs tasks periodically in the background to keep LN node operational.
    // InvoicePayer will act as our event handler as it handles some of the payments related events before
    // delegating it to LightningEventHandler.
    // note: background_processor stops automatically when dropped since BackgroundProcessor implements the Drop trait.
    let background_processor = BackgroundProcessor::start(
        persister.clone(),
        invoice_payer.clone(),
        chain_monitor.clone(),
        channel_manager.clone(),
        GossipSync::p2p(gossip_sync),
        peer_manager.clone(),
        logger,
        Some(scorer),
    );
    ctx.background_processors
        .lock()
        .unwrap()
        .insert(conf.ticker.clone(), background_processor);

    // If channel_nodes_data file exists, read channels nodes data from disk and reconnect to channel nodes/peers if possible.
    let open_channels_nodes = Arc::new(PaMutex::new(
        ln_utils::get_open_channels_nodes_addresses(persister.clone(), channel_manager.clone()).await?,
    ));

    platform.spawner().spawn(ln_p2p::connect_to_ln_nodes_loop(
        open_channels_nodes.clone(),
        peer_manager.clone(),
    ));

    // Broadcast Node Announcement
    platform.spawner().spawn(ln_p2p::ln_node_announcement_loop(
        channel_manager.clone(),
        params.node_name,
        params.node_color,
        params.listening_port,
    ));

    Ok(LightningCoin {
        platform,
        conf,
        peer_manager,
        channel_manager,
        chain_monitor,
        keys_manager,
        invoice_payer,
        persister,
        db,
        open_channels_nodes,
        trusted_nodes,
    })
}

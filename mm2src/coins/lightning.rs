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

use super::{lp_coinfind_or_err, DerivationMethod, MmCoinEnum};
use crate::utxo::rpc_clients::UtxoRpcClientEnum;
use crate::utxo::utxo_common::big_decimal_from_sat_unsigned;
use crate::utxo::BlockchainNetwork;
use crate::{BalanceFut, CoinBalance, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin,
            NegotiateSwapContractAddrErr, RawTransactionFut, RawTransactionRequest, SearchForSwapTxSpendInput,
            SignatureError, SignatureResult, SwapOps, TradeFee, TradePreimageFut, TradePreimageResult,
            TradePreimageValue, TransactionEnum, TransactionFut, TxMarshalingErr, UnexpectedDerivationMethod,
            UtxoStandardCoin, ValidateAddressResult, ValidatePaymentInput, VerificationError, VerificationResult,
            WithdrawError, WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use bitcoin::hashes::Hash;
use bitcoin_hashes::sha256::Hash as Sha256;
use bitcrypto::dhash256;
use bitcrypto::ChecksumType;
use common::executor::spawn;
use common::log::{LogOnError, LogState};
use common::{async_blocking, log, now_ms, PagingOptionsEnum};
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::{hash::H256, CompactSignature, KeyPair, Private, Public};
use lightning::chain::keysinterface::{KeysInterface, KeysManager, Recipient};
use lightning::chain::Access;
use lightning::ln::channelmanager::{ChannelDetails, MIN_FINAL_CLTV_EXPIRY};
use lightning::ln::{PaymentHash, PaymentPreimage};
use lightning::routing::gossip;
use lightning_background_processor::{BackgroundProcessor, GossipSync};
use lightning_invoice::payment;
use lightning_invoice::utils::{create_invoice_from_channelmanager, DefaultRouter};
use lightning_invoice::{Invoice, InvoiceDescription};
use ln_conf::{LightningCoinConf, LightningProtocolConf, PlatformCoinConfirmationTargets};
use ln_db::{DBChannelDetails, DBPaymentInfo, HTLCStatus, LightningDB, PaymentType};
use ln_errors::{EnableLightningError, EnableLightningResult, GenerateInvoiceError, GenerateInvoiceResult};
use ln_events::{init_events_abort_handlers, LightningEventHandler};
use ln_filesystem_persister::LightningFilesystemPersister;
use ln_p2p::{connect_to_node, PeerManager};
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

#[derive(Display)]
pub(crate) enum PaymentError {
    #[display(fmt = "Final cltv expiry delta {} is below the required minimum of {}", _0, _1)]
    CLTVExpiry(u32, u32),
    #[display(fmt = "Error paying invoice: {}", _0)]
    Invoice(String),
    #[display(fmt = "Keysend error: {}", _0)]
    Keysend(String),
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

    pub(crate) async fn pay_invoice(&self, invoice: Invoice) -> Result<DBPaymentInfo, PaymentError> {
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
                .map_err(|e| PaymentError::Invoice(format!("{:?}", e)))
        })
        .await?;

        Ok(DBPaymentInfo {
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
        })
    }

    pub(crate) async fn keysend(
        &self,
        destination: PublicKey,
        amount_msat: u64,
        final_cltv_expiry_delta: u32,
    ) -> Result<DBPaymentInfo, PaymentError> {
        if final_cltv_expiry_delta < MIN_FINAL_CLTV_EXPIRY {
            return Err(PaymentError::CLTVExpiry(final_cltv_expiry_delta, MIN_FINAL_CLTV_EXPIRY));
        }
        let payment_preimage = PaymentPreimage(self.keys_manager.get_secure_random_bytes());

        let selfi = self.clone();
        async_blocking(move || {
            selfi
                .invoice_payer
                .pay_pubkey(destination, payment_preimage, amount_msat, final_cltv_expiry_delta)
                .map_err(|e| PaymentError::Keysend(format!("{:?}", e)))
        })
        .await?;

        let payment_hash = PaymentHash(Sha256::hash(&payment_preimage.0).into_inner());
        let payment_type = PaymentType::OutboundPayment { destination };

        Ok(DBPaymentInfo {
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
        })
    }

    pub(crate) async fn get_open_channels_by_filter(
        &self,
        filter: Option<OpenChannelsFilter>,
        paging: PagingOptionsEnum<u64>,
        limit: usize,
    ) -> GetOpenChannelsResult {
        fn apply_open_channel_filter(channel_details: &ChannelDetailsForRPC, filter: &OpenChannelsFilter) -> bool {
            let is_channel_id =
                filter.channel_id.is_none() || Some(&channel_details.channel_id) == filter.channel_id.as_ref();

            let is_counterparty_node_id = filter.counterparty_node_id.is_none()
                || Some(&channel_details.counterparty_node_id) == filter.counterparty_node_id.as_ref();

            let is_funding_tx = filter.funding_tx.is_none() || channel_details.funding_tx == filter.funding_tx;

            let is_from_funding_value_sats =
                Some(&channel_details.funding_tx_value_sats) >= filter.from_funding_value_sats.as_ref();

            let is_to_funding_value_sats = filter.to_funding_value_sats.is_none()
                || Some(&channel_details.funding_tx_value_sats) <= filter.to_funding_value_sats.as_ref();

            let is_outbound =
                filter.is_outbound.is_none() || Some(&channel_details.is_outbound) == filter.is_outbound.as_ref();

            let is_from_balance_msat = Some(&channel_details.balance_msat) >= filter.from_balance_msat.as_ref();

            let is_to_balance_msat = filter.to_balance_msat.is_none()
                || Some(&channel_details.balance_msat) <= filter.to_balance_msat.as_ref();

            let is_from_outbound_capacity_msat =
                Some(&channel_details.outbound_capacity_msat) >= filter.from_outbound_capacity_msat.as_ref();

            let is_to_outbound_capacity_msat = filter.to_outbound_capacity_msat.is_none()
                || Some(&channel_details.outbound_capacity_msat) <= filter.to_outbound_capacity_msat.as_ref();

            let is_from_inbound_capacity_msat =
                Some(&channel_details.inbound_capacity_msat) >= filter.from_inbound_capacity_msat.as_ref();

            let is_to_inbound_capacity_msat = filter.to_inbound_capacity_msat.is_none()
                || Some(&channel_details.inbound_capacity_msat) <= filter.to_inbound_capacity_msat.as_ref();

            let is_confirmed = filter.is_ready.is_none() || Some(&channel_details.is_ready) == filter.is_ready.as_ref();

            let is_usable = filter.is_usable.is_none() || Some(&channel_details.is_usable) == filter.is_usable.as_ref();

            let is_public = filter.is_public.is_none() || Some(&channel_details.is_public) == filter.is_public.as_ref();

            is_channel_id
                && is_counterparty_node_id
                && is_funding_tx
                && is_from_funding_value_sats
                && is_to_funding_value_sats
                && is_outbound
                && is_from_balance_msat
                && is_to_balance_msat
                && is_from_outbound_capacity_msat
                && is_to_outbound_capacity_msat
                && is_from_inbound_capacity_msat
                && is_to_inbound_capacity_msat
                && is_confirmed
                && is_usable
                && is_public
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
}

#[async_trait]
// Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
impl SwapOps for LightningCoin {
    fn send_taker_fee(&self, _fee_addr: &[u8], _amount: BigDecimal, _uuid: &[u8]) -> TransactionFut { unimplemented!() }

    fn send_maker_payment(
        &self,
        _time_lock: u32,
        _taker_pub: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_payment(
        &self,
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_spends_taker_payment(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        _secret: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
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

    fn validate_fee(
        &self,
        _fee_tx: &TransactionEnum,
        _expected_sender: &[u8],
        _fee_addr: &[u8],
        _amount: &BigDecimal,
        _min_block_number: u64,
        _uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn validate_maker_payment(
        &self,
        _input: ValidatePaymentInput,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn validate_taker_payment(
        &self,
        _input: ValidatePaymentInput,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn check_if_my_payment_sent(
        &self,
        _time_lock: u32,
        _other_pub: &[u8],
        _secret_hash: &[u8],
        _search_from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        unimplemented!()
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

    fn extract_secret(&self, _secret_hash: &[u8], _spend_tx: &[u8]) -> Result<Vec<u8>, String> { unimplemented!() }

    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        unimplemented!()
    }

    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> KeyPair { unimplemented!() }
}

impl MarketCoinOps for LightningCoin {
    fn ticker(&self) -> &str { &self.conf.ticker }

    fn my_address(&self) -> Result<String, String> { Ok(self.my_node_id()) }

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

    // Todo: Implement this when implementing swaps for lightning as it's is used mainly for swaps
    fn wait_for_confirmations(
        &self,
        _tx: &[u8],
        _confirmations: u64,
        _requires_nota: bool,
        _wait_until: u64,
        _check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    // Todo: Implement this when implementing swaps for lightning as it's is used mainly for swaps
    fn wait_for_tx_spend(
        &self,
        _transaction: &[u8],
        _wait_until: u64,
        _from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

    // Todo: Implement this when implementing swaps for lightning as it's is used mainly for swaps
    fn tx_enum_from_bytes(&self, _bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        MmError::err(TxMarshalingErr::NotSupported(
            "tx_enum_from_bytes is not supported for Lightning yet.".to_string(),
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

    // Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
    fn min_tx_amount(&self) -> BigDecimal { unimplemented!() }

    // Todo: Implement this when implementing swaps for lightning as it's is used only for order matching/swaps
    fn min_trading_vol(&self) -> MmNumber { unimplemented!() }
}

#[async_trait]
impl MmCoin for LightningCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut {
        Box::new(self.platform_coin().get_raw_transaction(req))
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

    // Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
    async fn get_sender_trade_fee(
        &self,
        _value: TradePreimageValue,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    // Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { unimplemented!() }

    // Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
    async fn get_fee_to_send_taker_fee(
        &self,
        _dex_fee_amount: BigDecimal,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    // Lightning payments are either pending, successful or failed. Once a payment succeeds there is no need to for confirmations
    // unlike onchain transactions.
    fn required_confirmations(&self) -> u64 { 0 }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, _confirmations: u64) {}

    fn set_requires_notarization(&self, _requires_nota: bool) {}

    fn swap_contract_address(&self) -> Option<BytesJson> { None }

    fn mature_confirmations(&self) -> Option<u32> { None }

    // Todo: Implement this when implementing order matching for lightning as it's is used only for order matching
    fn coin_protocol_info(&self) -> Vec<u8> { unimplemented!() }

    // Todo: Implement this when implementing order matching for lightning as it's is used only for order matching
    fn is_coin_protocol_supported(&self, _info: &Option<Vec<u8>>) -> bool { unimplemented!() }
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

    let events_abort_handlers = init_events_abort_handlers(platform.clone(), db.clone()).await?;

    // Initialize the event handler
    let event_handler = Arc::new(ln_events::LightningEventHandler::new(
        platform.clone(),
        channel_manager.clone(),
        keys_manager.clone(),
        db.clone(),
        trusted_nodes.clone(),
        events_abort_handlers,
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
    spawn(ln_p2p::connect_to_nodes_loop(
        open_channels_nodes.clone(),
        peer_manager.clone(),
    ));

    // Broadcast Node Announcement
    spawn(ln_p2p::ln_node_announcement_loop(
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

#[derive(Deserialize)]
pub struct GenerateInvoiceRequest {
    pub coin: String,
    pub amount_in_msat: Option<u64>,
    pub description: String,
    pub expiry: Option<u32>,
}

#[derive(Serialize)]
pub struct GenerateInvoiceResponse {
    payment_hash: H256Json,
    invoice: Invoice,
}

/// Generates an invoice (request for payment) that can be paid on the lightning network by another node using send_payment.
pub async fn generate_invoice(
    ctx: MmArc,
    req: GenerateInvoiceRequest,
) -> GenerateInvoiceResult<GenerateInvoiceResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(GenerateInvoiceError::UnsupportedCoin(e.ticker().to_string())),
    };
    let open_channels_nodes = ln_coin.open_channels_nodes.lock().clone();
    for (node_pubkey, node_addr) in open_channels_nodes {
        connect_to_node(node_pubkey, node_addr, ln_coin.peer_manager.clone())
            .await
            .error_log_with_msg(&format!(
                "Channel with node: {} can't be used for invoice routing hints due to connection error.",
                node_pubkey
            ));
    }

    let network = ln_coin.platform.network.clone().into();
    let channel_manager = ln_coin.channel_manager.clone();
    let keys_manager = ln_coin.keys_manager.clone();
    let amount_in_msat = req.amount_in_msat;
    let description = req.description.clone();
    let expiry = req.expiry.unwrap_or(DEFAULT_INVOICE_EXPIRY);
    let invoice = async_blocking(move || {
        create_invoice_from_channelmanager(
            &channel_manager,
            keys_manager,
            network,
            amount_in_msat,
            description,
            expiry,
        )
    })
    .await?;

    let payment_hash = invoice.payment_hash().into_inner();
    let payment_info = DBPaymentInfo {
        payment_hash: PaymentHash(payment_hash),
        payment_type: PaymentType::InboundPayment,
        description: req.description,
        preimage: None,
        secret: Some(*invoice.payment_secret()),
        amt_msat: req.amount_in_msat.map(|a| a as i64),
        fee_paid_msat: None,
        status: HTLCStatus::Pending,
        created_at: (now_ms() / 1000) as i64,
        last_updated: (now_ms() / 1000) as i64,
    };
    ln_coin.db.add_or_update_payment_in_db(payment_info).await?;
    Ok(GenerateInvoiceResponse {
        payment_hash: payment_hash.into(),
        invoice,
    })
}

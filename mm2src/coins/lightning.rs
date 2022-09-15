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
use crate::lightning::ln_errors::{TrustedNodeError, TrustedNodeResult, UpdateChannelError, UpdateChannelResult};
use crate::lightning::ln_events::init_events_abort_handlers;
use crate::lightning::ln_serialization::PublicKeyForRPC;
use crate::lightning::ln_sql::SqliteLightningDB;
use crate::lightning::ln_storage::{NetworkGraph, TrustedNodesShared};
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
use common::{async_blocking, calc_total_pages, log, now_ms, ten, PagingOptionsEnum};
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::{hash::H256, CompactSignature, KeyPair, Private, Public};
use lightning::chain::channelmonitor::Balance;
use lightning::chain::keysinterface::{KeysInterface, KeysManager, Recipient};
use lightning::chain::Access;
use lightning::ln::channelmanager::{ChannelDetails, MIN_FINAL_CLTV_EXPIRY};
use lightning::ln::{PaymentHash, PaymentPreimage};
use lightning::routing::gossip;
use lightning_background_processor::{BackgroundProcessor, GossipSync};
use lightning_invoice::payment;
use lightning_invoice::utils::{create_invoice_from_channelmanager, DefaultRouter};
use lightning_invoice::{Invoice, InvoiceDescription};
use ln_conf::{ChannelOptions, LightningCoinConf, LightningProtocolConf, PlatformCoinConfirmationTargets};
use ln_db::{ClosedChannelsFilter, DBChannelDetails, DBPaymentInfo, DBPaymentsFilter, HTLCStatus, LightningDB,
            PaymentType};
use ln_errors::{ClaimableBalancesError, ClaimableBalancesResult, CloseChannelError, CloseChannelResult,
                EnableLightningError, EnableLightningResult, GenerateInvoiceError, GenerateInvoiceResult,
                GetChannelDetailsError, GetChannelDetailsResult, GetPaymentDetailsError, GetPaymentDetailsResult,
                ListChannelsError, ListChannelsResult, ListPaymentsError, ListPaymentsResult, SendPaymentError,
                SendPaymentResult};
use ln_events::LightningEventHandler;
use ln_filesystem_persister::LightningFilesystemPersister;
use ln_p2p::{connect_to_node, PeerManager};
use ln_platform::{h256_json_from_txid, Platform};
use ln_storage::{LightningStorage, NodesAddressesMapShared, Scorer};
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

impl LightningCoin {
    pub fn platform_coin(&self) -> &UtxoStandardCoin { &self.platform.coin }

    #[inline]
    fn my_node_id(&self) -> String { self.channel_manager.get_our_node_id().to_string() }

    async fn list_channels(&self) -> Vec<ChannelDetails> {
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

    async fn get_channel_by_rpc_id(&self, rpc_id: u64) -> Option<ChannelDetails> {
        self.list_channels()
            .await
            .into_iter()
            .find(|chan| chan.user_channel_id == rpc_id)
    }

    async fn pay_invoice(&self, invoice: Invoice) -> SendPaymentResult<DBPaymentInfo> {
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
                .map_to_mm(|e| SendPaymentError::PaymentError(format!("{:?}", e)))
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

    async fn keysend(
        &self,
        destination: PublicKey,
        amount_msat: u64,
        final_cltv_expiry_delta: u32,
    ) -> SendPaymentResult<DBPaymentInfo> {
        if final_cltv_expiry_delta < MIN_FINAL_CLTV_EXPIRY {
            return MmError::err(SendPaymentError::CLTVExpiryError(
                final_cltv_expiry_delta,
                MIN_FINAL_CLTV_EXPIRY,
            ));
        }
        let payment_preimage = PaymentPreimage(self.keys_manager.get_secure_random_bytes());

        let selfi = self.clone();
        async_blocking(move || {
            selfi
                .invoice_payer
                .pay_pubkey(destination, payment_preimage, amount_msat, final_cltv_expiry_delta)
                .map_to_mm(|e| SendPaymentError::PaymentError(format!("{:?}", e)))
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

    async fn get_open_channels_by_filter(
        &self,
        filter: Option<OpenChannelsFilter>,
        paging: PagingOptionsEnum<u64>,
        limit: usize,
    ) -> ListChannelsResult<GetOpenChannelsResult> {
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

        Ok(GetOpenChannelsResult {
            channels,
            skipped: offset,
            total,
        })
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
pub struct UpdateChannelReq {
    pub coin: String,
    pub rpc_channel_id: u64,
    pub channel_options: ChannelOptions,
}

#[derive(Serialize)]
pub struct UpdateChannelResponse {
    channel_options: ChannelOptions,
}

/// Updates configuration for an open channel.
pub async fn update_channel(ctx: MmArc, req: UpdateChannelReq) -> UpdateChannelResult<UpdateChannelResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(UpdateChannelError::UnsupportedCoin(e.ticker().to_string())),
    };

    let channel_details = ln_coin
        .get_channel_by_rpc_id(req.rpc_channel_id)
        .await
        .ok_or(UpdateChannelError::NoSuchChannel(req.rpc_channel_id))?;

    async_blocking(move || {
        let mut channel_options = ln_coin
            .conf
            .channel_options
            .unwrap_or_else(|| req.channel_options.clone());
        if channel_options != req.channel_options {
            channel_options.update_according_to(req.channel_options.clone());
        }
        drop_mutability!(channel_options);
        let channel_ids = &[channel_details.channel_id];
        let counterparty_node_id = channel_details.counterparty.node_id;
        ln_coin
            .channel_manager
            .update_channel_config(&counterparty_node_id, channel_ids, &channel_options.clone().into())
            .map_to_mm(|e| UpdateChannelError::FailureToUpdateChannel(req.rpc_channel_id, format!("{:?}", e)))?;
        Ok(UpdateChannelResponse { channel_options })
    })
    .await
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

fn apply_open_channel_filter(channel_details: &ChannelDetailsForRPC, filter: &OpenChannelsFilter) -> bool {
    let is_channel_id = filter.channel_id.is_none() || Some(&channel_details.channel_id) == filter.channel_id.as_ref();

    let is_counterparty_node_id = filter.counterparty_node_id.is_none()
        || Some(&channel_details.counterparty_node_id) == filter.counterparty_node_id.as_ref();

    let is_funding_tx = filter.funding_tx.is_none() || channel_details.funding_tx == filter.funding_tx;

    let is_from_funding_value_sats =
        Some(&channel_details.funding_tx_value_sats) >= filter.from_funding_value_sats.as_ref();

    let is_to_funding_value_sats = filter.to_funding_value_sats.is_none()
        || Some(&channel_details.funding_tx_value_sats) <= filter.to_funding_value_sats.as_ref();

    let is_outbound = filter.is_outbound.is_none() || Some(&channel_details.is_outbound) == filter.is_outbound.as_ref();

    let is_from_balance_msat = Some(&channel_details.balance_msat) >= filter.from_balance_msat.as_ref();

    let is_to_balance_msat =
        filter.to_balance_msat.is_none() || Some(&channel_details.balance_msat) <= filter.to_balance_msat.as_ref();

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

#[derive(Deserialize)]
pub struct ListOpenChannelsRequest {
    pub coin: String,
    pub filter: Option<OpenChannelsFilter>,
    #[serde(default = "ten")]
    limit: usize,
    #[serde(default)]
    paging_options: PagingOptionsEnum<u64>,
}

#[derive(Clone, Serialize)]
pub struct ChannelDetailsForRPC {
    pub rpc_channel_id: u64,
    pub channel_id: H256Json,
    pub counterparty_node_id: PublicKeyForRPC,
    pub funding_tx: Option<H256Json>,
    pub funding_tx_output_index: Option<u16>,
    pub funding_tx_value_sats: u64,
    /// True if the channel was initiated (and thus funded) by us.
    pub is_outbound: bool,
    pub balance_msat: u64,
    pub outbound_capacity_msat: u64,
    pub inbound_capacity_msat: u64,
    // Channel is confirmed onchain, this means that funding_locked messages have been exchanged,
    // the channel is not currently being shut down, and the required confirmation count has been reached.
    pub is_ready: bool,
    // Channel is confirmed and channel_ready messages have been exchanged, the peer is connected,
    // and the channel is not currently negotiating a shutdown.
    pub is_usable: bool,
    // A publicly-announced channel.
    pub is_public: bool,
}

impl From<ChannelDetails> for ChannelDetailsForRPC {
    fn from(details: ChannelDetails) -> ChannelDetailsForRPC {
        ChannelDetailsForRPC {
            rpc_channel_id: details.user_channel_id,
            channel_id: details.channel_id.into(),
            counterparty_node_id: PublicKeyForRPC(details.counterparty.node_id),
            funding_tx: details.funding_txo.map(|tx| h256_json_from_txid(tx.txid)),
            funding_tx_output_index: details.funding_txo.map(|tx| tx.index),
            funding_tx_value_sats: details.channel_value_satoshis,
            is_outbound: details.is_outbound,
            balance_msat: details.balance_msat,
            outbound_capacity_msat: details.outbound_capacity_msat,
            inbound_capacity_msat: details.inbound_capacity_msat,
            is_ready: details.is_channel_ready,
            is_usable: details.is_usable,
            is_public: details.is_public,
        }
    }
}

struct GetOpenChannelsResult {
    pub channels: Vec<ChannelDetailsForRPC>,
    pub skipped: usize,
    pub total: usize,
}

#[derive(Serialize)]
pub struct ListOpenChannelsResponse {
    open_channels: Vec<ChannelDetailsForRPC>,
    limit: usize,
    skipped: usize,
    total: usize,
    total_pages: usize,
    paging_options: PagingOptionsEnum<u64>,
}

pub async fn list_open_channels_by_filter(
    ctx: MmArc,
    req: ListOpenChannelsRequest,
) -> ListChannelsResult<ListOpenChannelsResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(ListChannelsError::UnsupportedCoin(e.ticker().to_string())),
    };

    let result = ln_coin
        .get_open_channels_by_filter(req.filter, req.paging_options.clone(), req.limit)
        .await?;

    Ok(ListOpenChannelsResponse {
        open_channels: result.channels,
        limit: req.limit,
        skipped: result.skipped,
        total: result.total,
        total_pages: calc_total_pages(result.total, req.limit),
        paging_options: req.paging_options,
    })
}

#[derive(Deserialize)]
pub struct ListClosedChannelsRequest {
    pub coin: String,
    pub filter: Option<ClosedChannelsFilter>,
    #[serde(default = "ten")]
    limit: usize,
    #[serde(default)]
    paging_options: PagingOptionsEnum<u64>,
}

#[derive(Serialize)]
pub struct ListClosedChannelsResponse {
    closed_channels: Vec<DBChannelDetails>,
    limit: usize,
    skipped: usize,
    total: usize,
    total_pages: usize,
    paging_options: PagingOptionsEnum<u64>,
}

pub async fn list_closed_channels_by_filter(
    ctx: MmArc,
    req: ListClosedChannelsRequest,
) -> ListChannelsResult<ListClosedChannelsResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(ListChannelsError::UnsupportedCoin(e.ticker().to_string())),
    };
    let closed_channels_res = ln_coin
        .db
        .get_closed_channels_by_filter(req.filter, req.paging_options.clone(), req.limit)
        .await?;

    Ok(ListClosedChannelsResponse {
        closed_channels: closed_channels_res.channels,
        limit: req.limit,
        skipped: closed_channels_res.skipped,
        total: closed_channels_res.total,
        total_pages: calc_total_pages(closed_channels_res.total, req.limit),
        paging_options: req.paging_options,
    })
}

#[derive(Deserialize)]
pub struct GetChannelDetailsRequest {
    pub coin: String,
    pub rpc_channel_id: u64,
}

#[derive(Serialize)]
#[serde(tag = "status", content = "details")]
pub enum GetChannelDetailsResponse {
    Open(ChannelDetailsForRPC),
    Closed(DBChannelDetails),
}

pub async fn get_channel_details(
    ctx: MmArc,
    req: GetChannelDetailsRequest,
) -> GetChannelDetailsResult<GetChannelDetailsResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(GetChannelDetailsError::UnsupportedCoin(e.ticker().to_string())),
    };

    let channel_details = match ln_coin.get_channel_by_rpc_id(req.rpc_channel_id).await {
        Some(details) => GetChannelDetailsResponse::Open(details.into()),
        None => GetChannelDetailsResponse::Closed(
            ln_coin
                .db
                .get_channel_from_db(req.rpc_channel_id)
                .await?
                .ok_or(GetChannelDetailsError::NoSuchChannel(req.rpc_channel_id))?,
        ),
    };

    Ok(channel_details)
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

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Payment {
    #[serde(rename = "invoice")]
    Invoice { invoice: Invoice },
    #[serde(rename = "keysend")]
    Keysend {
        // The recieving node pubkey (node ID)
        destination: PublicKeyForRPC,
        // Amount to send in millisatoshis
        amount_in_msat: u64,
        // The number of blocks the payment will be locked for if not claimed by the destination,
        // It's can be assumed that 6 blocks = 1 hour. We can claim the payment amount back after this cltv expires.
        // Minmum value allowed is MIN_FINAL_CLTV_EXPIRY which is currently 24 for rust-lightning.
        expiry: u32,
    },
}

#[derive(Deserialize)]
pub struct SendPaymentReq {
    pub coin: String,
    pub payment: Payment,
}

#[derive(Serialize)]
pub struct SendPaymentResponse {
    payment_hash: H256Json,
}

pub async fn send_payment(ctx: MmArc, req: SendPaymentReq) -> SendPaymentResult<SendPaymentResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(SendPaymentError::UnsupportedCoin(e.ticker().to_string())),
    };
    let open_channels_nodes = ln_coin.open_channels_nodes.lock().clone();
    for (node_pubkey, node_addr) in open_channels_nodes {
        connect_to_node(node_pubkey, node_addr, ln_coin.peer_manager.clone())
            .await
            .error_log_with_msg(&format!(
                "Channel with node: {} can't be used to route this payment due to connection error.",
                node_pubkey
            ));
    }
    let payment_info = match req.payment {
        Payment::Invoice { invoice } => ln_coin.pay_invoice(invoice).await?,
        Payment::Keysend {
            destination,
            amount_in_msat,
            expiry,
        } => ln_coin.keysend(destination.into(), amount_in_msat, expiry).await?,
    };
    ln_coin.db.add_or_update_payment_in_db(payment_info.clone()).await?;
    Ok(SendPaymentResponse {
        payment_hash: payment_info.payment_hash.0.into(),
    })
}

#[derive(Deserialize)]
pub struct PaymentsFilterForRPC {
    pub payment_type: Option<PaymentTypeForRPC>,
    pub description: Option<String>,
    pub status: Option<HTLCStatus>,
    pub from_amount_msat: Option<u64>,
    pub to_amount_msat: Option<u64>,
    pub from_fee_paid_msat: Option<u64>,
    pub to_fee_paid_msat: Option<u64>,
    pub from_timestamp: Option<u64>,
    pub to_timestamp: Option<u64>,
}

impl From<PaymentsFilterForRPC> for DBPaymentsFilter {
    fn from(filter: PaymentsFilterForRPC) -> Self {
        let (is_outbound, destination) = if let Some(payment_type) = filter.payment_type {
            match payment_type {
                PaymentTypeForRPC::OutboundPayment { destination } => (Some(true), Some(destination.0.to_string())),
                PaymentTypeForRPC::InboundPayment => (Some(false), None),
            }
        } else {
            (None, None)
        };
        DBPaymentsFilter {
            is_outbound,
            destination,
            description: filter.description,
            status: filter.status.map(|s| s.to_string()),
            from_amount_msat: filter.from_amount_msat.map(|a| a as i64),
            to_amount_msat: filter.to_amount_msat.map(|a| a as i64),
            from_fee_paid_msat: filter.from_fee_paid_msat.map(|f| f as i64),
            to_fee_paid_msat: filter.to_fee_paid_msat.map(|f| f as i64),
            from_timestamp: filter.from_timestamp.map(|f| f as i64),
            to_timestamp: filter.to_timestamp.map(|f| f as i64),
        }
    }
}

#[derive(Deserialize)]
pub struct ListPaymentsReq {
    pub coin: String,
    pub filter: Option<PaymentsFilterForRPC>,
    #[serde(default = "ten")]
    limit: usize,
    #[serde(default)]
    paging_options: PagingOptionsEnum<H256Json>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum PaymentTypeForRPC {
    #[serde(rename = "Outbound Payment")]
    OutboundPayment { destination: PublicKeyForRPC },
    #[serde(rename = "Inbound Payment")]
    InboundPayment,
}

impl From<PaymentType> for PaymentTypeForRPC {
    fn from(payment_type: PaymentType) -> Self {
        match payment_type {
            PaymentType::OutboundPayment { destination } => PaymentTypeForRPC::OutboundPayment {
                destination: PublicKeyForRPC(destination),
            },
            PaymentType::InboundPayment => PaymentTypeForRPC::InboundPayment,
        }
    }
}

impl From<PaymentTypeForRPC> for PaymentType {
    fn from(payment_type: PaymentTypeForRPC) -> Self {
        match payment_type {
            PaymentTypeForRPC::OutboundPayment { destination } => PaymentType::OutboundPayment {
                destination: destination.into(),
            },
            PaymentTypeForRPC::InboundPayment => PaymentType::InboundPayment,
        }
    }
}

#[derive(Serialize)]
pub struct PaymentInfoForRPC {
    payment_hash: H256Json,
    payment_type: PaymentTypeForRPC,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_in_msat: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fee_paid_msat: Option<i64>,
    status: HTLCStatus,
    created_at: i64,
    last_updated: i64,
}

impl From<DBPaymentInfo> for PaymentInfoForRPC {
    fn from(info: DBPaymentInfo) -> Self {
        PaymentInfoForRPC {
            payment_hash: info.payment_hash.0.into(),
            payment_type: info.payment_type.into(),
            description: info.description,
            amount_in_msat: info.amt_msat,
            fee_paid_msat: info.fee_paid_msat,
            status: info.status,
            created_at: info.created_at,
            last_updated: info.last_updated,
        }
    }
}

#[derive(Serialize)]
pub struct ListPaymentsResponse {
    payments: Vec<PaymentInfoForRPC>,
    limit: usize,
    skipped: usize,
    total: usize,
    total_pages: usize,
    paging_options: PagingOptionsEnum<H256Json>,
}

pub async fn list_payments_by_filter(ctx: MmArc, req: ListPaymentsReq) -> ListPaymentsResult<ListPaymentsResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(ListPaymentsError::UnsupportedCoin(e.ticker().to_string())),
    };
    let get_payments_res = ln_coin
        .db
        .get_payments_by_filter(
            req.filter.map(From::from),
            req.paging_options.clone().map(|h| PaymentHash(h.0)),
            req.limit,
        )
        .await?;

    Ok(ListPaymentsResponse {
        payments: get_payments_res.payments.into_iter().map(From::from).collect(),
        limit: req.limit,
        skipped: get_payments_res.skipped,
        total: get_payments_res.total,
        total_pages: calc_total_pages(get_payments_res.total, req.limit),
        paging_options: req.paging_options,
    })
}

#[derive(Deserialize)]
pub struct GetPaymentDetailsRequest {
    pub coin: String,
    pub payment_hash: H256Json,
}

#[derive(Serialize)]
pub struct GetPaymentDetailsResponse {
    payment_details: PaymentInfoForRPC,
}

pub async fn get_payment_details(
    ctx: MmArc,
    req: GetPaymentDetailsRequest,
) -> GetPaymentDetailsResult<GetPaymentDetailsResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(GetPaymentDetailsError::UnsupportedCoin(e.ticker().to_string())),
    };

    if let Some(payment_info) = ln_coin.db.get_payment_from_db(PaymentHash(req.payment_hash.0)).await? {
        return Ok(GetPaymentDetailsResponse {
            payment_details: payment_info.into(),
        });
    }

    MmError::err(GetPaymentDetailsError::NoSuchPayment(req.payment_hash))
}

#[derive(Deserialize)]
pub struct CloseChannelReq {
    pub coin: String,
    pub rpc_channel_id: u64,
    #[serde(default)]
    pub force_close: bool,
}

pub async fn close_channel(ctx: MmArc, req: CloseChannelReq) -> CloseChannelResult<String> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(CloseChannelError::UnsupportedCoin(e.ticker().to_string())),
    };

    let channel_details = ln_coin
        .get_channel_by_rpc_id(req.rpc_channel_id)
        .await
        .ok_or(CloseChannelError::NoSuchChannel(req.rpc_channel_id))?;
    let channel_id = channel_details.channel_id;
    let counterparty_node_id = channel_details.counterparty.node_id;

    if req.force_close {
        async_blocking(move || {
            ln_coin
                .channel_manager
                .force_close_broadcasting_latest_txn(&channel_id, &counterparty_node_id)
                .map_to_mm(|e| CloseChannelError::CloseChannelError(format!("{:?}", e)))
        })
        .await?;
    } else {
        async_blocking(move || {
            ln_coin
                .channel_manager
                .close_channel(&channel_id, &counterparty_node_id)
                .map_to_mm(|e| CloseChannelError::CloseChannelError(format!("{:?}", e)))
        })
        .await?;
    }

    Ok(format!(
        "Initiated closing of channel with rpc_channel_id: {}",
        req.rpc_channel_id
    ))
}

/// Details about the balance(s) available for spending once the channel appears on chain.
#[derive(Serialize)]
pub enum ClaimableBalance {
    /// The channel is not yet closed (or the commitment or closing transaction has not yet
    /// appeared in a block). The given balance is claimable (less on-chain fees) if the channel is
    /// force-closed now.
    ClaimableOnChannelClose {
        /// The amount available to claim, in satoshis, excluding the on-chain fees which will be
        /// required to do so.
        claimable_amount_satoshis: u64,
    },
    /// The channel has been closed, and the given balance is ours but awaiting confirmations until
    /// we consider it spendable.
    ClaimableAwaitingConfirmations {
        /// The amount available to claim, in satoshis, possibly excluding the on-chain fees which
        /// were spent in broadcasting the transaction.
        claimable_amount_satoshis: u64,
        /// The height at which an [`Event::SpendableOutputs`] event will be generated for this
        /// amount.
        confirmation_height: u32,
    },
    /// The channel has been closed, and the given balance should be ours but awaiting spending
    /// transaction confirmation. If the spending transaction does not confirm in time, it is
    /// possible our counterparty can take the funds by broadcasting an HTLC timeout on-chain.
    ///
    /// Once the spending transaction confirms, before it has reached enough confirmations to be
    /// considered safe from chain reorganizations, the balance will instead be provided via
    /// [`Balance::ClaimableAwaitingConfirmations`].
    ContentiousClaimable {
        /// The amount available to claim, in satoshis, excluding the on-chain fees which will be
        /// required to do so.
        claimable_amount_satoshis: u64,
        /// The height at which the counterparty may be able to claim the balance if we have not
        /// done so.
        timeout_height: u32,
    },
    /// HTLCs which we sent to our counterparty which are claimable after a timeout (less on-chain
    /// fees) if the counterparty does not know the preimage for the HTLCs. These are somewhat
    /// likely to be claimed by our counterparty before we do.
    MaybeClaimableHTLCAwaitingTimeout {
        /// The amount available to claim, in satoshis, excluding the on-chain fees which will be
        /// required to do so.
        claimable_amount_satoshis: u64,
        /// The height at which we will be able to claim the balance if our counterparty has not
        /// done so.
        claimable_height: u32,
    },
}

impl From<Balance> for ClaimableBalance {
    fn from(balance: Balance) -> Self {
        match balance {
            Balance::ClaimableOnChannelClose {
                claimable_amount_satoshis,
            } => ClaimableBalance::ClaimableOnChannelClose {
                claimable_amount_satoshis,
            },
            Balance::ClaimableAwaitingConfirmations {
                claimable_amount_satoshis,
                confirmation_height,
            } => ClaimableBalance::ClaimableAwaitingConfirmations {
                claimable_amount_satoshis,
                confirmation_height,
            },
            Balance::ContentiousClaimable {
                claimable_amount_satoshis,
                timeout_height,
            } => ClaimableBalance::ContentiousClaimable {
                claimable_amount_satoshis,
                timeout_height,
            },
            Balance::MaybeClaimableHTLCAwaitingTimeout {
                claimable_amount_satoshis,
                claimable_height,
            } => ClaimableBalance::MaybeClaimableHTLCAwaitingTimeout {
                claimable_amount_satoshis,
                claimable_height,
            },
        }
    }
}

#[derive(Deserialize)]
pub struct ClaimableBalancesReq {
    pub coin: String,
    #[serde(default)]
    pub include_open_channels_balances: bool,
}

pub async fn get_claimable_balances(
    ctx: MmArc,
    req: ClaimableBalancesReq,
) -> ClaimableBalancesResult<Vec<ClaimableBalance>> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(ClaimableBalancesError::UnsupportedCoin(e.ticker().to_string())),
    };
    let ignored_channels = if req.include_open_channels_balances {
        Vec::new()
    } else {
        ln_coin.list_channels().await
    };
    let claimable_balances = async_blocking(move || {
        ln_coin
            .chain_monitor
            .get_claimable_balances(&ignored_channels.iter().collect::<Vec<_>>()[..])
            .into_iter()
            .map(From::from)
            .collect()
    })
    .await;

    Ok(claimable_balances)
}

#[derive(Deserialize)]
pub struct AddTrustedNodeReq {
    pub coin: String,
    pub node_id: PublicKeyForRPC,
}

#[derive(Serialize)]
pub struct AddTrustedNodeResponse {
    pub added_node: PublicKeyForRPC,
}

pub async fn add_trusted_node(ctx: MmArc, req: AddTrustedNodeReq) -> TrustedNodeResult<AddTrustedNodeResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(TrustedNodeError::UnsupportedCoin(e.ticker().to_string())),
    };

    if ln_coin.trusted_nodes.lock().insert(req.node_id.clone().into()) {
        ln_coin.persister.save_trusted_nodes(ln_coin.trusted_nodes).await?;
    }

    Ok(AddTrustedNodeResponse {
        added_node: req.node_id,
    })
}

#[derive(Deserialize)]
pub struct RemoveTrustedNodeReq {
    pub coin: String,
    pub node_id: PublicKeyForRPC,
}

#[derive(Serialize)]
pub struct RemoveTrustedNodeResponse {
    pub removed_node: PublicKeyForRPC,
}

pub async fn remove_trusted_node(
    ctx: MmArc,
    req: RemoveTrustedNodeReq,
) -> TrustedNodeResult<RemoveTrustedNodeResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(TrustedNodeError::UnsupportedCoin(e.ticker().to_string())),
    };

    if ln_coin.trusted_nodes.lock().remove(&req.node_id.clone().into()) {
        ln_coin.persister.save_trusted_nodes(ln_coin.trusted_nodes).await?;
    }

    Ok(RemoveTrustedNodeResponse {
        removed_node: req.node_id,
    })
}

#[derive(Deserialize)]
pub struct ListTrustedNodesReq {
    pub coin: String,
}

#[derive(Serialize)]
pub struct ListTrustedNodesResponse {
    trusted_nodes: Vec<PublicKeyForRPC>,
}

pub async fn list_trusted_nodes(ctx: MmArc, req: ListTrustedNodesReq) -> TrustedNodeResult<ListTrustedNodesResponse> {
    let ln_coin = match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::LightningCoin(c) => c,
        e => return MmError::err(TrustedNodeError::UnsupportedCoin(e.ticker().to_string())),
    };

    let trusted_nodes = ln_coin.trusted_nodes.lock().clone();

    Ok(ListTrustedNodesResponse {
        trusted_nodes: trusted_nodes.into_iter().map(PublicKeyForRPC).collect(),
    })
}

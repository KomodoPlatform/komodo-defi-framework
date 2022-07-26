pub mod ln_conf;
pub mod ln_errors;
mod ln_events;
mod ln_p2p;
mod ln_platform;
mod ln_serialization;
mod ln_utils;

use super::{lp_coinfind_or_err, DerivationMethod, MmCoinEnum};
use crate::utxo::rpc_clients::UtxoRpcClientEnum;
use crate::utxo::utxo_common::{big_decimal_from_sat_unsigned, UtxoTxBuilder};
use crate::utxo::{sat_from_big_decimal, BlockchainNetwork, FeePolicy, GetUtxoListOps, UtxoTxGenerationOps};
use crate::{BalanceFut, CoinBalance, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin,
            NegotiateSwapContractAddrErr, RawTransactionFut, RawTransactionRequest, SearchForSwapTxSpendInput,
            SignatureError, SignatureResult, SwapOps, TradeFee, TradePreimageFut, TradePreimageResult,
            TradePreimageValue, TransactionEnum, TransactionFut, UnexpectedDerivationMethod, UtxoStandardCoin,
            ValidateAddressResult, ValidatePaymentInput, VerificationError, VerificationResult, WithdrawError,
            WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use bitcoin::hashes::Hash;
use bitcoin_hashes::sha256::Hash as Sha256;
use bitcrypto::dhash256;
use bitcrypto::ChecksumType;
use chain::TransactionOutput;
use common::executor::spawn;
use common::log::{LogOnError, LogState};
use common::{async_blocking, calc_total_pages, log, now_ms, ten, PagingOptionsEnum};
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::{hash::H256, AddressHashEnum, CompactSignature, KeyPair, Private, Public};
use lightning::chain::channelmonitor::Balance;
use lightning::chain::keysinterface::{KeysInterface, KeysManager, Recipient};
use lightning::chain::Access;
use lightning::ln::channelmanager::{ChannelDetails, MIN_FINAL_CLTV_EXPIRY};
use lightning::ln::{PaymentHash, PaymentPreimage};
use lightning::routing::network_graph::{NetGraphMsgHandler, NetworkGraph};
use lightning::util::config::UserConfig;
use lightning_background_processor::BackgroundProcessor;
use lightning_invoice::payment;
use lightning_invoice::utils::{create_invoice_from_channelmanager, DefaultRouter};
use lightning_invoice::{Invoice, InvoiceDescription};
use lightning_persister::storage::{ClosedChannelsFilter, DbStorage, FileSystemStorage, HTLCStatus,
                                   NodesAddressesMapShared, PaymentInfo, PaymentType, PaymentsFilter, Scorer,
                                   SqlChannelDetails};
use lightning_persister::LightningPersister;
use ln_conf::{ChannelOptions, LightningCoinConf, LightningProtocolConf, PlatformCoinConfirmations};
use ln_errors::{ClaimableBalancesError, ClaimableBalancesResult, CloseChannelError, CloseChannelResult,
                ConnectToNodeError, ConnectToNodeResult, EnableLightningError, EnableLightningResult,
                GenerateInvoiceError, GenerateInvoiceResult, GetChannelDetailsError, GetChannelDetailsResult,
                GetPaymentDetailsError, GetPaymentDetailsResult, ListChannelsError, ListChannelsResult,
                ListPaymentsError, ListPaymentsResult, OpenChannelError, OpenChannelResult, SendPaymentError,
                SendPaymentResult};
use ln_events::LightningEventHandler;
use ln_p2p::{connect_to_node, ConnectToNodeRes, PeerManager};
use ln_platform::{h256_json_from_txid, Platform};
use ln_serialization::{InvoiceForRPC, NodeAddress, PublicKeyForRPC};
use ln_utils::{ChainMonitor, ChannelManager};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_net::ip_addr::myipaddr;
use mm2_number::{BigDecimal, MmNumber};
use parking_lot::Mutex as PaMutex;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use script::{Builder, TransactionInputSigner};
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

type Router = DefaultRouter<Arc<NetworkGraph>, Arc<LogState>>;
type InvoicePayer<E> = payment::InvoicePayer<Arc<ChannelManager>, Router, Arc<Mutex<Scorer>>, Arc<LogState>, E>;

#[derive(Clone)]
pub struct LightningCoin {
    pub platform: Arc<Platform>,
    pub conf: LightningCoinConf,
    /// The lightning node peer manager that takes care of connecting to peers, etc..
    pub peer_manager: Arc<PeerManager>,
    /// The lightning node background processor that takes care of tasks that need to happen periodically
    pub background_processor: Arc<BackgroundProcessor>,
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
    pub persister: Arc<LightningPersister>,
    /// The mutex storing the addresses of the nodes that the lightning node has open channels with,
    /// these addresses are used for reconnecting.
    pub open_channels_nodes: NodesAddressesMapShared,
}

impl fmt::Debug for LightningCoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "LightningCoin {{ conf: {:?} }}", self.conf) }
}

impl LightningCoin {
    fn platform_coin(&self) -> &UtxoStandardCoin { &self.platform.coin }

    #[inline]
    fn my_node_id(&self) -> String { self.channel_manager.get_our_node_id().to_string() }

    fn get_balance_msat(&self) -> (u64, u64) {
        self.channel_manager
            .list_channels()
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

    fn pay_invoice(&self, invoice: Invoice) -> SendPaymentResult<PaymentInfo> {
        self.invoice_payer
            .pay_invoice(&invoice)
            .map_to_mm(|e| SendPaymentError::PaymentError(format!("{:?}", e)))?;
        let payment_hash = PaymentHash((*invoice.payment_hash()).into_inner());
        let payment_type = PaymentType::OutboundPayment {
            destination: *invoice.payee_pub_key().unwrap_or(&invoice.recover_payee_pub_key()),
        };
        let description = match invoice.description() {
            InvoiceDescription::Direct(d) => d.to_string(),
            InvoiceDescription::Hash(h) => hex::encode(h.0.into_inner()),
        };
        let payment_secret = Some(*invoice.payment_secret());
        Ok(PaymentInfo {
            payment_hash,
            payment_type,
            description,
            preimage: None,
            secret: payment_secret,
            amt_msat: invoice.amount_milli_satoshis(),
            fee_paid_msat: None,
            status: HTLCStatus::Pending,
            created_at: now_ms() / 1000,
            last_updated: now_ms() / 1000,
        })
    }

    fn keysend(
        &self,
        destination: PublicKey,
        amount_msat: u64,
        final_cltv_expiry_delta: u32,
    ) -> SendPaymentResult<PaymentInfo> {
        if final_cltv_expiry_delta < MIN_FINAL_CLTV_EXPIRY {
            return MmError::err(SendPaymentError::CLTVExpiryError(
                final_cltv_expiry_delta,
                MIN_FINAL_CLTV_EXPIRY,
            ));
        }
        let payment_preimage = PaymentPreimage(self.keys_manager.get_secure_random_bytes());
        self.invoice_payer
            .pay_pubkey(destination, payment_preimage, amount_msat, final_cltv_expiry_delta)
            .map_to_mm(|e| SendPaymentError::PaymentError(format!("{:?}", e)))?;
        let payment_hash = PaymentHash(Sha256::hash(&payment_preimage.0).into_inner());
        let payment_type = PaymentType::OutboundPayment { destination };

        Ok(PaymentInfo {
            payment_hash,
            payment_type,
            description: "".into(),
            preimage: Some(payment_preimage),
            secret: None,
            amt_msat: Some(amount_msat),
            fee_paid_msat: None,
            status: HTLCStatus::Pending,
            created_at: now_ms() / 1000,
            last_updated: now_ms() / 1000,
        })
    }

    async fn get_open_channels_by_filter(
        &self,
        filter: Option<OpenChannelsFilter>,
        paging: PagingOptionsEnum<u64>,
        limit: usize,
    ) -> ListChannelsResult<GetOpenChannelsResult> {
        let mut total_open_channels: Vec<ChannelDetailsForRPC> = self
            .channel_manager
            .list_channels()
            .into_iter()
            .map(From::from)
            .collect();

        total_open_channels.sort_by(|a, b| a.rpc_channel_id.cmp(&b.rpc_channel_id));

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
        let decimals = self.decimals();
        let (spendable_msat, unspendable_msat) = self.get_balance_msat();
        let my_balance = CoinBalance {
            spendable: big_decimal_from_sat_unsigned(spendable_msat, decimals),
            unspendable: big_decimal_from_sat_unsigned(unspendable_msat, decimals),
        };
        Box::new(futures01::future::ok(my_balance))
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
    fn tx_enum_from_bytes(&self, _bytes: &[u8]) -> Result<TransactionEnum, String> { unimplemented!() }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> { Box::new(futures01::future::ok(0)) }

    fn display_priv_key(&self) -> Result<String, String> {
        Ok(self
            .keys_manager
            .get_node_secret(Recipient::Node)
            .map_err(|_| "Unsupported recipient".to_string())?
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
        protocol_conf.confirmations,
    ));

    // Initialize the Logger
    let logger = ctx.log.0.clone();

    // Initialize Persister
    let persister = ln_utils::init_persister(ctx, platform.clone(), conf.ticker.clone(), params.backup_path).await?;

    // Initialize the KeysManager
    let keys_manager = ln_utils::init_keys_manager(ctx)?;

    // Initialize the NetGraphMsgHandler. This is used for providing routes to send payments over
    let network_graph = Arc::new(persister.get_network_graph(protocol_conf.network.into()).await?);
    spawn(ln_utils::persist_network_graph_loop(
        persister.clone(),
        network_graph.clone(),
    ));
    let network_gossip = Arc::new(NetGraphMsgHandler::new(
        network_graph.clone(),
        None::<Arc<dyn Access + Send + Sync>>,
        logger.clone(),
    ));

    // Initialize the ChannelManager
    let (chain_monitor, channel_manager) = ln_utils::init_channel_manager(
        platform.clone(),
        logger.clone(),
        persister.clone(),
        keys_manager.clone(),
        conf.clone().into(),
    )
    .await?;

    // Initialize the PeerManager
    let peer_manager = ln_p2p::init_peer_manager(
        ctx.clone(),
        params.listening_port,
        channel_manager.clone(),
        network_gossip.clone(),
        keys_manager
            .get_node_secret(Recipient::Node)
            .map_to_mm(|_| EnableLightningError::UnsupportedMode("'start_lightning'".into(), "local node".into()))?,
        logger.clone(),
    )
    .await?;

    // Initialize the event handler
    let event_handler = Arc::new(ln_events::LightningEventHandler::new(
        // It's safe to use unwrap here for now until implementing Native Client for Lightning
        platform.clone(),
        channel_manager.clone(),
        keys_manager.clone(),
        persister.clone(),
    ));

    // Initialize routing Scorer
    let scorer = Arc::new(Mutex::new(persister.get_scorer(network_graph.clone()).await?));
    spawn(ln_utils::persist_scorer_loop(persister.clone(), scorer.clone()));

    // Create InvoicePayer
    let router = DefaultRouter::new(network_graph, logger.clone(), keys_manager.get_secure_random_bytes());
    let invoice_payer = Arc::new(InvoicePayer::new(
        channel_manager.clone(),
        router,
        scorer,
        logger.clone(),
        event_handler,
        payment::RetryAttempts(params.payment_retries.unwrap_or(5)),
    ));

    // Persist ChannelManager
    // Note: if the ChannelManager is not persisted properly to disk, there is risk of channels force closing the next time LN starts up
    let channel_manager_persister = persister.clone();
    let persist_channel_manager_callback =
        move |node: &ChannelManager| channel_manager_persister.persist_manager(&*node);

    // Start Background Processing. Runs tasks periodically in the background to keep LN node operational.
    // InvoicePayer will act as our event handler as it handles some of the payments related events before
    // delegating it to LightningEventHandler.
    let background_processor = Arc::new(BackgroundProcessor::start(
        persist_channel_manager_callback,
        invoice_payer.clone(),
        chain_monitor.clone(),
        channel_manager.clone(),
        Some(network_gossip),
        peer_manager.clone(),
        logger,
    ));

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
        background_processor,
        channel_manager,
        chain_monitor,
        keys_manager,
        invoice_payer,
        persister,
        open_channels_nodes,
    })
}

#[derive(Deserialize)]
pub struct ConnectToNodeRequest {
    pub coin: String,
    pub node_address: NodeAddress,
}

/// Connect to a certain node on the lightning network.
pub async fn connect_to_lightning_node(ctx: MmArc, req: ConnectToNodeRequest) -> ConnectToNodeResult<String> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(ConnectToNodeError::UnsupportedCoin(coin.ticker().to_string())),
    };

    let node_pubkey = req.node_address.pubkey;
    let node_addr = req.node_address.addr;
    let res = connect_to_node(node_pubkey, node_addr, ln_coin.peer_manager.clone()).await?;

    // If a node that we have an open channel with changed it's address, "connect_to_lightning_node"
    // can be used to reconnect to the new address while saving this new address for reconnections.
    if let ConnectToNodeRes::ConnectedSuccessfully { .. } = res {
        if let Entry::Occupied(mut entry) = ln_coin.open_channels_nodes.lock().entry(node_pubkey) {
            entry.insert(node_addr);
        }
        ln_coin
            .persister
            .save_nodes_addresses(ln_coin.open_channels_nodes)
            .await?;
    }

    Ok(res.to_string())
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum ChannelOpenAmount {
    Exact(BigDecimal),
    Max,
}

#[derive(Deserialize)]
pub struct OpenChannelRequest {
    pub coin: String,
    pub node_address: NodeAddress,
    pub amount: ChannelOpenAmount,
    /// The amount to push to the counterparty as part of the open, in milli-satoshi. Creates inbound liquidity for the channel.
    /// By setting push_msat to a value, opening channel request will be equivalent to opening a channel then sending a payment with
    /// the push_msat amount.
    #[serde(default)]
    pub push_msat: u64,
    pub channel_options: Option<ChannelOptions>,
    pub counterparty_locktime: Option<u16>,
    pub our_htlc_minimum_msat: Option<u64>,
}

#[derive(Serialize)]
pub struct OpenChannelResponse {
    rpc_channel_id: u64,
    node_address: NodeAddress,
}

/// Opens a channel on the lightning network.
pub async fn open_channel(ctx: MmArc, req: OpenChannelRequest) -> OpenChannelResult<OpenChannelResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(OpenChannelError::UnsupportedCoin(coin.ticker().to_string())),
    };

    // Making sure that the node data is correct and that we can connect to it before doing more operations
    let node_pubkey = req.node_address.pubkey;
    let node_addr = req.node_address.addr;
    connect_to_node(node_pubkey, node_addr, ln_coin.peer_manager.clone()).await?;

    let platform_coin = ln_coin.platform_coin().clone();
    let decimals = platform_coin.as_ref().decimals;
    let my_address = platform_coin.as_ref().derivation_method.iguana_or_err()?;
    let (unspents, _) = platform_coin.get_unspent_ordered_list(my_address).await?;
    let (value, fee_policy) = match req.amount.clone() {
        ChannelOpenAmount::Max => (
            unspents.iter().fold(0, |sum, unspent| sum + unspent.value),
            FeePolicy::DeductFromOutput(0),
        ),
        ChannelOpenAmount::Exact(v) => {
            let value = sat_from_big_decimal(&v, decimals)?;
            (value, FeePolicy::SendExact)
        },
    };

    // The actual script_pubkey will replace this before signing the transaction after receiving the required
    // output script from the other node when the channel is accepted
    let script_pubkey =
        Builder::build_witness_script(&AddressHashEnum::WitnessScriptHash(Default::default())).to_bytes();
    let outputs = vec![TransactionOutput { value, script_pubkey }];

    let mut tx_builder = UtxoTxBuilder::new(&platform_coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs)
        .with_fee_policy(fee_policy);

    let fee = platform_coin
        .get_tx_fee()
        .await
        .map_err(|e| OpenChannelError::RpcError(e.to_string()))?;
    tx_builder = tx_builder.with_fee(fee);

    let (unsigned, _) = tx_builder.build().await?;

    let amount_in_sat = unsigned.outputs[0].value;
    let push_msat = req.push_msat;
    let channel_manager = ln_coin.channel_manager.clone();

    let mut conf = ln_coin.conf.clone();
    if let Some(options) = req.channel_options {
        match conf.channel_options.as_mut() {
            Some(o) => o.update(options),
            None => conf.channel_options = Some(options),
        }
    }

    let mut user_config: UserConfig = conf.into();
    if let Some(locktime) = req.counterparty_locktime {
        user_config.own_channel_config.our_to_self_delay = locktime;
    }
    if let Some(min) = req.our_htlc_minimum_msat {
        user_config.own_channel_config.our_htlc_minimum_msat = min;
    }

    let rpc_channel_id = ln_coin.persister.get_last_channel_rpc_id().await? as u64 + 1;

    let temp_channel_id = async_blocking(move || {
        channel_manager
            .create_channel(node_pubkey, amount_in_sat, push_msat, rpc_channel_id, Some(user_config))
            .map_to_mm(|e| OpenChannelError::FailureToOpenChannel(node_pubkey.to_string(), format!("{:?}", e)))
    })
    .await?;

    {
        let mut unsigned_funding_txs = ln_coin.platform.unsigned_funding_txs.lock();
        unsigned_funding_txs.insert(rpc_channel_id, unsigned);
    }

    let pending_channel_details = SqlChannelDetails::new(
        rpc_channel_id,
        temp_channel_id,
        node_pubkey,
        true,
        user_config.channel_options.announced_channel,
    );

    // Saving node data to reconnect to it on restart
    ln_coin.open_channels_nodes.lock().insert(node_pubkey, node_addr);
    ln_coin
        .persister
        .save_nodes_addresses(ln_coin.open_channels_nodes)
        .await?;

    ln_coin.persister.add_channel_to_db(pending_channel_details).await?;

    Ok(OpenChannelResponse {
        rpc_channel_id,
        node_address: req.node_address,
    })
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
    pub confirmed: Option<bool>,
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

    let is_confirmed = filter.confirmed.is_none() || Some(&channel_details.confirmed) == filter.confirmed.as_ref();

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
    pub confirmed: bool,
    // Channel is confirmed and funding_locked messages have been exchanged, the peer is connected,
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
            confirmed: details.is_funding_locked,
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
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(ListChannelsError::UnsupportedCoin(coin.ticker().to_string())),
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
    closed_channels: Vec<SqlChannelDetails>,
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
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(ListChannelsError::UnsupportedCoin(coin.ticker().to_string())),
    };
    let closed_channels_res = ln_coin
        .persister
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
    Closed(SqlChannelDetails),
}

pub async fn get_channel_details(
    ctx: MmArc,
    req: GetChannelDetailsRequest,
) -> GetChannelDetailsResult<GetChannelDetailsResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(GetChannelDetailsError::UnsupportedCoin(coin.ticker().to_string())),
    };
    let channel_details = match ln_coin
        .channel_manager
        .list_channels()
        .into_iter()
        .find(|chan| chan.user_channel_id == req.rpc_channel_id)
    {
        Some(details) => GetChannelDetailsResponse::Open(details.into()),
        None => GetChannelDetailsResponse::Closed(
            ln_coin
                .persister
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
}

#[derive(Serialize)]
pub struct GenerateInvoiceResponse {
    payment_hash: H256Json,
    invoice: InvoiceForRPC,
}

/// Generates an invoice (request for payment) that can be paid on the lightning network by another node using send_payment.
pub async fn generate_invoice(
    ctx: MmArc,
    req: GenerateInvoiceRequest,
) -> GenerateInvoiceResult<GenerateInvoiceResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(GenerateInvoiceError::UnsupportedCoin(coin.ticker().to_string())),
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
    let invoice = create_invoice_from_channelmanager(
        &ln_coin.channel_manager,
        ln_coin.keys_manager,
        network,
        req.amount_in_msat,
        req.description.clone(),
    )?;
    let payment_hash = invoice.payment_hash().into_inner();
    let payment_info = PaymentInfo {
        payment_hash: PaymentHash(payment_hash),
        payment_type: PaymentType::InboundPayment,
        description: req.description,
        preimage: None,
        secret: Some(*invoice.payment_secret()),
        amt_msat: req.amount_in_msat,
        fee_paid_msat: None,
        status: HTLCStatus::Pending,
        created_at: now_ms() / 1000,
        last_updated: now_ms() / 1000,
    };
    ln_coin.persister.add_or_update_payment_in_db(payment_info).await?;
    Ok(GenerateInvoiceResponse {
        payment_hash: payment_hash.into(),
        invoice: invoice.into(),
    })
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Payment {
    #[serde(rename = "invoice")]
    Invoice { invoice: InvoiceForRPC },
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
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(SendPaymentError::UnsupportedCoin(coin.ticker().to_string())),
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
        Payment::Invoice { invoice } => ln_coin.pay_invoice(invoice.into())?,
        Payment::Keysend {
            destination,
            amount_in_msat,
            expiry,
        } => ln_coin.keysend(destination.into(), amount_in_msat, expiry)?,
    };
    ln_coin
        .persister
        .add_or_update_payment_in_db(payment_info.clone())
        .await?;
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

impl From<PaymentsFilterForRPC> for PaymentsFilter {
    fn from(filter: PaymentsFilterForRPC) -> Self {
        PaymentsFilter {
            payment_type: filter.payment_type.map(From::from),
            description: filter.description,
            status: filter.status,
            from_amount_msat: filter.from_amount_msat,
            to_amount_msat: filter.to_amount_msat,
            from_fee_paid_msat: filter.from_fee_paid_msat,
            to_fee_paid_msat: filter.to_fee_paid_msat,
            from_timestamp: filter.from_timestamp,
            to_timestamp: filter.to_timestamp,
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
    amount_in_msat: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fee_paid_msat: Option<u64>,
    status: HTLCStatus,
    created_at: u64,
    last_updated: u64,
}

impl From<PaymentInfo> for PaymentInfoForRPC {
    fn from(info: PaymentInfo) -> Self {
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
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(ListPaymentsError::UnsupportedCoin(coin.ticker().to_string())),
    };
    let get_payments_res = ln_coin
        .persister
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
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(GetPaymentDetailsError::UnsupportedCoin(coin.ticker().to_string())),
    };

    if let Some(payment_info) = ln_coin
        .persister
        .get_payment_from_db(PaymentHash(req.payment_hash.0))
        .await?
    {
        return Ok(GetPaymentDetailsResponse {
            payment_details: payment_info.into(),
        });
    }

    MmError::err(GetPaymentDetailsError::NoSuchPayment(req.payment_hash))
}

#[derive(Deserialize)]
pub struct CloseChannelReq {
    pub coin: String,
    pub channel_id: H256Json,
    #[serde(default)]
    pub force_close: bool,
}

pub async fn close_channel(ctx: MmArc, req: CloseChannelReq) -> CloseChannelResult<String> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(CloseChannelError::UnsupportedCoin(coin.ticker().to_string())),
    };
    if req.force_close {
        ln_coin
            .channel_manager
            .force_close_channel(&req.channel_id.0)
            .map_to_mm(|e| CloseChannelError::CloseChannelError(format!("{:?}", e)))?;
    } else {
        ln_coin
            .channel_manager
            .close_channel(&req.channel_id.0)
            .map_to_mm(|e| CloseChannelError::CloseChannelError(format!("{:?}", e)))?;
    }

    Ok(format!("Initiated closing of channel: {:?}", req.channel_id))
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
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(ClaimableBalancesError::UnsupportedCoin(coin.ticker().to_string())),
    };
    let ignored_channels = if req.include_open_channels_balances {
        Vec::new()
    } else {
        ln_coin.channel_manager.list_channels()
    };
    let claimable_balances = ln_coin
        .chain_monitor
        .get_claimable_balances(&ignored_channels.iter().collect::<Vec<_>>()[..])
        .into_iter()
        .map(From::from)
        .collect();

    Ok(claimable_balances)
}

use super::{lp_coinfind_or_err, MmCoinEnum};
use crate::utxo::rpc_clients::UtxoRpcClientEnum;
use crate::utxo::utxo_common::{big_decimal_from_sat_unsigned, UtxoTxBuilder};
use crate::utxo::{sat_from_big_decimal, BlockchainNetwork, FeePolicy, UtxoCommonOps, UtxoTxGenerationOps};
use crate::{BalanceFut, CoinBalance, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin,
            NegotiateSwapContractAddrErr, SwapOps, TradeFee, TradePreimageFut, TradePreimageValue, TransactionEnum,
            TransactionFut, UtxoStandardCoin, ValidateAddressResult, WithdrawError, WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use bigdecimal::BigDecimal;
use bitcoin::blockdata::script::Script;
use bitcoin::hash_types::Txid;
use bitcoin::hashes::Hash;
use bitcoin_hashes::sha256::Hash as Sha256;
use chain::TransactionOutput;
use common::ip_addr::myipaddr;
use common::log::LogOnError;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::mm_number::MmNumber;
use common::{async_blocking, log};
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::AddressHashEnum;
use lightning::chain::channelmonitor::Balance;
use lightning::chain::keysinterface::KeysInterface;
use lightning::chain::keysinterface::KeysManager;
use lightning::chain::WatchedOutput;
use lightning::ln::channelmanager::{ChannelDetails, MIN_FINAL_CLTV_EXPIRY};
use lightning::ln::{PaymentHash, PaymentPreimage, PaymentSecret};
use lightning::util::config::UserConfig;
use lightning_background_processor::BackgroundProcessor;
use lightning_invoice::utils::create_invoice_from_channelmanager;
use lightning_invoice::Invoice;
use lightning_persister::storage::{NodesAddressesMapShared, Storage};
use lightning_persister::FilesystemPersister;
use ln_conf::{ChannelOptions, LightningCoinConf, PlatformCoinConfirmations};
use ln_connections::{connect_to_node, ConnectToNodeRes};
use ln_errors::{ClaimableBalancesError, ClaimableBalancesResult, CloseChannelError, CloseChannelResult,
                ConnectToNodeError, ConnectToNodeResult, EnableLightningError, EnableLightningResult,
                GenerateInvoiceError, GenerateInvoiceResult, GetChannelDetailsError, GetChannelDetailsResult,
                GetPaymentDetailsError, GetPaymentDetailsResult, ListChannelsError, ListChannelsResult,
                ListPaymentsError, ListPaymentsResult, OpenChannelError, OpenChannelResult, SendPaymentError,
                SendPaymentResult};
use ln_events::LightningEventHandler;
use ln_serialization::{InvoiceForRPC, NodeAddress, PublicKeyForRPC};
use ln_utils::{ChainMonitor, ChannelManager, InvoicePayer, PeerManager};
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
use std::sync::Arc;

pub mod ln_conf;
mod ln_connections;
pub mod ln_errors;
mod ln_events;
mod ln_rpc;
mod ln_serialization;
pub mod ln_utils;

type PaymentsMap = HashMap<PaymentHash, PaymentInfo>;
type PaymentsMapShared = Arc<PaMutex<PaymentsMap>>;

pub struct PlatformFields {
    pub platform_coin: UtxoStandardCoin,
    /// Main/testnet/signet/regtest Needed for lightning node to know which network to connect to
    pub network: BlockchainNetwork,
    // Default fees to and confirmation targets to be used for FeeEstimator. Default fees are used when the call for
    // estimate_fee_sat fails.
    pub default_fees_and_confirmations: PlatformCoinConfirmations,
    // This cache stores the transactions that the LN node has interest in.
    pub registered_txs: PaMutex<HashMap<Txid, HashSet<Script>>>,
    // This cache stores the outputs that the LN node has interest in.
    pub registered_outputs: PaMutex<Vec<WatchedOutput>>,
    // This cache stores transactions to be broadcasted once the other node accepts the channel
    pub unsigned_funding_txs: PaMutex<HashMap<[u8; 32], TransactionInputSigner>>,
}

impl PlatformFields {
    pub fn add_tx(&self, txid: &Txid, script_pubkey: &Script) {
        let mut registered_txs = self.registered_txs.lock();
        match registered_txs.get_mut(txid) {
            Some(h) => {
                h.insert(script_pubkey.clone());
            },
            None => {
                let mut script_pubkeys = HashSet::new();
                script_pubkeys.insert(script_pubkey.clone());
                registered_txs.insert(*txid, script_pubkeys);
            },
        }
    }

    pub fn add_output(&self, output: WatchedOutput) {
        let mut registered_outputs = self.registered_outputs.lock();
        registered_outputs.push(output);
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HTLCStatus {
    Pending,
    Succeeded,
    Failed,
}

#[derive(Clone)]
pub struct PaymentInfo {
    pub preimage: Option<PaymentPreimage>,
    pub secret: Option<PaymentSecret>,
    pub status: HTLCStatus,
    pub amt_msat: Option<u64>,
    pub fee_paid_msat: Option<u64>,
}

#[derive(Clone)]
pub struct LightningCoin {
    pub platform_fields: Arc<PlatformFields>,
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
    pub persister: Arc<FilesystemPersister>,
    /// The mutex storing the inbound payments info.
    pub inbound_payments: PaymentsMapShared,
    /// The mutex storing the outbound payments info.
    pub outbound_payments: PaymentsMapShared,
    /// The mutex storing the addresses of the nodes that are used for reconnecting.
    pub nodes_addresses: NodesAddressesMapShared,
}

impl fmt::Debug for LightningCoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "LightningCoin {{ conf: {:?} }}", self.conf) }
}

impl LightningCoin {
    fn platform_coin(&self) -> &UtxoStandardCoin { &self.platform_fields.platform_coin }

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

    fn pay_invoice(&self, invoice: Invoice) -> SendPaymentResult<(PaymentHash, PaymentInfo)> {
        self.invoice_payer
            .pay_invoice(&invoice)
            .map_to_mm(|e| SendPaymentError::PaymentError(format!("{:?}", e)))?;
        let payment_hash = PaymentHash((*invoice.payment_hash()).into_inner());
        let payment_secret = Some(*invoice.payment_secret());
        Ok((payment_hash, PaymentInfo {
            preimage: None,
            secret: payment_secret,
            status: HTLCStatus::Pending,
            amt_msat: invoice.amount_milli_satoshis(),
            fee_paid_msat: None,
        }))
    }

    fn keysend(
        &self,
        destination: PublicKey,
        amount_msat: u64,
        final_cltv_expiry_delta: u32,
    ) -> SendPaymentResult<(PaymentHash, PaymentInfo)> {
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

        Ok((payment_hash, PaymentInfo {
            preimage: Some(payment_preimage),
            secret: None,
            status: HTLCStatus::Pending,
            amt_msat: Some(amount_msat),
            fee_paid_msat: None,
        }))
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
        _payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn validate_taker_payment(
        &self,
        _payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
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
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        unimplemented!()
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        _time_lock: u32,
        _other_pub: &[u8],
        _secret_hash: &[u8],
        _tx: &[u8],
        _search_from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        _time_lock: u32,
        _other_pub: &[u8],
        _secret_hash: &[u8],
        _tx: &[u8],
        _search_from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
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
}

impl MarketCoinOps for LightningCoin {
    fn ticker(&self) -> &str { &self.conf.ticker }

    fn my_address(&self) -> Result<String, String> { Ok(self.my_node_id()) }

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

    fn send_raw_tx(&self, _tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
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

    fn display_priv_key(&self) -> Result<String, String> { Ok(self.keys_manager.get_node_secret().to_string()) }

    // Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
    fn min_tx_amount(&self) -> BigDecimal { unimplemented!() }

    // Todo: Implement this when implementing swaps for lightning as it's is used only for order matching/swaps
    fn min_trading_vol(&self) -> MmNumber { unimplemented!() }
}

impl MmCoin for LightningCoin {
    fn is_asset_chain(&self) -> bool { false }

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
    fn get_sender_trade_fee(&self, _value: TradePreimageValue, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        unimplemented!()
    }

    // Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { unimplemented!() }

    // Todo: Implement this when implementing swaps for lightning as it's is used only for swaps
    fn get_fee_to_send_taker_fee(
        &self,
        _dex_fee_amount: BigDecimal,
        _stage: FeeApproxStage,
    ) -> TradePreimageFut<TradeFee> {
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
    if let ConnectToNodeRes::ConnectedSuccessfully(_, _) = res {
        if let Entry::Occupied(mut entry) = ln_coin.nodes_addresses.lock().entry(node_pubkey) {
            entry.insert(node_addr);
        }
        ln_coin.persister.save_nodes_addresses(ln_coin.nodes_addresses).await?;
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
    temporary_channel_id: H256Json,
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
    let (unspents, _) = platform_coin.list_unspent_ordered(my_address).await?;
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

    let temp_channel_id = async_blocking(move || {
        channel_manager
            .create_channel(node_pubkey, amount_in_sat, push_msat, 1, Some(user_config))
            .map_to_mm(|e| OpenChannelError::FailureToOpenChannel(node_pubkey.to_string(), format!("{:?}", e)))
    })
    .await?;

    {
        let mut unsigned_funding_txs = ln_coin.platform_fields.unsigned_funding_txs.lock();
        unsigned_funding_txs.insert(temp_channel_id, unsigned);
    }

    // Saving node data to reconnect to it on restart
    ln_coin.nodes_addresses.lock().insert(node_pubkey, node_addr);
    ln_coin.persister.save_nodes_addresses(ln_coin.nodes_addresses).await?;

    Ok(OpenChannelResponse {
        temporary_channel_id: temp_channel_id.into(),
        node_address: req.node_address,
    })
}

#[derive(Deserialize)]
pub struct ListChannelsRequest {
    pub coin: String,
}

#[derive(Serialize)]
pub struct ChannelDetailsForRPC {
    pub channel_id: String,
    pub counterparty_node_id: String,
    pub funding_tx: Option<String>,
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
            channel_id: hex::encode(details.channel_id),
            counterparty_node_id: details.counterparty.node_id.to_string(),
            funding_tx: details.funding_txo.map(|tx| tx.txid.to_string()),
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

#[derive(Serialize)]
pub struct ListChannelsResponse {
    channels: Vec<ChannelDetailsForRPC>,
}

pub async fn list_channels(ctx: MmArc, req: ListChannelsRequest) -> ListChannelsResult<ListChannelsResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(ListChannelsError::UnsupportedCoin(coin.ticker().to_string())),
    };
    let channels = ln_coin
        .channel_manager
        .list_channels()
        .into_iter()
        .map(From::from)
        .collect();

    Ok(ListChannelsResponse { channels })
}

#[derive(Deserialize)]
pub struct GetChannelDetailsRequest {
    pub coin: String,
    pub channel_id: H256Json,
}

#[derive(Serialize)]
pub struct GetChannelDetailsResponse {
    channel_details: ChannelDetailsForRPC,
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
    let channel_details = ln_coin
        .channel_manager
        .list_channels()
        .into_iter()
        .find(|chan| chan.channel_id == req.channel_id.0)
        .ok_or(GetChannelDetailsError::NoSuchChannel(req.channel_id))?
        .into();

    Ok(GetChannelDetailsResponse { channel_details })
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
    let nodes_addresses = ln_coin.nodes_addresses.lock().clone();
    for (node_pubkey, node_addr) in nodes_addresses {
        connect_to_node(node_pubkey, node_addr, ln_coin.peer_manager.clone())
            .await
            .error_log_with_msg(&format!(
                "Channel with node: {} can't be used for invoice routing hints due to connection error.",
                node_pubkey
            ));
    }
    let network = ln_coin.platform_fields.network.clone().into();
    let invoice = create_invoice_from_channelmanager(
        &ln_coin.channel_manager,
        ln_coin.keys_manager,
        network,
        req.amount_in_msat,
        req.description,
    )?;
    Ok(GenerateInvoiceResponse {
        payment_hash: invoice.payment_hash().into_inner().into(),
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
    let nodes_addresses = ln_coin.nodes_addresses.lock().clone();
    for (node_pubkey, node_addr) in nodes_addresses {
        connect_to_node(node_pubkey, node_addr, ln_coin.peer_manager.clone())
            .await
            .error_log_with_msg(&format!(
                "Channel with node: {} can't be used to route this payment due to connection error.",
                node_pubkey
            ));
    }
    let (payment_hash, payment_info) = match req.payment {
        Payment::Invoice { invoice } => ln_coin.pay_invoice(invoice.into())?,
        Payment::Keysend {
            destination,
            amount_in_msat,
            expiry,
        } => ln_coin.keysend(destination.into(), amount_in_msat, expiry)?,
    };
    let mut outbound_payments = ln_coin.outbound_payments.lock();
    outbound_payments.insert(payment_hash, payment_info);
    Ok(SendPaymentResponse {
        payment_hash: payment_hash.0.into(),
    })
}

#[derive(Deserialize)]
pub struct ListPaymentsReq {
    pub coin: String,
}

#[derive(Serialize)]
pub struct PaymentInfoForRPC {
    status: HTLCStatus,
    amount_in_msat: Option<u64>,
    fee_paid_msat: Option<u64>,
}

impl From<PaymentInfo> for PaymentInfoForRPC {
    fn from(info: PaymentInfo) -> Self {
        PaymentInfoForRPC {
            status: info.status,
            amount_in_msat: info.amt_msat,
            fee_paid_msat: info.fee_paid_msat,
        }
    }
}

#[derive(Serialize)]
pub struct ListPaymentsResponse {
    pub inbound_payments: HashMap<H256Json, PaymentInfoForRPC>,
    pub outbound_payments: HashMap<H256Json, PaymentInfoForRPC>,
}

pub async fn list_payments(ctx: MmArc, req: ListPaymentsReq) -> ListPaymentsResult<ListPaymentsResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(ListPaymentsError::UnsupportedCoin(coin.ticker().to_string())),
    };
    let inbound_payments = ln_coin
        .inbound_payments
        .lock()
        .clone()
        .into_iter()
        .map(|(hash, info)| (hash.0.into(), info.into()))
        .collect();
    let outbound_payments = ln_coin
        .outbound_payments
        .lock()
        .clone()
        .into_iter()
        .map(|(hash, info)| (hash.0.into(), info.into()))
        .collect();

    Ok(ListPaymentsResponse {
        inbound_payments,
        outbound_payments,
    })
}

#[derive(Deserialize)]
pub struct GetPaymentDetailsRequest {
    pub coin: String,
    pub payment_hash: H256Json,
}

#[derive(Serialize)]
enum PaymentType {
    #[serde(rename = "Outbound Payment")]
    OutboundPayment,
    #[serde(rename = "Inbound Payment")]
    InboundPayment,
}

#[derive(Serialize)]
pub struct GetPaymentDetailsResponse {
    payment_type: PaymentType,
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

    if let Some(payment_info) = ln_coin.outbound_payments.lock().get(&PaymentHash(req.payment_hash.0)) {
        return Ok(GetPaymentDetailsResponse {
            payment_type: PaymentType::OutboundPayment,
            payment_details: payment_info.clone().into(),
        });
    }

    if let Some(payment_info) = ln_coin.inbound_payments.lock().get(&PaymentHash(req.payment_hash.0)) {
        return Ok(GetPaymentDetailsResponse {
            payment_type: PaymentType::InboundPayment,
            payment_details: payment_info.clone().into(),
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

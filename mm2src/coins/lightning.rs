#[cfg(not(target_arch = "wasm32"))]
use super::{lp_coinfind_or_err, MmCoinEnum};
#[cfg(not(target_arch = "wasm32"))]
use crate::utxo::rpc_clients::UtxoRpcClientEnum;
#[cfg(not(target_arch = "wasm32"))]
use crate::utxo::utxo_common::UtxoTxBuilder;
use crate::utxo::BlockchainNetwork;
#[cfg(not(target_arch = "wasm32"))]
use crate::utxo::{sat_from_big_decimal, FeePolicy, UtxoCommonOps, UtxoTxGenerationOps};
use crate::{BalanceFut, CoinBalance, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin,
            NegotiateSwapContractAddrErr, SwapOps, TradeFee, TradePreimageFut, TradePreimageValue, TransactionEnum,
            TransactionFut, UtxoStandardCoin, ValidateAddressResult, WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use bigdecimal::BigDecimal;
use bitcoin::blockdata::script::Script;
use bitcoin::hash_types::Txid;
#[cfg(not(target_arch = "wasm32"))] use chain::TransactionOutput;
#[cfg(not(target_arch = "wasm32"))]
use common::ip_addr::myipaddr;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::mm_number::MmNumber;
use futures::lock::Mutex as AsyncMutex;
use futures01::Future;
#[cfg(not(target_arch = "wasm32"))] use keys::AddressHashEnum;
use lightning::chain::WatchedOutput;
#[cfg(not(target_arch = "wasm32"))]
use lightning_background_processor::BackgroundProcessor;
use ln_errors::{ConnectToNodeError, ConnectToNodeResult, EnableLightningError, EnableLightningResult,
                OpenChannelError, OpenChannelResult};
#[cfg(not(target_arch = "wasm32"))]
use ln_utils::{connect_to_node, last_request_id_path, nodes_data_path, open_ln_channel, parse_node_info,
               read_last_request_id_from_file, read_nodes_data_from_file, save_last_request_id_to_file,
               save_node_data_to_file, ChannelManager, PeerManager};
use rpc::v1::types::Bytes as BytesJson;
#[cfg(not(target_arch = "wasm32"))] use script::Builder;
use script::TransactionInputSigner;
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

pub mod ln_errors;
mod ln_rpc;
pub mod ln_utils;

#[derive(Debug)]
pub struct LightningProtocolConf {
    pub platform_coin_ticker: String,
    pub network: BlockchainNetwork,
}

#[derive(Debug)]
pub struct PlatformFields {
    pub platform_coin: UtxoStandardCoin,
    // This cache stores the transactions that the LN node has interest in.
    pub registered_txs: AsyncMutex<HashMap<Txid, HashSet<Script>>>,
    // This cache stores the outputs that the LN node has interest in.
    pub registered_outputs: AsyncMutex<Vec<WatchedOutput>>,
    // This cache stores transactions to be broadcasted once the other node accepts the channel
    pub unsigned_funding_txs: AsyncMutex<HashMap<u64, TransactionInputSigner>>,
}

impl PlatformFields {
    pub async fn add_tx(&self, txid: &Txid, script_pubkey: &Script) {
        let mut registered_txs = self.registered_txs.lock().await;
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

    pub async fn add_output(&self, output: WatchedOutput) {
        let mut registered_outputs = self.registered_outputs.lock().await;
        registered_outputs.push(output);
    }
}

#[derive(Debug)]
pub struct LightningCoinConf {
    ticker: String,
}

#[derive(Clone)]
pub struct LightningCoin {
    pub platform_fields: Arc<PlatformFields>,
    pub conf: Arc<LightningCoinConf>,
    /// The lightning node peer manager that takes care of connecting to peers, etc..
    #[cfg(not(target_arch = "wasm32"))]
    pub peer_manager: Arc<PeerManager>,
    /// The lightning node background processor that takes care of tasks that need to happen periodically
    #[cfg(not(target_arch = "wasm32"))]
    pub background_processor: Arc<BackgroundProcessor>,
    /// The lightning node channel manager which keeps track of the number of open channels and sends messages to the appropriate
    /// channel, also tracks HTLC preimages and forwards onion packets appropriately.
    #[cfg(not(target_arch = "wasm32"))]
    pub channel_manager: Arc<ChannelManager>,
}

impl fmt::Debug for LightningCoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LightningCoin {{ platform_fields: {:?}, conf: {:?} }}",
            self.platform_fields, self.conf
        )
    }
}

impl LightningCoin {
    fn platform_coin(&self) -> &UtxoStandardCoin { &self.platform_fields.platform_coin }
}

#[async_trait]
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

    // Returns platform_coin address for now
    fn my_address(&self) -> Result<String, String> { self.platform_coin().my_address() }

    // Returns platform_coin balance for now
    fn my_balance(&self) -> BalanceFut<CoinBalance> { self.platform_coin().my_balance() }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { unimplemented!() }

    fn send_raw_tx(&self, _tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> { unimplemented!() }

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

    fn wait_for_tx_spend(
        &self,
        _transaction: &[u8],
        _wait_until: u64,
        _from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn tx_enum_from_bytes(&self, _bytes: &[u8]) -> Result<TransactionEnum, String> { unimplemented!() }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        self.platform_coin().current_block()
    }

    fn display_priv_key(&self) -> Result<String, String> { unimplemented!() }

    fn min_tx_amount(&self) -> BigDecimal { unimplemented!() }

    fn min_trading_vol(&self) -> MmNumber { unimplemented!() }
}

impl MmCoin for LightningCoin {
    fn is_asset_chain(&self) -> bool { unimplemented!() }

    fn withdraw(&self, _req: WithdrawRequest) -> WithdrawFut { unimplemented!() }

    fn decimals(&self) -> u8 { unimplemented!() }

    fn convert_to_address(&self, _from: &str, _to_address_format: Json) -> Result<String, String> { unimplemented!() }

    fn validate_address(&self, _address: &str) -> ValidateAddressResult { unimplemented!() }

    fn process_history_loop(&self, _ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { unimplemented!() }

    fn history_sync_status(&self) -> HistorySyncState { unimplemented!() }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { unimplemented!() }

    fn get_sender_trade_fee(&self, _value: TradePreimageValue, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        unimplemented!()
    }

    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { unimplemented!() }

    fn get_fee_to_send_taker_fee(
        &self,
        _dex_fee_amount: BigDecimal,
        _stage: FeeApproxStage,
    ) -> TradePreimageFut<TradeFee> {
        unimplemented!()
    }

    fn required_confirmations(&self) -> u64 { self.platform_coin().required_confirmations() }

    fn requires_notarization(&self) -> bool { self.platform_coin().requires_notarization() }

    fn set_required_confirmations(&self, _confirmations: u64) { unimplemented!() }

    fn set_requires_notarization(&self, _requires_nota: bool) { unimplemented!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { unimplemented!() }

    fn mature_confirmations(&self) -> Option<u32> { self.platform_coin().mature_confirmations() }

    fn coin_protocol_info(&self) -> Vec<u8> { unimplemented!() }

    fn is_coin_protocol_supported(&self, _info: &Option<Vec<u8>>) -> bool { unimplemented!() }
}

#[derive(Deserialize)]
pub struct ConnectToNodeRequest {
    pub coin: String,
    pub node_id: String,
}

#[cfg(target_arch = "wasm32")]
pub async fn connect_to_lightning_node(_ctx: MmArc, _req: ConnectToNodeRequest) -> ConnectToNodeResult<String> {
    MmError::err(ConnectToNodeError::UnsupportedMode(
        "'connect_to_lightning_node'".into(),
        "native".into(),
    ))
}

/// Connect to a certain node on the lightning network.
#[cfg(not(target_arch = "wasm32"))]
pub async fn connect_to_lightning_node(ctx: MmArc, req: ConnectToNodeRequest) -> ConnectToNodeResult<String> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(ConnectToNodeError::UnsupportedCoin(coin.ticker().to_string())),
    };

    let (node_pubkey, node_addr) = parse_node_info(req.node_id.clone())?;
    let res = connect_to_node(node_pubkey, node_addr, ln_coin.peer_manager.clone()).await?;

    Ok(res.to_string())
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum ChannelOpenAmount {
    Exact(BigDecimal),
    Max,
}

fn get_true() -> bool { true }

#[allow(dead_code)]
#[derive(Debug, Deserialize, PartialEq)]
pub struct OpenChannelRequest {
    pub coin: String,
    pub node_id: String,
    pub amount: ChannelOpenAmount,
    #[serde(default = "get_true")]
    pub announce_channel: bool,
}

#[derive(Serialize)]
pub struct OpenChannelResponse {
    temporary_channel_id: [u8; 32],
    node_id: String,
    request_id: u64,
}

#[cfg(target_arch = "wasm32")]
pub async fn open_channel(_ctx: MmArc, _req: OpenChannelRequest) -> OpenChannelResult<OpenChannelResponse> {
    MmError::err(OpenChannelError::UnsupportedMode(
        "'open_channel'".into(),
        "native".into(),
    ))
}

/// Opens a channel on the lightning network.
#[cfg(not(target_arch = "wasm32"))]
pub async fn open_channel(ctx: MmArc, req: OpenChannelRequest) -> OpenChannelResult<OpenChannelResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let ln_coin = match coin {
        MmCoinEnum::LightningCoin(c) => c,
        _ => return MmError::err(OpenChannelError::UnsupportedCoin(coin.ticker().to_string())),
    };

    // Making sure that the node data is correct and that we can connect to it before doing more operations
    let (node_pubkey, node_addr) = parse_node_info(req.node_id.clone())?;
    connect_to_node(node_pubkey, node_addr, ln_coin.peer_manager.clone()).await?;

    let platform_coin = ln_coin.platform_coin().clone();
    let decimals = platform_coin.as_ref().decimals;
    let my_address = platform_coin.as_ref().derivation_method.iguana_or_err()?;
    let (unspents, _) = platform_coin.ordered_mature_unspents(my_address).await?;
    let (value, fee_policy) = match req.amount {
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

    // Saving node data to reconnect to it on restart
    let ticker = ln_coin.ticker();
    let nodes_data = read_nodes_data_from_file(&nodes_data_path(&ctx, ticker))?;
    if !nodes_data.contains_key(&node_pubkey) {
        save_node_data_to_file(&nodes_data_path(&ctx, ticker), &req.node_id)?;
    }

    // Helps in tracking which FundingGenerationReady events corresponds to which open_channel call
    let request_id = match read_last_request_id_from_file(&last_request_id_path(&ctx, ticker)) {
        Ok(id) => id + 1,
        Err(e) => match e.get_inner() {
            OpenChannelError::InvalidPath(_) => 1,
            _ => return Err(e),
        },
    };
    save_last_request_id_to_file(&last_request_id_path(&ctx, ticker), request_id)?;

    let temporary_channel_id = open_ln_channel(
        node_pubkey,
        unsigned.outputs[0].value,
        request_id,
        req.announce_channel,
        ln_coin.channel_manager.clone(),
    )?;

    let mut unsigned_funding_txs = ln_coin.platform_fields.unsigned_funding_txs.lock().await;
    unsigned_funding_txs.insert(request_id, unsigned);

    Ok(OpenChannelResponse {
        temporary_channel_id,
        node_id: req.node_id,
        request_id,
    })
}

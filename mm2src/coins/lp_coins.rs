/******************************************************************************
 * Copyright © 2014-2018 The SuperNET Developers.                             *
 *                                                                            *
 * See the AUTHORS, DEVELOPER-AGREEMENT and LICENSE files at                  *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * SuperNET software, including this file may be copied, modified, propagated *
 * or distributed except according to the terms contained in the LICENSE file *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  coins.rs
//  marketmaker
//

#![feature(integer_atomics)]
#![feature(non_ascii_idents)]
#![feature(async_closure)]
#![feature(hash_raw_entry)]

#![allow(uncommon_codepoints)]

#[macro_use] extern crate common;
#[macro_use] extern crate fomat_macros;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate unwrap;

use bigdecimal::BigDecimal;
use common::{rpc_response, rpc_err_response, HyRes};
use common::duplex_mutex::DuplexMutex;
use common::mm_ctx::{from_ctx, MmArc};
use common::mm_metrics::{MetricsWeak};
use common::mm_number::MmNumber;
use futures01::Future;
use futures::compat::Future01CompatExt;
use gstuff::{slurp};
use http::Response;
use rpc::v1::types::{Bytes as BytesJson};
use serde_json::{self as json, Value as Json};
use std::borrow::Cow;
use std::collections::hash_map::{HashMap, RawEntryMut};
use std::fmt;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

// using custom copy of try_fus as futures crate was renamed to futures01
macro_rules! try_fus {
  ($e: expr) => {match $e {
    Ok (ok) => ok,
    Err (err) => {return Box::new (futures01::future::err (ERRL! ("{}", err)))}}}}

#[doc(hidden)]
pub mod coins_tests;
pub mod eth;
use self::eth::{eth_coin_from_conf_and_request, EthCoin, EthTxFeeDetails, SignedEthTx};
pub mod utxo;
use self::utxo::{utxo_coin_from_conf_and_request, UtxoCoin, UtxoFeeDetails, UtxoTx};
#[doc(hidden)]
#[allow(unused_variables)]
pub mod test_coin;
pub use self::test_coin::TestCoin;

pub trait Transaction: fmt::Debug + 'static {
    /// Raw transaction bytes of the transaction
    fn tx_hex(&self) -> Vec<u8>;
    fn extract_secret(&self) -> Result<Vec<u8>, String>;
    /// Serializable representation of tx hash for displaying purpose
    fn tx_hash(&self) -> BytesJson;
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransactionEnum {
    UtxoTx (UtxoTx),
    SignedEthTx (SignedEthTx)
}
ifrom! (TransactionEnum, UtxoTx);
ifrom! (TransactionEnum, SignedEthTx);

// NB: When stable and groked by IDEs, `enum_dispatch` can be used instead of `Deref` to speed things up.
impl Deref for TransactionEnum {
    type Target = dyn Transaction;
    fn deref (&self) -> &dyn Transaction {
        match self {
            &TransactionEnum::UtxoTx (ref t) => t,
            &TransactionEnum::SignedEthTx (ref t) => t,
}   }   }

pub type TransactionFut = Box<dyn Future<Item=TransactionEnum, Error=String> + Send>;

#[derive(Debug, PartialEq)]
pub enum FoundSwapTxSpend {
    Spent(TransactionEnum),
    Refunded(TransactionEnum),
}

/// Swap operations (mostly based on the Hash/Time locked transactions implemented by coin wallets).
pub trait SwapOps {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal) -> TransactionFut;

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
    ) -> TransactionFut;

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
    ) -> TransactionFut;

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut;

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut;

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
    ) -> TransactionFut;

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
    ) -> TransactionFut;

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        fee_addr: &[u8],
        amount: &BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send>;

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send>;

    fn validate_taker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send>;

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
    ) -> Box<dyn Future<Item=Option<TransactionEnum>, Error=String> + Send>;

    fn search_for_swap_tx_spend_my(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String>;

    fn search_for_swap_tx_spend_other(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String>;
}

/// Operations that coins have independently from the MarketMaker.
/// That is, things implemented by the coin wallets or public coin services.
pub trait MarketCoinOps {
    fn ticker (&self) -> &str;

    fn my_address(&self) -> Cow<str>;

    fn my_balance(&self) -> Box<dyn Future<Item=BigDecimal, Error=String> + Send>;

    /// Receives raw transaction bytes in hexadecimal format as input and returns tx hash in hexadecimal format
    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item=String, Error=String> + Send>;

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item=(), Error=String> + Send>;

    fn wait_for_tx_spend(&self, transaction: &[u8], wait_until: u64, from_block: u64) -> TransactionFut;

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String>;

    fn current_block(&self) -> Box<dyn Future<Item=u64, Error=String> + Send>;

    fn address_from_pubkey_str(&self, pubkey: &str) -> Result<String, String>;

    fn display_priv_key(&self) -> String;
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum WithdrawFee {
    UtxoFixed { amount: BigDecimal },
    UtxoPerKbyte { amount: BigDecimal },
    EthGas {
        // in gwei
        gas_price: BigDecimal,
        gas: u64,
    },
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct WithdrawRequest {
    coin: String,
    to: String,
    #[serde(default)]
    amount: BigDecimal,
    #[serde(default)]
    max: bool,
    fee: Option<WithdrawFee>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TxFeeDetails {
    Utxo(UtxoFeeDetails),
    Eth(EthTxFeeDetails),
}

impl Into<TxFeeDetails> for EthTxFeeDetails {
    fn into(self: EthTxFeeDetails) -> TxFeeDetails {
        TxFeeDetails::Eth(self)
    }
}

impl Into<TxFeeDetails> for UtxoFeeDetails {
    fn into(self: UtxoFeeDetails) -> TxFeeDetails {
        TxFeeDetails::Utxo(self)
    }
}

/// Transaction details
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TransactionDetails {
    /// Raw bytes of signed transaction in hexadecimal string, this should be sent as is to send_raw_transaction RPC to broadcast the transaction
    pub tx_hex: BytesJson,
    /// Transaction hash in hexadecimal format
    tx_hash: BytesJson,
    /// Coins are sent from these addresses
    from: Vec<String>,
    /// Coins are sent to these addresses
    to: Vec<String>,
    /// Total tx amount
    total_amount: BigDecimal,
    /// The amount spent from "my" address
    spent_by_me: BigDecimal,
    /// The amount received by "my" address
    received_by_me: BigDecimal,
    /// Resulting "my" balance change
    my_balance_change: BigDecimal,
    /// Block height
    block_height: u64,
    /// Transaction timestamp
    timestamp: u64,
    /// Every coin can has specific fee details:
    /// In UTXO tx fee is paid with the coin itself (e.g. 1 BTC and 0.0001 BTC fee).
    /// But for ERC20 token transfer fee is paid with another coin: ETH, because it's ETH smart contract function call that requires gas to be burnt.
    fee_details: Option<TxFeeDetails>,
    /// The coin transaction belongs to
    coin: String,
    /// Internal MM2 id used for internal transaction identification, for some coins it might be equal to transaction hash
    internal_id: BytesJson,
}

impl TransactionDetails {
    /// Whether the transaction details block height should be updated (when tx is confirmed)
    pub fn should_update_block_height(&self) -> bool {
        // checking for std::u64::MAX because there was integer overflow
        // in case of electrum returned -1 so there could be records with MAX confirmations
        self.block_height == 0 || self.block_height == std::u64::MAX
    }

    /// Whether the transaction timestamp should be updated (when tx is confirmed)
    pub fn should_update_timestamp(&self) -> bool {
        // checking for std::u64::MAX because there was integer overflow
        // in case of electrum returned -1 so there could be records with MAX confirmations
        self.timestamp == 0
    }
}

pub enum TradeInfo {
    // going to act as maker
    Maker,
    // going to act as taker with expected dexfee amount
    Taker(BigDecimal),
}

#[derive(Debug, Serialize)]
pub struct TradeFee {
    pub coin: String,
    pub amount: BigDecimal,
}

/// NB: Implementations are expected to follow the pImpl idiom, providing cheap reference-counted cloning and garbage collection.
pub trait MmCoin: SwapOps + MarketCoinOps + fmt::Debug + Send + Sync + 'static {
    // `MmCoin` is an extension fulcrum for something that doesn't fit the `MarketCoinOps`. Practical examples:
    // name (might be required for some APIs, CoinMarketCap for instance);
    // coin statistics that we might want to share with UI;
    // state serialization, to get full rewind and debugging information about the coins participating in a SWAP operation.
    // status/availability check: https://github.com/artemii235/SuperNET/issues/156#issuecomment-446501816

    fn is_asset_chain(&self) -> bool;

    fn check_i_have_enough_to_trade(&self, amount: &MmNumber, balance: &MmNumber, trade_info: TradeInfo) -> Box<dyn Future<Item=(), Error=String> + Send>;

    fn can_i_spend_other_payment(&self) -> Box<dyn Future<Item=(), Error=String> + Send>;

    fn withdraw(&self, req: WithdrawRequest) -> Box<dyn Future<Item=TransactionDetails, Error=String> + Send>;

    /// Maximum number of digits after decimal point used to denominate integer coin units (satoshis, wei, etc.)
    fn decimals(&self) -> u8;

    /// Loop collecting coin transaction history and saving it to local DB
    fn process_history_loop(&self, ctx: MmArc);

    /// Path to tx history file
    fn tx_history_path(&self, ctx: &MmArc) -> PathBuf {
        ctx.dbdir().join("TRANSACTIONS").join(format!("{}_{}.json", self.ticker(), self.my_address()))
    }

    /// Loads existing tx history from file, returns empty vector if file is not found
    /// Cleans the existing file if deserialization fails
    fn load_history_from_file(&self, ctx: &MmArc) -> Vec<TransactionDetails> {
        let content = slurp(&self.tx_history_path(&ctx));
        let history: Vec<TransactionDetails> = if content.is_empty() {
            vec![]
        } else {
            match json::from_slice(&content) {
                Ok(c) => c,
                Err(e) => {
                    ctx.log.log("🌋", &[&"tx_history", &self.ticker().to_string()],
                        &ERRL!("Error {} on history deserialization, resetting the cache.", e));
                    unwrap!(std::fs::remove_file(&self.tx_history_path(&ctx)));
                    vec![]
                }
            }
        };
        history
    }

    fn save_history_to_file(&self, content: &[u8], ctx: &MmArc) {
        let tmp_file = format!("{}.tmp", self.tx_history_path(&ctx).display());
        unwrap!(std::fs::write(&tmp_file, content));
        unwrap!(std::fs::rename(tmp_file, self.tx_history_path(&ctx)));
    }

    /// Gets tx details by hash requesting the coin RPC if required
    fn tx_details_by_hash(&self, hash: &[u8]) -> Box<dyn Future<Item=TransactionDetails, Error=String> + Send>;

    /// Transaction history background sync status
    fn history_sync_status(&self) -> HistorySyncState;

    /// Get fee to be paid per 1 swap transaction
    fn get_trade_fee(&self) -> Box<dyn Future<Item=TradeFee, Error=String> + Send>;

    /// required transaction confirmations number to ensure double-spend safety
    fn required_confirmations(&self) -> u64;

    /// whether coin requires notarization to ensure double-spend safety
    fn requires_notarization(&self) -> bool;

    /// set required transaction confirmations number
    fn set_required_confirmations(&self, confirmations: u64);

    /// set requires notarization
    fn set_requires_notarization(&self, requires_nota: bool);
}

#[derive(Clone, Debug)]
pub enum MmCoinEnum {
    UtxoCoin (UtxoCoin),
    EthCoin (EthCoin),
    Test (TestCoin)
}

impl From<UtxoCoin> for MmCoinEnum {
    fn from (c: UtxoCoin) -> MmCoinEnum {
        MmCoinEnum::UtxoCoin (c)
}   }

impl From<EthCoin> for MmCoinEnum {
    fn from (c: EthCoin) -> MmCoinEnum {
        MmCoinEnum::EthCoin (c)
}   }

impl From<TestCoin> for MmCoinEnum {
    fn from (c: TestCoin) -> MmCoinEnum {
        MmCoinEnum::Test (c)
}   }

// NB: When stable and groked by IDEs, `enum_dispatch` can be used instead of `Deref` to speed things up.
impl Deref for MmCoinEnum {
    type Target = dyn MmCoin;
    fn deref (&self) -> &dyn MmCoin {
        match self {
            &MmCoinEnum::UtxoCoin (ref c) => c,
            &MmCoinEnum::EthCoin (ref c) => c,
            &MmCoinEnum::Test (ref c) => c,
}   }   }

struct CoinsContext {
    /// A map from a currency ticker symbol to the corresponding coin.
    /// Similar to `LP_coins`.
    coins: DuplexMutex<HashMap<String, MmCoinEnum>>
}
impl CoinsContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    fn from_ctx (ctx: &MmArc) -> Result<Arc<CoinsContext>, String> {
        Ok (try_s! (from_ctx (&ctx.coins_ctx, move || {
            Ok (CoinsContext {
                coins: DuplexMutex::new (HashMap::new())
            })
        })))
    }
}

pub type RpcTransportEventHandlerShared = Arc<dyn RpcTransportEventHandler + Send + Sync + 'static>;

/// Common methods to measure the outgoing requests and incoming responses statistics.
pub trait RpcTransportEventHandler {
    fn debug_info(&self) -> String;

    fn on_outgoing_request(&self, data: &[u8]);

    fn on_incoming_response(&self, data: &[u8]);
}

impl fmt::Debug for dyn RpcTransportEventHandler + Send + Sync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.debug_info())
    }
}

impl RpcTransportEventHandler for RpcTransportEventHandlerShared {
    fn debug_info(&self) -> String {
        self.deref().debug_info()
    }

    fn on_outgoing_request(&self, data: &[u8]) {
        self.as_ref().on_outgoing_request(data)
    }

    fn on_incoming_response(&self, data: &[u8]) {
        self.as_ref().on_incoming_response(data)
    }
}

impl<T: RpcTransportEventHandler> RpcTransportEventHandler for Vec<T> {
    fn debug_info(&self) -> String {
        let selfi: Vec<String> = self.iter().map(|x| x.debug_info()).collect();
        format!("{:?}", selfi)
    }

    fn on_outgoing_request(&self, data: &[u8]) {
        for handler in self {
            handler.on_outgoing_request(data)
        }
    }

    fn on_incoming_response(&self, data: &[u8]) {
        for handler in self {
            handler.on_incoming_response(data)
        }
    }
}

pub enum RpcClientType {
    Native,
    Electrum,
    Ethereum,
}

impl ToString for RpcClientType {
    fn to_string(&self) -> String {
        match self {
            RpcClientType::Native => "native".into(),
            RpcClientType::Electrum => "electrum".into(),
            RpcClientType::Ethereum => "ethereum".into(),
        }
    }
}

#[derive(Clone)]
pub struct CoinTransportMetrics {
    /// Using a weak reference by default in order to avoid circular references and leaks.
    metrics: MetricsWeak,
    /// Name of coin the rpc client is intended to work with.
    ticker: String,
    /// RPC client type.
    client: String,
}

impl CoinTransportMetrics {
    fn new(metrics: MetricsWeak, ticker: String, client: RpcClientType) -> CoinTransportMetrics {
        CoinTransportMetrics { metrics, ticker, client: client.to_string() }
    }

    fn into_shared(self) -> RpcTransportEventHandlerShared {
        Arc::new(self)
    }
}

impl RpcTransportEventHandler for CoinTransportMetrics {
    fn debug_info(&self) -> String {
        "CoinTransportMetrics".into()
    }

    fn on_outgoing_request(&self, data: &[u8]) {
        mm_counter!(self.metrics, "rpc_client.traffic.out", data.len() as u64,
            "coin" => self.ticker.clone(), "client" => self.client.clone());
        mm_counter!(self.metrics, "rpc_client.request.count", 1,
            "coin" => self.ticker.clone(), "client" => self.client.clone());
    }

    fn on_incoming_response(&self, data: &[u8]) {
        mm_counter!(self.metrics, "rpc_client.traffic.in", data.len() as u64,
            "coin" => self.ticker.clone(), "client" => self.client.clone());
        mm_counter!(self.metrics, "rpc_client.response.count", 1,
            "coin" => self.ticker.clone(), "client" => self.client.clone());
    }
}

/// Adds a new currency into the list of currencies configured.
///
/// Returns an error if the currency already exists. Initializing the same currency twice is a bad habit
/// (might lead to misleading and confusing information during debugging and maintenance, see DRY)
/// and should be fixed on the call site.
///
/// * `req` - Payload of the corresponding "enable" or "electrum" RPC request.
pub async fn lp_coininit (ctx: &MmArc, ticker: &str, req: &Json) -> Result<MmCoinEnum, String> {
    let cctx = try_s! (CoinsContext::from_ctx (ctx));
    {   let coins = try_s! (cctx.coins.sleeplock (77) .await);
        if coins.get (ticker) .is_some() {return ERR! ("Coin {} already initialized", ticker)}   }

    let coins_en = if let Some (coins) = ctx.conf["coins"].as_array() {
        coins.iter().find (|coin| coin["coin"].as_str() == Some (ticker)) .unwrap_or (&Json::Null)
    } else {&Json::Null};

    if coins_en.is_null() {
        ctx.log.log ("😅", &[&("coin" as &str), &ticker, &("no-conf" as &str)],
            &fomat! ("Warning, coin " (ticker) " is used without a corresponding configuration."));
    }

    if coins_en["mm2"].is_null() && req["mm2"].is_null() {return ERR! (concat! (
        "mm2 param is not set neither in coins config nor enable request, ",
        "assuming that coin is not supported"
    ))}
    let secret = &*ctx.secp256k1_key_pair().private().secret;

    let coin: MmCoinEnum = if coins_en["etomic"].is_null() {
        try_s! (utxo_coin_from_conf_and_request (ctx, ticker, coins_en, req, secret) .await) .into()
    } else {
        try_s! (eth_coin_from_conf_and_request (ctx, ticker, coins_en, req, secret) .await) .into()
    };

    let block_count = try_s! (coin.current_block().compat().await);
    // TODO, #156: Warn the user when we know that the wallet is under-initialized.
    log! ([=ticker] if !coins_en["etomic"].is_null() {", etomic"} ", " [=block_count]);
    // TODO AP: locking the coins list during the entire initialization prevents different coins from being
    // activated concurrently which results in long activation time: https://github.com/KomodoPlatform/atomicDEX/issues/24
    // So I'm leaving the possibility of race condition intentionally in favor of faster concurrent activation.
    // Should consider refactoring: maybe extract the RPC client initialization part from coin init functions.
    let mut coins = try_s! (cctx.coins.sleeplock (77) .await);
    match coins.raw_entry_mut().from_key (ticker) {
        RawEntryMut::Occupied (_oe) => return ERR! ("Coin {} already initialized", ticker),
        RawEntryMut::Vacant (ve) => ve.insert (ticker.to_string(), coin.clone())
    };
    let history = req["tx_history"].as_bool().unwrap_or(false);
    #[cfg(not(feature = "native"))] let history = {
        if history {ctx.log.log ("🍼", &[&("tx_history" as &str), &ticker],
            "Note that the WASM port does not include the history loading thread at the moment.")}
        false
    };
    if history {
        try_s!(thread::Builder::new().name(format!("tx_history_{}", ticker)).spawn({
            let coin = coin.clone();
            let ctx = ctx.clone();
            move || coin.process_history_loop(ctx)
        }));
    }

    Ok (coin)
}

/// NB: Returns only the enabled (aka active) coins.
pub fn lp_coinfind (ctx: &MmArc, ticker: &str) -> Result<Option<MmCoinEnum>, String> {
    let cctx = try_s! (CoinsContext::from_ctx (ctx));
    let coins = try_s! (cctx.coins.spinlock (77));
    Ok (coins.get (ticker) .map (|coin| coin.clone()))
}

/// NB: Returns only the enabled (aka active) coins.
pub async fn lp_coinfindᵃ (ctx: &MmArc, ticker: &str) -> Result<Option<MmCoinEnum>, String> {
    let cctx = try_s! (CoinsContext::from_ctx (ctx));
    let coins = try_s! (cctx.coins.sleeplock (77) .await);
    Ok (coins.get (ticker) .map (|coin| coin.clone()))
}

pub async fn withdraw (ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s! (req["coin"].as_str().ok_or ("No 'coin' field")) .to_owned();
    let coin = match lp_coinfindᵃ (&ctx, &ticker) .await {
        Ok (Some (t)) => t,
        Ok (None) => return ERR! ("No such coin: {}", ticker),
        Err (err) => return ERR! ("!lp_coinfind({}): {}", ticker, err)
    };
    let withdraw_req: WithdrawRequest = try_s! (json::from_value (req));
    let res = try_s! (coin.withdraw (withdraw_req) .compat().await);
    let body = try_s! (json::to_vec (&res));
    Ok (try_s! (Response::builder().body (body)))
}

pub async fn send_raw_transaction (ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s! (req["coin"].as_str().ok_or ("No 'coin' field")) .to_owned();
    let coin = match lp_coinfindᵃ (&ctx, &ticker) .await {
        Ok (Some (t)) => t,
        Ok (None) => return ERR! ("No such coin: {}", ticker),
        Err (err) => return ERR! ("!lp_coinfind({}): {}", ticker, err)
    };
    let bytes_string = try_s! (req["tx_hex"].as_str().ok_or ("No 'tx_hex' field"));
    let res = try_s! (coin.send_raw_tx (&bytes_string) .compat().await);
    let body = try_s! (json::to_vec (&json! ({"tx_hash": res})));
    Ok (try_s! (Response::builder().body (body)))
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", content = "additional_info")]
pub enum HistorySyncState {
    NotEnabled,
    NotStarted,
    InProgress(Json),
    Error(Json),
    Finished,
}

/// Returns the transaction history of selected coin. Returns no more than `limit` records (default: 10).
/// Skips the first `skip` records (default: 0).
/// Transactions are sorted by number of confirmations in ascending order.
pub fn my_tx_history(ctx: MmArc, req: Json) -> HyRes {
    let ticker = try_h!(req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let coin = match lp_coinfind(&ctx, &ticker) {  // Should switch to lp_coinfindᵃ when my_tx_history is async.
        Ok(Some(t)) => t,
        Ok(None) => return rpc_err_response(500, &fomat!("No such coin: " (ticker))),
        Err(err) => return rpc_err_response(500, &fomat!("!lp_coinfind(" (ticker) "): " (err)))
    };
    let limit = req["limit"].as_u64().unwrap_or(10);
    let from_id: Option<BytesJson> = try_h!(json::from_value(req["from_id"].clone()));
    let file_path = coin.tx_history_path(&ctx);
    let content = slurp(&file_path);
    let history: Vec<TransactionDetails> = match json::from_slice(&content) {
        Ok(h) => h,
        Err(e) => {
            if !content.is_empty() {
                log!("Error " (e) " on attempt to deserialize file " (file_path.display()) " content as Vec<TransactionDetails>");
            }
            vec![]
        }
    };
    let total_records = history.len();
    Box::new(coin.current_block().and_then(move |block_number| {
        let skip = match &from_id {
            Some(id) => {
                try_h!(history.iter().position(|item| item.internal_id == *id).ok_or(format!("from_id {:02x} is not found", id))) + 1
            },
            None => 0,
        };
        let history = history.into_iter().skip(skip).take(limit as usize);
        let history: Vec<Json> = history.map(|item| {
            let tx_block = item.block_height;
            let mut json = unwrap!(json::to_value(item));
            json["confirmations"] = if tx_block == 0 {
                Json::from(0)
            } else {
                if block_number >= tx_block {
                    Json::from((block_number - tx_block) + 1)
                } else {
                    Json::from(0)
                }
            };
            json
        }).collect();
        rpc_response(200, json!({
            "result": {
                "transactions": history,
                "limit": limit,
                "skipped": skip,
                "from_id": from_id,
                "total": total_records,
                "current_block": block_number,
                "sync_status": coin.history_sync_status(),
            }
        }).to_string())
    }))
}

pub async fn get_trade_fee(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let coin = match lp_coinfindᵃ(&ctx, &ticker).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): {}", ticker, err),
    };
    let fee_info = try_s!(coin.get_trade_fee().compat().await);
    let res = try_s!(json::to_vec(&json!({
        "result": fee_info
    })));
    Ok(try_s!(Response::builder().body(res)))
}

#[derive(Serialize)]
struct EnabledCoin {
    ticker: String,
    address: String,
}

pub async fn get_enabled_coins(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    let coins_ctx: Arc<CoinsContext> = try_s!(CoinsContext::from_ctx(&ctx));
    let coins = try_s!(coins_ctx.coins.sleeplock(77).await);
    let enabled_coins: Vec<_> = coins.iter().map(|(ticker, coin)| EnabledCoin {
        ticker: ticker.clone(),
        address: coin.my_address().to_string(),
    }).collect();
    let res = try_s!(json::to_vec(&json!({
        "result": enabled_coins
    })));
    Ok(try_s!(Response::builder().body(res)))
}

pub fn disable_coin(ctx: &MmArc, ticker: &str) -> Result<(), String> {
    let coins_ctx = try_s!(CoinsContext::from_ctx(&ctx));
    let mut coins = try_s!(coins_ctx.coins.spinlock(77));
    match coins.remove(ticker) {
        Some(_) => Ok(()),
        None => ERR!("{} is disabled already", ticker)
    }
}

#[derive(Deserialize)]
pub struct ConfirmationsReq {
    coin: String,
    confirmations: u64,
}

pub async fn set_required_confirmations(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: ConfirmationsReq = try_s!(json::from_value(req));
    let coin = match lp_coinfindᵃ(&ctx, &req.coin).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin {}", req.coin),
        Err(err) => return ERR!("!lp_coinfind ({}): {}", req.coin, err),
    };
    coin.set_required_confirmations(req.confirmations);
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "coin": req.coin,
            "confirmations": coin.required_confirmations(),
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

#[derive(Deserialize)]
pub struct RequiresNotaReq {
    coin: String,
    requires_notarization: bool,
}

pub async fn set_requires_notarization(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: RequiresNotaReq = try_s!(json::from_value(req));
    let coin = match lp_coinfindᵃ(&ctx, &req.coin).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin {}", req.coin),
        Err(err) => return ERR!("!lp_coinfind ({}): {}", req.coin, err),
    };
    coin.set_requires_notarization(req.requires_notarization);
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "coin": req.coin,
            "requires_notarization": coin.requires_notarization(),
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn show_priv_key(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let coin = match lp_coinfindᵃ(&ctx, &ticker).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): {}", ticker, err),
    };
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "coin": ticker,
            "priv_key": coin.display_priv_key(),
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

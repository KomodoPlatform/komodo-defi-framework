
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
#![feature(async_await, async_closure)]
#![feature(hash_raw_entry)]

#[macro_use] extern crate common;
#[macro_use] extern crate fomat_macros;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate unwrap;

use bigdecimal::BigDecimal;
use common::{HyRes, rpc_response, rpc_err_response};
use common::mm_ctx::{from_ctx, MmArc};
use common::mm_number::MmNumber;
use futures01::{Future};
use gstuff::{slurp};
use http::Response;
use rpc::v1::types::{Bytes as BytesJson};
use serde_json::{self as json, Value as Json};
use std::borrow::Cow;
use std::collections::hash_map::{HashMap, RawEntryMut};
use std::fmt::Debug;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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

pub trait Transaction: Debug + 'static {
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

pub type TransactionFut = Box<dyn Future<Item=TransactionEnum, Error=String>>;

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
    ) -> Result<(), String>;

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Result<(), String>;

    fn validate_taker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Result<(), String>;

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
    ) -> Result<Option<TransactionEnum>, String>;

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
        wait_until: u64,
        check_every: u64,
    ) -> Result<(), String>;

    fn wait_for_tx_spend(&self, transaction: &[u8], wait_until: u64, from_block: u64) -> Result<TransactionEnum, String>;

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String>;

    fn current_block(&self) -> Box<dyn Future<Item=u64, Error=String> + Send>;

    fn address_from_pubkey_str(&self, pubkey: &str) -> Result<String, String>;
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

pub enum TradeInfo {
    // going to act as maker
    Maker,
    // going to act as taker with expected dexfee amount
    Taker(BigDecimal),
}

/// NB: Implementations are expected to follow the pImpl idiom, providing cheap reference-counted cloning and garbage collection.
pub trait MmCoin: SwapOps + MarketCoinOps + Debug + 'static {
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
                    ctx.log.log("", &[&"tx_history", &self.ticker().to_string()], &ERRL!("Error {} on history deserialization, resetting the cache", e));
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
    fn tx_details_by_hash(&self, hash: &[u8]) -> Result<TransactionDetails, String>;

    /// Transaction history background sync status
    fn history_sync_status(&self) -> HistorySyncState;

    /// Get fee to be paid per 1 swap transaction
    fn get_trade_fee(&self) -> HyRes;

    /// required transaction confirmations number to ensure double-spend safety
    fn required_confirmations(&self) -> u64;

    /// set required transaction confirmations number
    fn set_required_confirmations(&self, confirmations: u64);
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
    coins: Mutex<HashMap<String, MmCoinEnum>>
}
impl CoinsContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    fn from_ctx (ctx: &MmArc) -> Result<Arc<CoinsContext>, String> {
        Ok (try_s! (from_ctx (&ctx.coins_ctx, move || {
            Ok (CoinsContext {
                coins: Mutex::new (HashMap::new())
            })
        })))
    }
}

/// Adds a new currency into the list of currencies configured.
///
/// Returns an error if the currency already exists. Initializing the same currency twice is a bad habit
/// (might lead to misleading and confusing information during debugging and maintenance, see DRY)
/// and should be fixed on the call site.
///
/// * `req` - Payload of the corresponding "enable" or "electrum" RPC request.
pub fn lp_coininit (ctx: &MmArc, ticker: &str, req: &Json) -> Result<MmCoinEnum, String> {
    let cctx = try_s! (CoinsContext::from_ctx (ctx));
    {
        let coins = try_s!(cctx.coins.lock());
        if coins.get(ticker).is_some() {
            return ERR!("Coin {} already initialized", ticker)
        };
    }

    let coins_en = if let Some (coins) = ctx.conf["coins"].as_array() {
        coins.iter().find (|coin| coin["coin"].as_str() == Some (ticker)) .unwrap_or (&Json::Null)
    } else {&Json::Null};

    if coins_en.is_null() {
        ctx.log.log ("😅", &[&("coin" as &str), &ticker, &("no-conf" as &str)],
            &fomat! ("Warning, coin " (ticker) " is used without a corresponding configuration."));
    }

    if coins_en["mm2"].is_null() && req["mm2"].is_null() {
        return ERR!("mm2 param is not set neither in coins config nor enable request, assuming that coin is not supported");
    }
    let secret = &*ctx.secp256k1_key_pair().private().secret;

    let coin: MmCoinEnum = if coins_en["etomic"].is_null() {
        try_s! (utxo_coin_from_conf_and_request (ticker, coins_en, req, secret)) .into()
    } else {
        try_s! (eth_coin_from_conf_and_request (ctx, ticker, coins_en, req, secret)) .into()
    };

    let block_count = try_s!(coin.current_block().wait());
    // TODO, #156: Warn the user when we know that the wallet is under-initialized.
    log! ([=ticker] if !coins_en["etomic"].is_null() {", etomic"} ", " [=block_count]);
    // TODO AP: locking the coins list during the entire initialization prevents different coins from being
    // activated concurrently which results in long activation time: https://github.com/KomodoPlatform/atomicDEX/issues/24
    // So I'm leaving the possibility of race condition intentionally in favor of faster concurrent activation.
    // Should consider refactoring: maybe extract the RPC client initialization part from coin init functions.
    let cctx = try_s! (CoinsContext::from_ctx (ctx));
    let mut coins = try_s! (cctx.coins.lock());
    match coins.raw_entry_mut().from_key (ticker) {
        RawEntryMut::Occupied (_oe) => return ERR! ("Coin {} already initialized", ticker),
        RawEntryMut::Vacant (ve) => ve.insert (ticker.to_string(), coin.clone())
    };
    let history = req["tx_history"].as_bool().unwrap_or(false);
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
    let coins = try_s! (cctx.coins.lock());
    Ok (coins.get (ticker) .map (|coin| coin.clone()))
}

pub fn withdraw (ctx: MmArc, req: Json) -> HyRes {
    let ticker = try_h! (req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let coin = match lp_coinfind (&ctx, &ticker) {
        Ok (Some (t)) => t,
        Ok (None) => return rpc_err_response (500, &fomat! ("No such coin: " (ticker))),
        Err (err) => return rpc_err_response (500, &fomat! ("!lp_coinfind(" (ticker) "): " (err)))
    };
    let withdraw_req: WithdrawRequest = try_h!(json::from_value(req));
    Box::new(coin.withdraw(withdraw_req).and_then(|res| {
        let body = try_h!(json::to_string(&res));
        rpc_response(200, body)
    }))
}

pub fn send_raw_transaction (ctx: MmArc, req: Json) -> HyRes {
    let ticker = try_h! (req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let coin = match lp_coinfind (&ctx, &ticker) {
        Ok (Some (t)) => t,
        Ok (None) => return rpc_err_response (500, &fomat! ("No such coin: " (ticker))),
        Err (err) => return rpc_err_response (500, &fomat! ("!lp_coinfind(" (ticker) "): " (err)))
    };
    let bytes_string = try_h! (req["tx_hex"].as_str().ok_or ("No 'tx_hex' field"));
    Box::new(coin.send_raw_tx(&bytes_string).and_then(|res| {
        rpc_response(200, json!({
            "tx_hash": res
        }).to_string())
    }))
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
    let coin = match lp_coinfind(&ctx, &ticker) {
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

pub fn get_trade_fee(ctx: MmArc, req: Json) -> HyRes {
    let ticker = try_h!(req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let coin = match lp_coinfind(&ctx, &ticker) {
        Ok(Some(t)) => t,
        Ok(None) => return rpc_err_response(500, &fomat!("No such coin: " (ticker))),
        Err(err) => return rpc_err_response(500, &fomat!("!lp_coinfind(" (ticker) "): " (err)))
    };
    coin.get_trade_fee()
}

#[derive(Serialize)]
struct EnabledCoin {
    ticker: String,
    address: String,
}

pub fn get_enabled_coins(ctx: MmArc) -> HyRes {
    let coins_ctx: Arc<CoinsContext> = try_h!(CoinsContext::from_ctx(&ctx));
    let enabled_coins: Vec<_> = try_h!(coins_ctx.coins.lock()).iter().map(|(ticker, coin)| EnabledCoin {
        ticker: ticker.clone(),
        address: coin.my_address().to_string(),
    }).collect();
    rpc_response(200, json!({
        "result": enabled_coins
    }).to_string())
}

pub fn disable_coin(ctx: &MmArc, ticker: &str) -> Result<(), String> {
    let coins_ctx = try_s!(CoinsContext::from_ctx(&ctx));
    let mut coins = try_s!(coins_ctx.coins.lock());
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
    let coin = match lp_coinfind(&ctx, &req.coin) {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin {}", req.coin),
        Err(err) => return ERR!("!lp_coinfind ({}): {}", req.coin, err),
    };
    coin.set_required_confirmations(req.confirmations);
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "coin": req.coin,
            "confirmations": req.confirmations,
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

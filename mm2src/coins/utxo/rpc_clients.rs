#![cfg_attr(not(feature = "native"), allow(unused_imports))]
#![cfg_attr(not(feature = "native"), allow(unused_macros))]
#![cfg_attr(not(feature = "native"), allow(dead_code))]

use crate::utxo::sat_from_big_decimal;
use crate::{RpcTransportEventHandler, RpcTransportEventHandlerShared};
use bigdecimal::BigDecimal;
use chain::{BlockHeader, OutPoint, Transaction as UtxoTx};
use common::custom_futures::select_ok_sequential;
use common::executor::{spawn, Timer};
use common::jsonrpc_client::{JsonRpcClient, JsonRpcError, JsonRpcMultiClient, JsonRpcRemoteAddr, JsonRpcRequest,
                             JsonRpcResponse, JsonRpcResponseFut, RpcRes};
use common::mm_number::MmNumber;
use common::wio::slurp_req;
use common::{median, OrdRange, StringError};
use futures::channel::oneshot as async_oneshot;
#[cfg(not(feature = "native"))]
use futures::channel::oneshot::Sender as ShotSender;
use futures::compat::{Future01CompatExt, Stream01CompatExt};
use futures::future::{select as select_func, Either, FutureExt, TryFutureExt};
use futures::io::Error;
use futures::lock::Mutex as AsyncMutex;
use futures::{select, StreamExt};
use futures01::future::select_ok;
use futures01::sync::{mpsc, oneshot};
use futures01::{Future, Sink, Stream};
use futures_timer::FutureExt as FutureTimerExt;
use gstuff::{now_float, now_ms};
use http::header::AUTHORIZATION;
use http::Uri;
use http::{Request, StatusCode};
use keys::Address;
#[cfg(test)] use mocktopus::macros::*;
use rpc::v1::types::{Bytes as BytesJson, Transaction as RpcTransaction, VerboseBlockClient, H256 as H256Json};
#[cfg(feature = "native")] use rustls::{self};
use script::Builder;
use serde_json::{self as json, Value as Json};
use serialization::{deserialize, serialize, CompactInteger, Reader};
use sha2::{Digest, Sha256};
use std::collections::hash_map::{Entry, HashMap};
use std::fmt;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::num::NonZeroU64;
use std::ops::Deref;
#[cfg(not(feature = "native"))] use std::os::raw::c_char;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
#[cfg(feature = "native")]
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(feature = "native")] use tokio::net::TcpStream;
#[cfg(feature = "native")] use tokio_rustls::webpki::DNSNameRef;
#[cfg(feature = "native")]
use tokio_rustls::{client::TlsStream, TlsConnector};
#[cfg(feature = "native")] use webpki_roots::TLS_SERVER_ROOTS;

pub type AddressesByLabelResult = HashMap<String, AddressPurpose>;

#[derive(Debug, Deserialize)]
pub struct AddressPurpose {
    purpose: String,
}

/// Skips the server certificate verification on TLS connection
pub struct NoCertificateVerification {}

#[cfg(feature = "native")]
impl rustls::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _roots: &rustls::RootCertStore,
        _presented_certs: &[rustls::Certificate],
        _dns_name: DNSNameRef<'_>,
        _ocsp: &[u8],
    ) -> Result<rustls::ServerCertVerified, rustls::TLSError> {
        Ok(rustls::ServerCertVerified::assertion())
    }
}

#[derive(Debug)]
pub enum UtxoRpcClientEnum {
    Native(NativeClient),
    Electrum(ElectrumClient),
}

impl From<ElectrumClient> for UtxoRpcClientEnum {
    fn from(client: ElectrumClient) -> UtxoRpcClientEnum { UtxoRpcClientEnum::Electrum(client) }
}

impl From<NativeClient> for UtxoRpcClientEnum {
    fn from(client: NativeClient) -> UtxoRpcClientEnum { UtxoRpcClientEnum::Native(client) }
}

impl Deref for UtxoRpcClientEnum {
    type Target = dyn UtxoRpcClientOps;
    fn deref(&self) -> &dyn UtxoRpcClientOps {
        match self {
            UtxoRpcClientEnum::Native(ref c) => c,
            UtxoRpcClientEnum::Electrum(ref c) => c,
        }
    }
}

impl Clone for UtxoRpcClientEnum {
    fn clone(&self) -> Self {
        match self {
            UtxoRpcClientEnum::Native(c) => UtxoRpcClientEnum::Native(c.clone()),
            UtxoRpcClientEnum::Electrum(c) => UtxoRpcClientEnum::Electrum(c.clone()),
        }
    }
}

impl UtxoRpcClientEnum {
    pub fn wait_for_confirmations(
        &self,
        tx: &UtxoTx,
        confirmations: u32,
        requires_notarization: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let tx = tx.clone();
        let selfi = self.clone();
        let fut = async move {
            loop {
                if now_ms() / 1000 > wait_until {
                    return ERR!(
                        "Waited too long until {} for transaction {:?} to be confirmed {} times",
                        wait_until,
                        tx,
                        confirmations
                    );
                }

                match selfi
                    .get_verbose_transaction(tx.hash().reversed().into())
                    .compat()
                    .await
                {
                    Ok(t) => {
                        let tx_confirmations = if requires_notarization {
                            t.confirmations
                        } else {
                            t.rawconfirmations.unwrap_or(t.confirmations)
                        };
                        if tx_confirmations >= confirmations {
                            return Ok(());
                        } else {
                            log!({ "Waiting for tx {:?} confirmations, now {}, required {}, requires_notarization {}", tx.hash().reversed(), tx_confirmations, confirmations, requires_notarization });
                        }
                    },
                    Err(e) => {
                        log!("Error " [e] " getting the transaction " [tx.hash().reversed()] ", retrying in 10 seconds")
                    },
                }

                Timer::sleep(check_every as f64).await;
            }
        };
        Box::new(fut.boxed().compat())
    }
}

/// Generic unspent info required to build transactions, we need this separate type because native
/// and Electrum provide different list_unspent format.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct UnspentInfo {
    pub outpoint: OutPoint,
    pub value: u64,
    /// The block height transaction mined in.
    /// Note None if the transaction is not mined yet.
    pub height: Option<u64>,
}

pub type UtxoRpcRes<T> = Box<dyn Future<Item = T, Error = String> + Send + 'static>;

/// Common operations that both types of UTXO clients have but implement them differently
pub trait UtxoRpcClientOps: fmt::Debug + Send + Sync + 'static {
    fn list_unspent(&self, address: &Address, decimals: u8) -> UtxoRpcRes<Vec<UnspentInfo>>;

    fn send_transaction(&self, tx: &UtxoTx) -> UtxoRpcRes<H256Json>;

    fn send_raw_transaction(&self, tx: BytesJson) -> RpcRes<H256Json>;

    fn get_transaction_bytes(&self, txid: H256Json) -> RpcRes<BytesJson>;

    fn get_verbose_transaction(&self, txid: H256Json) -> RpcRes<RpcTransaction>;

    fn get_block_count(&self) -> RpcRes<u64>;

    fn display_balance(&self, address: Address, decimals: u8) -> RpcRes<BigDecimal>;

    /// returns fee estimation per KByte in satoshis
    fn estimate_fee_sat(
        &self,
        decimals: u8,
        fee_method: &EstimateFeeMethod,
        mode: &Option<EstimateFeeMode>,
    ) -> RpcRes<u64>;

    fn get_relay_fee(&self) -> RpcRes<BigDecimal>;

    fn find_output_spend(
        &self,
        tx: &UtxoTx,
        vout: usize,
        from_block: u64,
    ) -> Box<dyn Future<Item = Option<UtxoTx>, Error = String> + Send>;

    /// Get median time past for `count` blocks in the past including `starting_block`
    fn get_median_time_past(
        &self,
        starting_block: u64,
        count: NonZeroU64,
    ) -> Box<dyn Future<Item = u32, Error = String> + Send>;
}

#[derive(Clone, Deserialize, Debug)]
pub struct NativeUnspent {
    pub txid: H256Json,
    pub vout: u32,
    pub address: String,
    pub account: Option<String>,
    #[serde(rename = "scriptPubKey")]
    pub script_pub_key: BytesJson,
    pub amount: MmNumber,
    pub confirmations: u64,
    pub spendable: bool,
}

#[derive(Clone, Deserialize, Debug)]
pub struct ValidateAddressRes {
    #[serde(rename = "isvalid")]
    pub is_valid: bool,
    pub address: String,
    #[serde(rename = "scriptPubKey")]
    pub script_pub_key: BytesJson,
    #[serde(rename = "segid")]
    pub seg_id: Option<u32>,
    #[serde(rename = "ismine")]
    pub is_mine: Option<bool>,
    #[serde(rename = "iswatchonly")]
    pub is_watch_only: Option<bool>,
    #[serde(rename = "isscript")]
    pub is_script: bool,
    pub account: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListTransactionsItem {
    pub account: Option<String>,
    #[serde(default)]
    pub address: String,
    pub category: String,
    pub amount: f64,
    pub vout: u64,
    #[serde(default)]
    pub fee: f64,
    #[serde(default)]
    pub confirmations: i64,
    #[serde(default)]
    pub blockhash: H256Json,
    #[serde(default)]
    pub blockindex: u64,
    #[serde(default)]
    pub txid: H256Json,
    pub timereceived: u64,
    #[serde(default)]
    pub walletconflicts: Vec<String>,
}

impl ListTransactionsItem {
    /// Checks if the transaction is conflicting.
    /// It means the transaction has conflicts or has negative confirmations.
    pub fn is_conflicting(&self) -> bool { self.confirmations < 0 || !self.walletconflicts.is_empty() }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReceivedByAddressItem {
    #[serde(default)]
    pub account: String,
    pub address: String,
    pub txids: Vec<H256Json>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EstimateSmartFeeRes {
    #[serde(rename = "feerate")]
    #[serde(default)]
    pub fee_rate: f64,
    #[serde(default)]
    pub errors: Vec<String>,
    pub blocks: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListSinceBlockRes {
    transactions: Vec<ListTransactionsItem>,
    #[serde(rename = "lastblock")]
    last_block: H256Json,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkInfoLocalAddress {
    address: String,
    port: u16,
    score: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkInfoNetwork {
    name: String,
    limited: bool,
    reachable: bool,
    proxy: String,
    proxy_randomize_credentials: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkInfo {
    connections: u64,
    #[serde(rename = "localaddresses")]
    local_addresses: Vec<NetworkInfoLocalAddress>,
    #[serde(rename = "localservices")]
    local_services: String,
    networks: Vec<NetworkInfoNetwork>,
    #[serde(rename = "protocolversion")]
    protocol_version: u64,
    #[serde(rename = "relayfee")]
    relay_fee: BigDecimal,
    subversion: String,
    #[serde(rename = "timeoffset")]
    time_offset: i64,
    version: u64,
    warnings: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GetAddressInfoRes {
    // as of now we are interested in ismine and iswatchonly fields only, but this response contains much more info
    #[serde(rename = "ismine")]
    pub is_mine: bool,
    #[serde(rename = "iswatchonly")]
    pub is_watch_only: bool,
}

#[derive(Debug)]
pub enum EstimateFeeMethod {
    /// estimatefee, deprecated in many coins: https://bitcoincore.org/en/doc/0.16.0/rpc/util/estimatefee/
    Standard,
    /// estimatesmartfee added since 0.16.0 bitcoind RPC: https://bitcoincore.org/en/doc/0.16.0/rpc/util/estimatesmartfee/
    SmartFee,
}

pub type RpcReqSub<T> = async_oneshot::Sender<Result<T, JsonRpcError>>;

/// RPC client for UTXO based coins
/// https://developer.bitcoin.org/reference/rpc/index.html - Bitcoin RPC API reference
/// Other coins have additional methods or miss some of these
/// This description will be updated with more info
#[derive(Debug)]
pub struct NativeClientImpl {
    /// Name of coin the rpc client is intended to work with
    pub coin_ticker: String,
    /// The uri to send requests to
    pub uri: String,
    /// Value of Authorization header, e.g. "Basic base64(user:password)"
    pub auth: String,
    /// Transport event handlers
    pub event_handlers: Vec<RpcTransportEventHandlerShared>,
    pub request_id: AtomicU64,
    pub list_unspent_in_progress: AtomicBool,
    pub list_unspent_subs: AsyncMutex<Vec<RpcReqSub<Vec<NativeUnspent>>>>,
}

#[cfg(test)]
impl Default for NativeClientImpl {
    fn default() -> Self {
        NativeClientImpl {
            coin_ticker: "TEST".to_string(),
            uri: "".to_string(),
            auth: "".to_string(),
            event_handlers: vec![],
            request_id: Default::default(),
            list_unspent_in_progress: Default::default(),
            list_unspent_subs: Default::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NativeClient(pub Arc<NativeClientImpl>);
impl Deref for NativeClient {
    type Target = NativeClientImpl;
    fn deref(&self) -> &NativeClientImpl { &*self.0 }
}

/// The trait provides methods to generate the JsonRpcClient instance info such as name of coin.
pub trait UtxoJsonRpcClientInfo: JsonRpcClient {
    /// Name of coin the rpc client is intended to work with
    fn coin_name(&self) -> &str;

    /// Generate client info from coin name
    fn client_info(&self) -> String { format!("coin: {}", self.coin_name()) }
}

impl UtxoJsonRpcClientInfo for NativeClientImpl {
    fn coin_name(&self) -> &str { self.coin_ticker.as_str() }
}

impl JsonRpcClient for NativeClientImpl {
    fn version(&self) -> &'static str { "1.0" }

    fn next_id(&self) -> String { self.request_id.fetch_add(1, AtomicOrdering::Relaxed).to_string() }

    fn client_info(&self) -> String { UtxoJsonRpcClientInfo::client_info(self) }

    fn transport(&self, request: JsonRpcRequest) -> JsonRpcResponseFut {
        let request_body = try_fus!(json::to_string(&request));
        // measure now only body length, because the `hyper` crate doesn't allow to get total HTTP packet length
        self.event_handlers.on_outgoing_request(request_body.as_bytes());

        let uri = self.uri.clone();

        let http_request = try_fus!(Request::builder()
            .method("POST")
            .header(AUTHORIZATION, self.auth.clone())
            .uri(uri.clone())
            .body(Vec::from(request_body)));

        let event_handles = self.event_handlers.clone();
        Box::new(slurp_req(http_request).boxed().compat().then(
            move |result| -> Result<(JsonRpcRemoteAddr, JsonRpcResponse), String> {
                let res = try_s!(result);
                // measure now only body length, because the `hyper` crate doesn't allow to get total HTTP packet length
                event_handles.on_incoming_response(&res.2);

                let body = try_s!(std::str::from_utf8(&res.2));

                if res.0 != StatusCode::OK {
                    return ERR!(
                        "Rpc request {:?} failed with HTTP status code {}, response body: {}",
                        request,
                        res.0,
                        body
                    );
                }

                let response = try_s!(json::from_str(body));
                Ok((uri.into(), response))
            },
        ))
    }
}

#[cfg_attr(test, mockable)]
impl UtxoRpcClientOps for NativeClient {
    fn list_unspent(&self, address: &Address, decimals: u8) -> UtxoRpcRes<Vec<UnspentInfo>> {
        let fut = self
            .list_unspent_impl(0, std::i32::MAX, vec![address.to_string()])
            .map_err(|e| ERRL!("{}", e))
            .and_then(move |unspents| {
                let unspents: Result<Vec<_>, _> = unspents
                    .into_iter()
                    .map(|unspent| {
                        Ok(UnspentInfo {
                            outpoint: OutPoint {
                                hash: unspent.txid.reversed().into(),
                                index: unspent.vout,
                            },
                            value: try_s!(sat_from_big_decimal(&unspent.amount.to_decimal(), decimals)),
                            height: None,
                        })
                    })
                    .collect();
                Ok(try_s!(unspents))
            });
        Box::new(fut)
    }

    fn send_transaction(&self, tx: &UtxoTx) -> UtxoRpcRes<H256Json> {
        let tx_bytes = BytesJson::from(serialize(tx));
        Box::new(self.send_raw_transaction(tx_bytes).map_err(|e| ERRL!("{}", e)))
    }

    /// https://developer.bitcoin.org/reference/rpc/sendrawtransaction
    fn send_raw_transaction(&self, tx: BytesJson) -> RpcRes<H256Json> { rpc_func!(self, "sendrawtransaction", tx) }

    fn get_transaction_bytes(&self, txid: H256Json) -> RpcRes<BytesJson> { self.get_raw_transaction_bytes(txid) }

    fn get_verbose_transaction(&self, txid: H256Json) -> RpcRes<RpcTransaction> {
        self.get_raw_transaction_verbose(txid)
    }

    fn get_block_count(&self) -> RpcRes<u64> { self.0.get_block_count() }

    fn display_balance(&self, address: Address, _decimals: u8) -> RpcRes<BigDecimal> {
        Box::new(
            self.list_unspent_impl(0, std::i32::MAX, vec![address.to_string()])
                .map(|unspents| {
                    unspents
                        .iter()
                        .fold(BigDecimal::from(0), |sum, unspent| sum + unspent.amount.to_decimal())
                }),
        )
    }

    fn estimate_fee_sat(
        &self,
        decimals: u8,
        fee_method: &EstimateFeeMethod,
        mode: &Option<EstimateFeeMode>,
    ) -> RpcRes<u64> {
        match fee_method {
            EstimateFeeMethod::Standard => Box::new(self.estimate_fee().map(move |fee| {
                if fee > 0.00001 {
                    (fee * 10.0_f64.powf(decimals as f64)) as u64
                } else {
                    1000
                }
            })),
            EstimateFeeMethod::SmartFee => Box::new(self.estimate_smart_fee(mode).map(move |res| {
                if res.fee_rate > 0.00001 {
                    (res.fee_rate * 10.0_f64.powf(decimals as f64)) as u64
                } else {
                    1000
                }
            })),
        }
    }

    fn get_relay_fee(&self) -> RpcRes<BigDecimal> { Box::new(self.get_network_info().map(|info| info.relay_fee)) }

    fn find_output_spend(
        &self,
        tx: &UtxoTx,
        vout: usize,
        from_block: u64,
    ) -> Box<dyn Future<Item = Option<UtxoTx>, Error = String> + Send> {
        let selfi = self.clone();
        let tx = tx.clone();
        let fut = async move {
            let from_block_hash = try_s!(selfi.get_block_hash(from_block).compat().await);
            let list_since_block: ListSinceBlockRes = try_s!(selfi.list_since_block(from_block_hash).compat().await);
            for transaction in list_since_block
                .transactions
                .into_iter()
                .filter(|tx| !tx.is_conflicting())
            {
                let maybe_spend_tx_bytes = try_s!(selfi.get_raw_transaction_bytes(transaction.txid).compat().await);
                let maybe_spend_tx: UtxoTx =
                    try_s!(deserialize(maybe_spend_tx_bytes.as_slice()).map_err(|e| ERRL!("{:?}", e)));

                for input in maybe_spend_tx.inputs.iter() {
                    if input.previous_output.hash == tx.hash() && input.previous_output.index == vout as u32 {
                        return Ok(Some(maybe_spend_tx));
                    }
                }
            }
            Ok(None)
        };
        Box::new(fut.boxed().compat())
    }

    fn get_median_time_past(
        &self,
        starting_block: u64,
        count: NonZeroU64,
    ) -> Box<dyn Future<Item = u32, Error = String> + Send> {
        let selfi = self.clone();
        let fut = async move {
            let starting_block_data = try_s!(selfi.get_block(starting_block.to_string()).compat().await);
            if let Some(median) = starting_block_data.mediantime {
                return Ok(median);
            }

            let mut block_timestamps = vec![starting_block_data.time];
            let from = if starting_block <= count.get() {
                0
            } else {
                starting_block - count.get() + 1
            };
            for block_n in from..starting_block {
                let block_data = try_s!(selfi.get_block(block_n.to_string()).compat().await);
                block_timestamps.push(block_data.time);
            }
            // can unwrap because count is non zero
            Ok(median(block_timestamps.as_mut_slice()).unwrap())
        };
        Box::new(fut.boxed().compat())
    }
}

#[cfg_attr(test, mockable)]
impl NativeClient {
    /// https://developer.bitcoin.org/reference/rpc/listunspent
    pub fn list_unspent_impl(
        &self,
        min_conf: i32,
        max_conf: i32,
        addresses: Vec<String>,
    ) -> RpcRes<Vec<NativeUnspent>> {
        let arc = self.clone();
        if self
            .list_unspent_in_progress
            .compare_and_swap(false, true, AtomicOrdering::Relaxed)
        {
            let fut = async move {
                let (tx, rx) = async_oneshot::channel();
                arc.list_unspent_subs.lock().await.push(tx);
                rx.await.unwrap()
            };
            Box::new(fut.boxed().compat())
        } else {
            let fut = async move {
                let unspents_res = rpc_func!(arc, "listunspent", min_conf, max_conf, addresses)
                    .compat()
                    .await;
                for sub in arc.list_unspent_subs.lock().await.drain(..) {
                    if sub.send(unspents_res.clone()).is_err() {
                        log!("list_unspent_sub is dropped");
                    }
                }
                arc.list_unspent_in_progress.store(false, AtomicOrdering::Relaxed);
                unspents_res
            };
            Box::new(fut.boxed().compat())
        }
    }
}

#[cfg_attr(test, mockable)]
impl NativeClientImpl {
    /// https://developer.bitcoin.org/reference/rpc/importaddress
    pub fn import_address(&self, address: &str, label: &str, rescan: bool) -> RpcRes<()> {
        rpc_func!(self, "importaddress", address, label, rescan)
    }

    /// https://developer.bitcoin.org/reference/rpc/validateaddress
    pub fn validate_address(&self, address: &str) -> RpcRes<ValidateAddressRes> {
        rpc_func!(self, "validateaddress", address)
    }

    pub fn output_amount(&self, txid: H256Json, index: usize) -> UtxoRpcRes<u64> {
        let fut = self.get_raw_transaction_bytes(txid).map_err(|e| ERRL!("{}", e));
        Box::new(fut.and_then(move |bytes| {
            let tx: UtxoTx = try_s!(deserialize(bytes.as_slice()).map_err(|e| ERRL!(
                "Error {:?} trying to deserialize the transaction {:?}",
                e,
                bytes
            )));
            Ok(tx.outputs[index].value)
        }))
    }

    /// https://developer.bitcoin.org/reference/rpc/getblock.html
    /// Always returns verbose block
    pub fn get_block(&self, height: String) -> RpcRes<VerboseBlockClient> {
        let verbose = true;
        rpc_func!(self, "getblock", height, verbose)
    }

    /// https://developer.bitcoin.org/reference/rpc/getblockcount.html
    pub fn get_block_count(&self) -> RpcRes<u64> { rpc_func!(self, "getblockcount") }

    /// https://developer.bitcoin.org/reference/rpc/getrawtransaction.html
    /// Always returns verbose transaction
    fn get_raw_transaction_verbose(&self, txid: H256Json) -> RpcRes<RpcTransaction> {
        let verbose = 1;
        rpc_func!(self, "getrawtransaction", txid, verbose)
    }

    /// https://developer.bitcoin.org/reference/rpc/getrawtransaction.html
    /// Always returns transaction bytes
    pub fn get_raw_transaction_bytes(&self, txid: H256Json) -> RpcRes<BytesJson> {
        let verbose = 0;
        rpc_func!(self, "getrawtransaction", txid, verbose)
    }

    /// https://developer.bitcoin.org/reference/rpc/estimatefee.html
    /// Always estimate fee for transaction to be confirmed in next block
    fn estimate_fee(&self) -> RpcRes<f64> {
        let n_blocks = 1;
        rpc_func!(self, "estimatefee", n_blocks)
    }

    /// https://developer.bitcoin.org/reference/rpc/estimatesmartfee.html
    /// Always estimate fee for transaction to be confirmed in next block
    pub fn estimate_smart_fee(&self, mode: &Option<EstimateFeeMode>) -> RpcRes<EstimateSmartFeeRes> {
        let n_blocks = 1;
        match mode {
            Some(m) => rpc_func!(self, "estimatesmartfee", n_blocks, m),
            None => rpc_func!(self, "estimatesmartfee", n_blocks),
        }
    }

    /// https://developer.bitcoin.org/reference/rpc/listtransactions.html
    pub fn list_transactions(&self, count: u64, from: u64) -> RpcRes<Vec<ListTransactionsItem>> {
        let account = "*";
        let watch_only = true;
        rpc_func!(self, "listtransactions", account, count, from, watch_only)
    }

    /// https://developer.bitcoin.org/reference/rpc/listreceivedbyaddress.html
    pub fn list_received_by_address(
        &self,
        min_conf: u64,
        include_empty: bool,
        include_watch_only: bool,
    ) -> RpcRes<Vec<ReceivedByAddressItem>> {
        rpc_func!(
            self,
            "listreceivedbyaddress",
            min_conf,
            include_empty,
            include_watch_only
        )
    }

    pub fn detect_fee_method(&self) -> impl Future<Item = EstimateFeeMethod, Error = String> + Send {
        let estimate_fee_fut = self.estimate_fee();
        self.estimate_smart_fee(&None).then(move |res| -> Box<dyn Future<Item=EstimateFeeMethod, Error=String> + Send> {
            match res {
                Ok(smart_fee) => if smart_fee.fee_rate > 0. {
                    Box::new(futures01::future::ok(EstimateFeeMethod::SmartFee))
                } else {
                    log!("fee_rate from smart fee should be above zero, but got " [smart_fee] ", trying estimatefee");
                    Box::new(estimate_fee_fut.map_err(|e| ERRL!("{}", e)).and_then(|res| if res > 0. {
                        Ok(EstimateFeeMethod::Standard)
                    } else {
                        ERR!("Estimate fee result should be above zero, but got {}, consider setting txfee in config", res)
                    }))
                },
                Err(e) => {
                    log!("Error " (e) " on estimate smart fee, trying estimatefee");
                    Box::new(estimate_fee_fut.map_err(|e| ERRL!("{}", e)).and_then(|res| if res > 0. {
                        Ok(EstimateFeeMethod::Standard)
                    } else {
                        ERR!("Estimate fee result should be above zero, but got {}, consider setting txfee in config", res)
                    }))
                }
            }
        })
    }

    /// https://developer.bitcoin.org/reference/rpc/listsinceblock.html
    /// uses default target confirmations 1 and always includes watch_only addresses
    pub fn list_since_block(&self, block_hash: H256Json) -> RpcRes<ListSinceBlockRes> {
        let target_confirmations = 1;
        let include_watch_only = true;
        rpc_func!(
            self,
            "listsinceblock",
            block_hash,
            target_confirmations,
            include_watch_only
        )
    }

    /// https://developer.bitcoin.org/reference/rpc/getblockhash.html
    pub fn get_block_hash(&self, block_number: u64) -> RpcRes<H256Json> {
        rpc_func!(self, "getblockhash", block_number)
    }

    /// https://developer.bitcoin.org/reference/rpc/sendtoaddress.html
    pub fn send_to_address(&self, addr: &str, amount: &BigDecimal) -> RpcRes<H256Json> {
        rpc_func!(self, "sendtoaddress", addr, amount)
    }

    /// Returns the list of addresses assigned the specified label.
    /// https://developer.bitcoin.org/reference/rpc/getaddressesbylabel.html
    pub fn get_addresses_by_label(&self, label: &str) -> RpcRes<AddressesByLabelResult> {
        rpc_func!(self, "getaddressesbylabel", label)
    }

    /// https://developer.bitcoin.org/reference/rpc/getnetworkinfo.html
    pub fn get_network_info(&self) -> RpcRes<NetworkInfo> { rpc_func!(self, "getnetworkinfo") }

    /// https://developer.bitcoin.org/reference/rpc/getaddressinfo.html
    pub fn get_address_info(&self, address: &str) -> RpcRes<GetAddressInfoRes> {
        rpc_func!(self, "getaddressinfo", address)
    }
}

impl NativeClientImpl {
    /// Check whether input address is imported to daemon
    pub async fn is_address_imported(&self, address: &str) -> Result<bool, String> {
        let validate_res = try_s!(self.validate_address(address).compat().await);
        match (validate_res.is_mine, validate_res.is_watch_only) {
            (Some(is_mine), Some(is_watch_only)) => Ok(is_mine || is_watch_only),
            // ignoring (Some(_), None) and (None, Some(_)) variants, there seem to be no known daemons that return is_mine,
            // but do not return is_watch_only, so it's ok to fallback to getaddressinfo
            _ => {
                let address_info = try_s!(self.get_address_info(address).compat().await);
                Ok(address_info.is_mine || address_info.is_watch_only)
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ElectrumUnspent {
    height: Option<u64>,
    tx_hash: H256Json,
    tx_pos: u32,
    value: u64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ElectrumNonce {
    Number(u64),
    Hash(H256Json),
}

#[derive(Debug, Deserialize)]
pub struct ElectrumBlockHeadersRes {
    count: u64,
    pub hex: BytesJson,
    max: u64,
}

/// The block header compatible with Electrum 1.2
#[derive(Debug, Deserialize)]
pub struct ElectrumBlockHeaderV12 {
    bits: u64,
    block_height: u64,
    merkle_root: H256Json,
    nonce: ElectrumNonce,
    prev_block_hash: H256Json,
    timestamp: u64,
    version: u64,
}

/// The block header compatible with Electrum 1.4
#[derive(Debug, Deserialize)]
pub struct ElectrumBlockHeaderV14 {
    height: u64,
    hex: BytesJson,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ElectrumBlockHeader {
    V12(ElectrumBlockHeaderV12),
    V14(ElectrumBlockHeaderV14),
}

#[derive(Debug, Deserialize, Serialize)]
pub enum EstimateFeeMode {
    ECONOMICAL,
    CONSERVATIVE,
    UNSET,
}

impl ElectrumBlockHeader {
    fn block_height(&self) -> u64 {
        match self {
            ElectrumBlockHeader::V12(h) => h.block_height,
            ElectrumBlockHeader::V14(h) => h.height,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ElectrumTxHistoryItem {
    pub height: i64,
    pub tx_hash: H256Json,
    pub fee: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
struct ElectrumBalance {
    confirmed: i64,
    unconfirmed: i64,
}

fn sha_256(input: &[u8]) -> Vec<u8> {
    let mut sha = Sha256::new();
    sha.input(input);
    sha.result().to_vec()
}

pub fn electrum_script_hash(script: &[u8]) -> Vec<u8> {
    let mut result = sha_256(script);
    result.reverse();
    result
}

#[derive(Debug, Deserialize, Serialize)]
/// Deserializable Electrum protocol representation for RPC
pub enum ElectrumProtocol {
    /// TCP
    TCP,
    /// SSL/TLS
    SSL,
}

#[derive(Debug, Deserialize, Serialize)]
/// Deserializable Electrum protocol version representation for RPC
/// https://electrumx-spesmilo.readthedocs.io/en/latest/protocol-methods.html#server.version
pub struct ElectrumProtocolVersion {
    pub server_software_version: String,
    pub protocol_version: String,
}

#[derive(Debug, Deserialize, Serialize)]
/// Electrum request RPC representation
pub struct ElectrumRpcRequest {
    pub url: String,
    #[serde(default)]
    pub protocol: ElectrumProtocol,
    #[serde(default)]
    pub disable_cert_verification: bool,
}

impl Default for ElectrumProtocol {
    fn default() -> Self { ElectrumProtocol::TCP }
}

/// Electrum client configuration
#[derive(Clone, Debug, Serialize)]
enum ElectrumConfig {
    TCP,
    SSL { dns_name: String, skip_validation: bool },
}

fn addr_to_socket_addr(input: &str) -> Result<SocketAddr, String> {
    let mut addr = match input.to_socket_addrs() {
        Ok(a) => a,
        Err(e) => return ERR!("{} resolve error {:?}", input, e),
    };
    match addr.next() {
        Some(a) => Ok(a),
        None => ERR!("{} resolved to None.", input),
    }
}

/// Attempts to process the request (parse url, etc), build up the config and create new electrum connection
#[cfg(feature = "native")]
pub fn spawn_electrum(
    req: &ElectrumRpcRequest,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<ElectrumConnection, String> {
    let config = match req.protocol {
        ElectrumProtocol::TCP => ElectrumConfig::TCP,
        ElectrumProtocol::SSL => {
            let uri: Uri = try_s!(req.url.parse());
            let host = try_s!(uri.host().ok_or(ERRL!("Couldn't retrieve host from addr {}", req.url)));

            #[cfg(feature = "native")]
            fn check(host: &str) -> Result<(), String> {
                DNSNameRef::try_from_ascii_str(host)
                    .map(|_| ())
                    .map_err(|e| fomat!([e]))
            }
            #[cfg(not(feature = "native"))]
            fn check(_host: &str) -> Result<(), String> { Ok(()) }

            try_s!(check(host));

            ElectrumConfig::SSL {
                dns_name: host.into(),
                skip_validation: req.disable_cert_verification,
            }
        },
    };

    Ok(electrum_connect(req.url.clone(), config, event_handlers))
}

#[cfg(not(feature = "native"))]
#[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
extern "C" {
    fn host_electrum_connect(ptr: *const c_char, len: i32) -> i32;
    fn host_electrum_is_connected(ri: i32) -> i32;
    fn host_electrum_request(ri: i32, ptr: *const c_char, len: i32) -> i32;
    fn host_electrum_reply(ri: i32, id: i32, rbuf: *mut c_char, rcap: i32) -> i32;
}

#[cfg(not(feature = "native"))]
pub fn spawn_electrum(
    req: &ElectrumRpcRequest,
    _event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<ElectrumConnection, String> {
    use std::net::{IpAddr, Ipv4Addr};

    let args = unwrap!(json::to_vec(req));
    let rc = unsafe { host_electrum_connect(args.as_ptr() as *const c_char, args.len() as i32) };
    if rc < 0 {
        panic!("!host_electrum_connect: {}", rc)
    }
    let ri = rc; // Random ID assigned by the host to connection.

    let responses = Arc::new(Mutex::new(HashMap::new()));
    let tx = Arc::new(AsyncMutex::new(None));

    let config = match req.protocol {
        ElectrumProtocol::TCP => ElectrumConfig::TCP,
        ElectrumProtocol::SSL => {
            let uri: Uri = try_s!(req.url.parse());
            let host = try_s!(uri.host().ok_or("!host"));
            ElectrumConfig::SSL {
                dns_name: host.into(),
                skip_validation: req.disable_cert_verification,
            }
        },
    };

    Ok(ElectrumConnection {
        addr: req.url.clone(),
        config,
        tx,
        shutdown_tx: None,
        responses,
        ri,
        protocol_version: AsyncMutex::new(None),
    })
}

#[derive(Debug)]
/// Represents the active Electrum connection to selected address
pub struct ElectrumConnection {
    /// The client connected to this SocketAddr
    addr: String,
    /// Configuration
    config: ElectrumConfig,
    /// The Sender forwarding requests to writing part of underlying stream
    tx: Arc<AsyncMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    /// The Sender used to shutdown the background connection loop when ElectrumConnection is dropped
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Responses are stored here
    responses: Arc<AsyncMutex<HashMap<String, async_oneshot::Sender<JsonRpcResponse>>>>,
    /// [Random] connection ID assigned by the WASM host
    ri: i32,
    /// Selected protocol version. The value is initialized after the server.version RPC call.
    protocol_version: AsyncMutex<Option<f32>>,
}

impl ElectrumConnection {
    #[cfg(feature = "native")]
    async fn is_connected(&self) -> bool { self.tx.lock().await.is_some() }

    #[cfg(not(feature = "native"))]
    async fn is_connected(&self) -> bool {
        let rc = unsafe { host_electrum_is_connected(self.ri) };
        if rc < 0 {
            panic!("!host_electrum_is_connected: {}", rc)
        }
        //log! ("is_connected] host_electrum_is_connected (" [=self.ri] ") " [=rc]);
        if rc == 1 {
            true
        } else {
            false
        }
    }

    async fn set_protocol_version(&self, version: f32) { self.protocol_version.lock().await.replace(version); }
}

impl Drop for ElectrumConnection {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            if shutdown_tx.send(()).is_err() {
                log!("electrum_connection_drop] Warning, shutdown_tx already closed");
            }
        }
    }
}

#[derive(Debug)]
pub struct RpcRequestState<T> {
    is_request_running: Arc<AtomicBool>,
    response_subs: Vec<oneshot::Sender<Result<T, JsonRpcError>>>,
}

#[derive(Debug)]
pub struct RpcRequestWrapper<Key, Response> {
    inner: HashMap<Key, RpcRequestState<Response>>,
}

impl<Key, Response> RpcRequestWrapper<Key, Response> {
    fn new() -> Self { RpcRequestWrapper { inner: HashMap::new() } }

    #[allow(dead_code)]
    async fn wrap_request(
        &mut self,
        _key: Key,
        _request: impl Future<Item = Response, Error = JsonRpcError>,
    ) -> Result<Response, JsonRpcError> {
        unimplemented!()
    }
}

#[derive(Debug)]
pub struct ElectrumClientImpl {
    coin_ticker: String,
    connections: AsyncMutex<Vec<ElectrumConnection>>,
    next_id: AtomicU64,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    protocol_version: OrdRange<f32>,
    get_balance_wrapper: RpcRequestWrapper<Address, ElectrumBalance>,
    list_unspent_wrapper: RpcRequestWrapper<Address, Vec<ElectrumUnspent>>,
    list_unspent_in_progress: AtomicBool,
    list_unspent_subs: AsyncMutex<Vec<RpcReqSub<Vec<ElectrumUnspent>>>>,
    get_balance_in_progress: AtomicBool,
    get_balance_subs: AsyncMutex<Vec<async_oneshot::Sender<Result<ElectrumBalance, JsonRpcError>>>>,
}

#[cfg(feature = "native")]
async fn electrum_request_multi(
    client: ElectrumClient,
    request: JsonRpcRequest,
) -> Result<(JsonRpcRemoteAddr, JsonRpcResponse), String> {
    let mut futures = vec![];
    for connection in client.connections.lock().await.iter() {
        let connection_addr = connection.addr.clone();
        match &*connection.tx.lock().await {
            Some(tx) => {
                let fut = electrum_request(request.clone(), tx.clone(), connection.responses.clone())
                    .map(|response| (JsonRpcRemoteAddr(connection_addr), response));
                futures.push(fut)
            },
            None => (),
        }
    }
    if futures.is_empty() {
        return ERR!("All electrums are currently disconnected");
    }
    if request.method != "server.ping" {
        Ok(try_s!(
            select_ok_sequential(futures)
                .map_err(|e| ERRL!("{:?}", e))
                .compat()
                .await
        ))
    } else {
        // server.ping must be sent to all servers to keep all connections alive
        Ok(try_s!(
            select_ok(futures)
                .map(|(result, _)| result)
                .map_err(|e| ERRL!("{:?}", e))
                .compat()
                .await
        ))
    }
}

#[cfg(feature = "native")]
async fn electrum_request_to(
    client: ElectrumClient,
    request: JsonRpcRequest,
    to_addr: String,
) -> Result<(JsonRpcRemoteAddr, JsonRpcResponse), String> {
    let (tx, responses) = {
        let connections = client.connections.lock().await;
        let connection = connections
            .iter()
            .find(|c| c.addr == to_addr)
            .ok_or(ERRL!("Unknown destination address {}", to_addr))?;
        let responses = connection.responses.clone();
        let tx = {
            match &*connection.tx.lock().await {
                Some(tx) => tx.clone(),
                None => return ERR!("Connection {} is not established yet", to_addr),
            }
        };
        (tx, responses)
    };

    let response = try_s!(electrum_request(request.clone(), tx, responses).compat().await);
    Ok((JsonRpcRemoteAddr(to_addr.to_owned()), response))
}

#[cfg(not(feature = "native"))]
lazy_static! {
    static ref ELECTRUM_REPLIES: Mutex<HashMap<(i32, i32), ShotSender<()>>> = Mutex::new(HashMap::new());
}

#[no_mangle]
#[cfg(not(feature = "native"))]
pub extern "C" fn electrum_replied(ri: i32, id: i32) {
    //log! ("electrum_replied] " [=ri] ", " [=id]);
    let mut electrum_replies = unwrap!(ELECTRUM_REPLIES.lock());
    if let Some(tx) = electrum_replies.remove(&(ri, id)) {
        let _ = tx.send(());
    }
}

/// AG: As of now the pings tend to fail.
///     I haven't looked into this because we'll probably use a websocket or Java implementation instead.
#[cfg(not(feature = "native"))]
async fn electrum_request_multi(
    client: ElectrumClient,
    request: JsonRpcRequest,
) -> Result<(JsonRpcRemoteAddr, JsonRpcResponse), String> {
    use futures::future::{select, Either};
    use std::mem::MaybeUninit;
    use std::os::raw::c_char;
    use std::str::from_utf8;

    let req = try_s!(json::to_string(&request));
    let id: i32 = try_s!(request.id.parse());
    let mut jres: Option<JsonRpcResponse> = None;
    // address of server from which an Rpc response was received
    let mut remote_address = JsonRpcRemoteAddr::default();

    for connection in client.connections.lock().await.iter() {
        let (tx, rx) = futures::channel::oneshot::channel();
        try_s!(ELECTRUM_REPLIES.lock()).insert((connection.ri, id), tx);
        let rc = unsafe { host_electrum_request(connection.ri, req.as_ptr() as *const c_char, req.len() as i32) };
        if rc != 0 {
            return ERR!("!host_electrum_request: {}", rc);
        }

        // Wait for the host to invoke `fn electrum_replied`.
        let timeout = Timer::sleep(10.);
        let rc = select(rx, timeout).await;
        match rc {
            Either::Left((_r, _t)) => (),
            Either::Right((_t, _r)) => {
                log! ("Electrum " (connection.ri) " timeout");
                continue;
            },
        };

        let mut buf: [u8; 131072] = unsafe { MaybeUninit::uninit().assume_init() };
        let rc = unsafe { host_electrum_reply(connection.ri, id, buf.as_mut_ptr() as *mut c_char, buf.len() as i32) };
        if rc <= 0 {
            log!("!host_electrum_reply: "(rc));
            continue;
        } // Skip to the next connection.
        let res = try_s!(from_utf8(&buf[0..rc as usize]));
        //log! ("electrum_request_multi] ri " (connection.ri) ", res: " (res));
        let res: Json = try_s!(json::from_str(res));
        // TODO: Detect errors and fill the `error` field somehow?
        jres = Some(JsonRpcResponse {
            jsonrpc: req.clone(),
            id: request.id.clone(),
            result: res,
            error: Json::Null,
        });
        remote_address = JsonRpcRemoteAddr(connection.addr.clone());
        // server.ping must be sent to all servers to keep all connections alive
        if request.method != "server.ping" {
            break;
        }
    }
    let jres = try_s!(jres.ok_or("!jres"));
    Ok((remote_address, jres))
}

impl ElectrumClientImpl {
    /// Create an Electrum connection and spawn a green thread actor to handle it.
    pub async fn add_server(&self, req: &ElectrumRpcRequest) -> Result<(), String> {
        let connection = try_s!(spawn_electrum(req, self.event_handlers.clone()));
        self.connections.lock().await.push(connection);
        Ok(())
    }

    /// Remove an Electrum connection and stop corresponding spawned actor.
    pub async fn remove_server(&self, server_addr: &str) -> Result<(), String> {
        let mut connections = self.connections.lock().await;
        // do not use retain, we would have to return an error if we did not find connection by the passd address
        let pos = connections
            .iter()
            .position(|con| con.addr == server_addr)
            .ok_or(ERRL!("Unknown electrum address {}", server_addr))?;
        // shutdown_tx will be closed immediately on the connection drop
        connections.remove(pos);
        Ok(())
    }

    /// Check if one of the spawned connections is connected.
    pub async fn is_connected(&self) -> bool {
        for connection in self.connections.lock().await.iter() {
            if connection.is_connected().await {
                return true;
            }
        }
        false
    }

    pub async fn count_connections(&self) -> usize { self.connections.lock().await.len() }

    /// Check if the protocol version was checked for one of the spawned connections.
    pub async fn is_protocol_version_checked(&self) -> bool {
        for connection in self.connections.lock().await.iter() {
            if connection.protocol_version.lock().await.is_some() {
                return true;
            }
        }
        false
    }

    /// Set the protocol version for the specified server.
    pub async fn set_protocol_version(&self, server_addr: &str, version: f32) -> Result<(), String> {
        let connections = self.connections.lock().await;
        let con = connections
            .iter()
            .find(|con| con.addr == server_addr)
            .ok_or(ERRL!("Unknown electrum address {}", server_addr))?;
        con.set_protocol_version(version).await;
        Ok(())
    }

    /// Get available protocol versions.
    pub fn protocol_version(&self) -> &OrdRange<f32> { &self.protocol_version }
}

#[derive(Clone, Debug)]
pub struct ElectrumClient(pub Arc<ElectrumClientImpl>);
impl Deref for ElectrumClient {
    type Target = ElectrumClientImpl;
    fn deref(&self) -> &ElectrumClientImpl { &*self.0 }
}

const BLOCKCHAIN_HEADERS_SUB_ID: &str = "blockchain.headers.subscribe";

impl UtxoJsonRpcClientInfo for ElectrumClient {
    fn coin_name(&self) -> &str { self.coin_ticker.as_str() }
}

impl JsonRpcClient for ElectrumClient {
    fn version(&self) -> &'static str { "2.0" }

    fn next_id(&self) -> String { self.next_id.fetch_add(1, AtomicOrdering::Relaxed).to_string() }

    fn client_info(&self) -> String { UtxoJsonRpcClientInfo::client_info(self) }

    fn transport(&self, request: JsonRpcRequest) -> JsonRpcResponseFut {
        Box::new(electrum_request_multi(self.clone(), request).boxed().compat())
    }
}

impl JsonRpcMultiClient for ElectrumClient {
    fn transport_exact(&self, to_addr: String, request: JsonRpcRequest) -> JsonRpcResponseFut {
        Box::new(electrum_request_to(self.clone(), request, to_addr).boxed().compat())
    }
}

impl ElectrumClient {
    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#server-ping
    pub fn server_ping(&self) -> RpcRes<()> { rpc_func!(self, "server.ping") }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#server-version
    pub fn server_version(
        &self,
        server_address: &str,
        client_name: &str,
        version: &OrdRange<f32>,
    ) -> RpcRes<ElectrumProtocolVersion> {
        let protocol_version: Vec<String> = version.flatten().into_iter().map(|v| format!("{}", v)).collect();
        rpc_func_from!(self, server_address, "server.version", client_name, protocol_version)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-listunspent
    /// It can return duplicates sometimes: https://github.com/artemii235/SuperNET/issues/269
    /// We should remove them to build valid transactions
    fn scripthash_list_unspent(&self, hash: &str) -> RpcRes<Vec<ElectrumUnspent>> {
        let arc = self.clone();
        let hash = hash.to_owned();
        if self
            .list_unspent_in_progress
            .compare_and_swap(false, true, AtomicOrdering::Relaxed)
        {
            let fut = async move {
                let (tx, rx) = async_oneshot::channel();
                arc.list_unspent_subs.lock().await.push(tx);
                rx.await.unwrap()
            };
            Box::new(fut.boxed().compat())
        } else {
            let fut = async move {
                let unspents_res = rpc_func!(arc, "blockchain.scripthash.listunspent", hash)
                    .and_then(move |unspents: Vec<ElectrumUnspent>| {
                        let mut map: HashMap<(H256Json, u32), bool> = HashMap::new();
                        let unspents = unspents
                            .into_iter()
                            .filter(|unspent| match map.entry((unspent.tx_hash.clone(), unspent.tx_pos)) {
                                Entry::Occupied(_) => false,
                                Entry::Vacant(e) => {
                                    e.insert(true);
                                    true
                                },
                            })
                            .collect();
                        Ok(unspents)
                    })
                    .compat()
                    .await;
                for sub in arc.list_unspent_subs.lock().await.drain(..) {
                    if sub.send(unspents_res.clone()).is_err() {
                        log!("list_unspent_sub is dropped");
                    }
                }
                arc.list_unspent_in_progress.store(false, AtomicOrdering::Relaxed);
                unspents_res
            };
            Box::new(fut.boxed().compat())
        }
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-get-history
    pub fn scripthash_get_history(&self, hash: &str) -> RpcRes<Vec<ElectrumTxHistoryItem>> {
        rpc_func!(self, "blockchain.scripthash.get_history", hash)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-gethistory
    fn scripthash_get_balance(&self, hash: &str) -> RpcRes<ElectrumBalance> {
        let arc = self.clone();
        let hash = hash.to_owned();
        if self
            .get_balance_in_progress
            .compare_and_swap(false, true, AtomicOrdering::Relaxed)
        {
            let fut = async move {
                let (tx, rx) = async_oneshot::channel();
                arc.get_balance_subs.lock().await.push(tx);
                rx.await.unwrap()
            };
            Box::new(fut.boxed().compat())
        } else {
            let fut = async move {
                let balance_res = rpc_func!(arc, "blockchain.scripthash.get_balance", hash).compat().await;
                for sub in arc.get_balance_subs.lock().await.drain(..) {
                    if sub.send(balance_res.clone()).is_err() {
                        log!("list_unspent_sub is dropped");
                    }
                }
                arc.get_balance_in_progress.store(false, AtomicOrdering::Relaxed);
                balance_res
            };
            Box::new(fut.boxed().compat())
        }
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-headers-subscribe
    pub fn blockchain_headers_subscribe(&self) -> RpcRes<ElectrumBlockHeader> {
        rpc_func!(self, "blockchain.headers.subscribe")
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-broadcast
    fn blockchain_transaction_broadcast(&self, tx: BytesJson) -> RpcRes<H256Json> {
        rpc_func!(self, "blockchain.transaction.broadcast", tx)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-estimatefee
    /// Always estimate fee for transaction to be confirmed in next block
    fn estimate_fee(&self, mode: &Option<EstimateFeeMode>) -> RpcRes<f64> {
        let n_blocks = 1;
        match mode {
            Some(m) => rpc_func!(self, "blockchain.estimatefee", n_blocks, m),
            None => rpc_func!(self, "blockchain.estimatefee", n_blocks),
        }
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-block-headers
    pub fn blockchain_block_headers(&self, start_height: u64, count: NonZeroU64) -> RpcRes<ElectrumBlockHeadersRes> {
        rpc_func!(self, "blockchain.block.headers", start_height, count)
    }
}

#[cfg_attr(test, mockable)]
impl UtxoRpcClientOps for ElectrumClient {
    fn list_unspent(&self, address: &Address, _decimals: u8) -> UtxoRpcRes<Vec<UnspentInfo>> {
        let script = Builder::build_p2pkh(&address.hash);
        let script_hash = electrum_script_hash(&script);
        Box::new(
            self.scripthash_list_unspent(&hex::encode(script_hash))
                .map_err(|e| ERRL!("{}", e))
                .map(move |unspents| {
                    unspents
                        .iter()
                        .map(|unspent| UnspentInfo {
                            outpoint: OutPoint {
                                hash: unspent.tx_hash.reversed().into(),
                                index: unspent.tx_pos,
                            },
                            value: unspent.value,
                            height: unspent.height,
                        })
                        .collect()
                }),
        )
    }

    fn send_transaction(&self, tx: &UtxoTx) -> UtxoRpcRes<H256Json> {
        let bytes = BytesJson::from(serialize(tx));
        Box::new(self.blockchain_transaction_broadcast(bytes).map_err(|e| ERRL!("{}", e)))
    }

    fn send_raw_transaction(&self, tx: BytesJson) -> RpcRes<H256Json> { self.blockchain_transaction_broadcast(tx) }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-get
    /// returns transaction bytes by default
    fn get_transaction_bytes(&self, txid: H256Json) -> RpcRes<BytesJson> {
        let verbose = false;
        rpc_func!(self, "blockchain.transaction.get", txid, verbose)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-get
    /// returns verbose transaction by default
    fn get_verbose_transaction(&self, txid: H256Json) -> RpcRes<RpcTransaction> {
        let verbose = true;
        rpc_func!(self, "blockchain.transaction.get", txid, verbose)
    }

    fn get_block_count(&self) -> RpcRes<u64> { Box::new(self.blockchain_headers_subscribe().map(|r| r.block_height())) }

    fn display_balance(&self, address: Address, decimals: u8) -> RpcRes<BigDecimal> {
        let hash = electrum_script_hash(&Builder::build_p2pkh(&address.hash));
        let hash_str = hex::encode(hash);
        Box::new(self.scripthash_get_balance(&hash_str).map(move |result| {
            BigDecimal::from(result.confirmed + result.unconfirmed) / BigDecimal::from(10u64.pow(decimals as u32))
        }))
    }

    fn estimate_fee_sat(
        &self,
        decimals: u8,
        _fee_method: &EstimateFeeMethod,
        mode: &Option<EstimateFeeMode>,
    ) -> RpcRes<u64> {
        Box::new(self.estimate_fee(mode).map(move |fee| {
            if fee > 0.00001 {
                (fee * 10.0_f64.powf(decimals as f64)) as u64
            } else {
                1000
            }
        }))
    }

    fn get_relay_fee(&self) -> RpcRes<BigDecimal> { rpc_func!(self, "blockchain.relayfee") }

    fn find_output_spend(
        &self,
        tx: &UtxoTx,
        vout: usize,
        _from_block: u64,
    ) -> Box<dyn Future<Item = Option<UtxoTx>, Error = String> + Send> {
        let selfi = self.clone();
        let script_hash = hex::encode(electrum_script_hash(&tx.outputs[vout].script_pubkey));
        let tx = tx.clone();
        let fut = async move {
            let history = try_s!(selfi.scripthash_get_history(&script_hash).compat().await);

            if history.len() < 2 {
                return Ok(None);
            }

            for item in history.iter() {
                let transaction = try_s!(selfi.get_transaction_bytes(item.tx_hash.clone()).compat().await);

                let maybe_spend_tx: UtxoTx = try_s!(deserialize(transaction.as_slice()).map_err(|e| ERRL!("{:?}", e)));

                for input in maybe_spend_tx.inputs.iter() {
                    if input.previous_output.hash == tx.hash() && input.previous_output.index == vout as u32 {
                        return Ok(Some(maybe_spend_tx));
                    }
                }
            }
            Ok(None)
        };
        Box::new(fut.boxed().compat())
    }

    fn get_median_time_past(
        &self,
        starting_block: u64,
        count: NonZeroU64,
    ) -> Box<dyn Future<Item = u32, Error = String> + Send> {
        let from = if starting_block <= count.get() {
            0
        } else {
            starting_block - count.get() + 1
        };
        Box::new(
            self.blockchain_block_headers(from, count)
                .map_err(|e| ERRL!("{}", e))
                .and_then(|res| {
                    if res.count == 0 {
                        return ERR!("Server returned zero count");
                    }
                    let len = CompactInteger::from(res.count);
                    let mut serialized = serialize(&len).take();
                    serialized.extend(res.hex.0.into_iter());
                    let mut reader = Reader::new(serialized.as_slice());
                    let headers = try_s!(reader.read_list::<BlockHeader>().map_err(|e| ERRL!("{:?}", e)));
                    let mut timestamps: Vec<_> = headers.into_iter().map(|block| block.time).collect();
                    // can unwrap because count is non zero
                    Ok(median(timestamps.as_mut_slice()).unwrap())
                }),
        )
    }
}

#[cfg_attr(test, mockable)]
impl ElectrumClientImpl {
    pub fn new(coin_ticker: String, event_handlers: Vec<RpcTransportEventHandlerShared>) -> ElectrumClientImpl {
        let protocol_version = OrdRange::new(1.2, 1.4).unwrap();
        ElectrumClientImpl {
            coin_ticker,
            connections: AsyncMutex::new(vec![]),
            next_id: 0.into(),
            event_handlers,
            protocol_version,
            get_balance_wrapper: RpcRequestWrapper::new(),
            list_unspent_wrapper: RpcRequestWrapper::new(),
            list_unspent_in_progress: Default::default(),
            list_unspent_subs: Default::default(),
            get_balance_in_progress: Default::default(),
            get_balance_subs: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn with_protocol_version(
        coin_ticker: String,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
        protocol_version: OrdRange<f32>,
    ) -> ElectrumClientImpl {
        ElectrumClientImpl {
            protocol_version,
            ..ElectrumClientImpl::new(coin_ticker, event_handlers)
        }
    }
}

/// Helper function casting mpsc::Receiver as Stream.
fn rx_to_stream(rx: mpsc::Receiver<Vec<u8>>) -> impl Stream<Item = Vec<u8>, Error = io::Error> {
    rx.map_err(|_| panic!("errors not possible on rx"))
}

async fn electrum_process_chunk(
    chunk: &[u8],
    arc: Arc<AsyncMutex<HashMap<String, async_oneshot::Sender<JsonRpcResponse>>>>,
) {
    // we should split the received chunk because we can get several responses in 1 chunk.
    let split = chunk.split(|item| *item == b'\n');
    for chunk in split {
        // split returns empty slice if it ends with separator which is our case
        if !chunk.is_empty() {
            let raw_json: Json = match json::from_slice(chunk) {
                Ok(json) => json,
                Err(e) => {
                    log!([e]);
                    return;
                },
            };

            // detect if we got standard JSONRPC response or subscription response as JSONRPC request
            if raw_json["method"].is_null() && raw_json["params"].is_null() {
                let response: JsonRpcResponse = match json::from_value(raw_json) {
                    Ok(res) => res,
                    Err(e) => {
                        log!([e]);
                        return;
                    },
                };
                let mut resp = arc.lock().await;
                // the corresponding sender may not exist, receiver may be dropped
                // these situations are not considered as errors so we just silently skip them
                if let Some(tx) = resp.remove(&response.id.to_string()) {
                    tx.send(response).unwrap_or(())
                }
                drop(resp);
            } else {
                let request: JsonRpcRequest = match json::from_value(raw_json) {
                    Ok(res) => res,
                    Err(e) => {
                        log!([e]);
                        return;
                    },
                };
                let id = match request.method.as_ref() {
                    BLOCKCHAIN_HEADERS_SUB_ID => BLOCKCHAIN_HEADERS_SUB_ID,
                    _ => {
                        log!("Couldn't get id of request "[request]);
                        return;
                    },
                };

                let response = JsonRpcResponse {
                    id: id.into(),
                    jsonrpc: "2.0".into(),
                    result: request.params[0].clone(),
                    error: Json::Null,
                };
                let mut resp = arc.lock().await;
                // the corresponding sender may not exist, receiver may be dropped
                // these situations are not considered as errors so we just silently skip them
                if let Some(tx) = resp.remove(&response.id.to_string()) {
                    tx.send(response).unwrap_or(())
                }
                drop(resp);
            }
        }
    }
}

macro_rules! try_loop {
    ($e:expr, $addr: ident, $delay: ident) => {
        match $e {
            Ok(res) => res,
            Err(e) => {
                log!([$addr] " error " [e]);
                if $delay < 30 {
                    $delay += 5;
                }
                continue;
            }
        }
    };
}

/// The enum wrapping possible variants of underlying Streams
#[cfg(feature = "native")]
#[allow(clippy::large_enum_variant)]
enum ElectrumStream {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
}

#[cfg(feature = "native")]
impl AsRef<TcpStream> for ElectrumStream {
    fn as_ref(&self) -> &TcpStream {
        match self {
            ElectrumStream::Tcp(stream) => stream,
            ElectrumStream::Tls(stream) => stream.get_ref().0,
        }
    }
}

impl AsyncRead for ElectrumStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            ElectrumStream::Tcp(stream) => AsyncRead::poll_read(Pin::new(stream), cx, buf),
            ElectrumStream::Tls(stream) => AsyncRead::poll_read(Pin::new(stream), cx, buf),
        }
    }
}

impl AsyncWrite for ElectrumStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            ElectrumStream::Tcp(stream) => AsyncWrite::poll_write(Pin::new(stream), cx, buf),
            ElectrumStream::Tls(stream) => AsyncWrite::poll_write(Pin::new(stream), cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        match self.get_mut() {
            ElectrumStream::Tcp(stream) => AsyncWrite::poll_flush(Pin::new(stream), cx),
            ElectrumStream::Tls(stream) => AsyncWrite::poll_flush(Pin::new(stream), cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        match self.get_mut() {
            ElectrumStream::Tcp(stream) => AsyncWrite::poll_shutdown(Pin::new(stream), cx),
            ElectrumStream::Tls(stream) => AsyncWrite::poll_shutdown(Pin::new(stream), cx),
        }
    }
}

const ELECTRUM_TIMEOUT: u64 = 60;

async fn electrum_last_chunk_loop(last_chunk: Arc<AtomicU64>) {
    loop {
        Timer::sleep(ELECTRUM_TIMEOUT as f64).await;
        let last = (last_chunk.load(AtomicOrdering::Relaxed) / 1000) as f64;
        if now_float() - last > ELECTRUM_TIMEOUT as f64 {
            log!("Didn't receive any data since " (last as i64) ". Shutting down the connection.");
            break;
        }
    }
}

#[cfg(feature = "native")]
async fn connect_loop(
    config: ElectrumConfig,
    addr: String,
    responses: Arc<AsyncMutex<HashMap<String, async_oneshot::Sender<JsonRpcResponse>>>>,
    connection_tx: Arc<AsyncMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<(), ()> {
    let mut delay: u64 = 0;

    loop {
        if delay > 0 {
            Timer::sleep(delay as f64).await;
        };

        let socket_addr = try_loop!(addr_to_socket_addr(&addr), addr, delay);

        let connect_f = match config.clone() {
            ElectrumConfig::TCP => Either::Left(TcpStream::connect(&socket_addr).map_ok(ElectrumStream::Tcp)),
            ElectrumConfig::SSL {
                dns_name,
                skip_validation,
            } => {
                let mut ssl_config = rustls::ClientConfig::new();
                ssl_config.root_store.add_server_trust_anchors(&TLS_SERVER_ROOTS);
                if skip_validation {
                    ssl_config
                        .dangerous()
                        .set_certificate_verifier(Arc::new(NoCertificateVerification {}));
                }
                let tls_connector = TlsConnector::from(Arc::new(ssl_config));

                Either::Right(TcpStream::connect(&socket_addr).and_then(move |stream| {
                    // Can use `unwrap` cause `dns_name` is pre-checked.
                    let dns = unwrap!(DNSNameRef::try_from_ascii_str(&dns_name).map_err(|e| fomat!([e])));
                    tls_connector.connect(dns, stream).map_ok(ElectrumStream::Tls)
                }))
            },
        };

        let stream = try_loop!(connect_f.await, addr, delay);
        try_loop!(stream.as_ref().set_nodelay(true), addr, delay);
        // reset the delay if we've connected successfully
        delay = 0;
        log!("Electrum client connected to "(addr));
        try_loop!(event_handlers.on_connected(addr.clone()), addr, delay);
        let last_chunk = Arc::new(AtomicU64::new(now_ms()));
        let mut last_chunk_f = electrum_last_chunk_loop(last_chunk.clone()).boxed().fuse();

        let (tx, rx) = mpsc::channel(0);
        *connection_tx.lock().await = Some(tx);
        let rx = rx_to_stream(rx).inspect(|data| {
            // measure the length of each sent packet
            event_handlers.on_outgoing_request(&data);
        });

        let (read, mut write) = tokio::io::split(stream);
        let recv_f = {
            let addr = addr.clone();
            let responses = responses.clone();
            async move {
                let mut buffer = String::with_capacity(1024);
                let mut buf_reader = BufReader::new(read);
                loop {
                    match buf_reader.read_line(&mut buffer).await {
                        Ok(c) => {
                            if c == 0 {
                                log!("EOF from"(addr));
                                break;
                            }
                        },
                        Err(e) => {
                            log!("Error on read "(e) " from "(addr));
                            break;
                        },
                    };
                    last_chunk.store(now_ms(), AtomicOrdering::Relaxed);
                    electrum_process_chunk(buffer.as_bytes(), responses.clone()).await;
                    buffer.clear();
                }
            }
        };
        let mut recv_f = Box::pin(recv_f).fuse();

        let send_f = {
            let addr = addr.clone();
            let mut rx = rx.compat();
            async move {
                while let Some(Ok(bytes)) = rx.next().await {
                    if let Err(e) = write.write_all(&bytes).await {
                        log!("Write error "(e) " to " (addr));
                    }
                }
            }
        };
        let mut send_f = Box::pin(send_f).fuse();
        macro_rules! reset_tx_and_continue {
            () => {
                log!([addr] " connection dropped");
                *connection_tx.lock().await = None;
                continue;
            };
        }

        select! {
            last_chunk = last_chunk_f => { reset_tx_and_continue!(); },
            recv = recv_f => { reset_tx_and_continue!(); },
            send = send_f => { reset_tx_and_continue!(); },
        }
    }
}

#[cfg(not(feature = "native"))]
async fn connect_loop(
    _config: ElectrumConfig,
    _addr: SocketAddr,
    _responses: Arc<Mutex<HashMap<String, JsonRpcResponse>>>,
    _connection_tx: Arc<AsyncMutex<Option<mpsc::Sender<Vec<u8>>>>>,
) -> Result<(), ()> {
    unimplemented!()
}

/// Builds up the electrum connection, spawns endless loop that attempts to reconnect to the server
/// in case of connection errors
#[cfg(feature = "native")]
fn electrum_connect(
    addr: String,
    config: ElectrumConfig,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> ElectrumConnection {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let responses = Arc::new(AsyncMutex::new(HashMap::new()));
    let tx = Arc::new(AsyncMutex::new(None));

    let connect_loop = connect_loop(
        config.clone(),
        addr.clone(),
        responses.clone(),
        tx.clone(),
        event_handlers,
    );

    let connect_loop = select_func(connect_loop.boxed(), shutdown_rx.compat());
    spawn(connect_loop.map(|_| ()));
    ElectrumConnection {
        addr,
        config,
        tx,
        shutdown_tx: Some(shutdown_tx),
        responses,
        ri: -1,
        protocol_version: AsyncMutex::new(None),
    }
}

#[cfg(not(feature = "native"))]
fn electrum_connect(
    _addr: SocketAddr,
    _config: ElectrumConfig,
    _event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> ElectrumConnection {
    unimplemented!()
}

fn electrum_request(
    request: JsonRpcRequest,
    tx: mpsc::Sender<Vec<u8>>,
    responses: Arc<AsyncMutex<HashMap<String, async_oneshot::Sender<JsonRpcResponse>>>>,
) -> Box<dyn Future<Item = JsonRpcResponse, Error = String> + Send + 'static> {
    let send_fut = async move {
        let mut json = try_s!(json::to_string(&request));
        // Electrum request and responses must end with \n
        // https://electrumx.readthedocs.io/en/latest/protocol-basics.html#message-stream
        json.push('\n');
        let request_id = request.get_id().to_string();
        let (req_tx, resp_rx) = async_oneshot::channel();
        responses.lock().await.insert(request_id, req_tx);
        try_s!(tx.send(json.into_bytes()).compat().await);
        let response = try_s!(resp_rx.await);
        Ok(response)
    };
    let send_fut = send_fut
        .boxed()
        .compat()
        .map_err(StringError)
        .timeout(Duration::from_secs(ELECTRUM_TIMEOUT));

    Box::new(send_fut.map_err(|e| ERRL!("{}", e.0)))
}

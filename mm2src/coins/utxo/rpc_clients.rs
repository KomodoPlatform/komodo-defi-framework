use bigdecimal::BigDecimal;
use bytes::{BytesMut};
use chain::{OutPoint, Transaction as UtxoTransaction};
use common::StringError;
use common::wio::{slurp_req, CORE};
use common::custom_futures::{join_all_sequential, select_ok_sequential, SendAll};
use common::jsonrpc_client::{JsonRpcClient, JsonRpcResponseFut, JsonRpcRequest, JsonRpcResponse, RpcRes};
use futures::{Async, Future, Poll, Sink, Stream};
use futures::future::{Either, loop_fn, Loop, select_ok};
use futures::sync::mpsc;
use futures_timer::{Delay, Interval, FutureExt};
use gstuff::now_ms;
use hashbrown::HashMap;
use hashbrown::hash_map::Entry;
use http::{Request, StatusCode};
use http::header::AUTHORIZATION;
use http::Uri;
use keys::Address;
#[cfg(test)]
use mocktopus::macros::*;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json, Transaction as RpcTransaction, VerboseBlockClient};
use rustls::{self, ClientConfig, Session};
use script::{Builder};
use serde_json::{self as json, Value as Json};
use serialization::{serialize, deserialize};
use sha2::{Sha256, Digest};
use std::{io, thread};
use std::fmt::Debug;
use std::cmp::Ordering;
use std::net::{ToSocketAddrs, SocketAddr, Shutdown};
use std::ops::Deref;
use std::sync::{Mutex, Arc};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration};
use tokio::codec::{Encoder, Decoder};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_rustls::{TlsConnector, TlsStream};
use tokio_rustls::webpki::{DNSName, DNSNameRef};
use tokio_tcp::TcpStream;
use webpki_roots::TLS_SERVER_ROOTS;

/// Skips the server certificate verification on TLS connection
pub struct NoCertificateVerification {}

impl rustls::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(&self,
                          _roots: &rustls::RootCertStore,
                          _presented_certs: &[rustls::Certificate],
                          _dns_name: DNSNameRef<'_>,
                          _ocsp: &[u8]) -> Result<rustls::ServerCertVerified, rustls::TLSError> {
        Ok(rustls::ServerCertVerified::assertion())
    }
}

#[derive(Debug)]
pub enum UtxoRpcClientEnum {
    Native(NativeClient),
    Electrum(ElectrumClient),
}

impl Deref for UtxoRpcClientEnum {
    type Target = dyn UtxoRpcClientOps;
    fn deref(&self) -> &dyn UtxoRpcClientOps {
        match self {
            &UtxoRpcClientEnum::Native(ref c) => c,
            &UtxoRpcClientEnum::Electrum(ref c) => c,
        }
    }
}

/// Generic unspent info required to build transactions, we need this separate type because native
/// and Electrum provide different list_unspent format.
#[derive(Debug)]
pub struct UnspentInfo {
    pub outpoint: OutPoint,
    pub value: u64,
}

pub type UtxoRpcRes<T> = Box<dyn Future<Item=T, Error=String> + Send + 'static>;

/// Common operations that both types of UTXO clients have but implement them differently
pub trait UtxoRpcClientOps: Debug + 'static {
    fn list_unspent_ordered(&self, address: &Address) -> UtxoRpcRes<Vec<UnspentInfo>>;

    fn send_transaction(&self, tx: &UtxoTransaction, my_addr: Address) -> UtxoRpcRes<H256Json>;

    fn send_raw_transaction(&self, tx: BytesJson) -> RpcRes<H256Json>;

    fn get_transaction_bytes(&self, txid: H256Json) -> RpcRes<BytesJson>;

    fn get_verbose_transaction(&self, txid: H256Json) -> RpcRes<RpcTransaction>;

    fn get_block_count(&self) -> RpcRes<u64>;

    // TODO This operation is synchronous because it's currently simpler to do it this way.
    // Might consider refactoring when async/await is released.
    fn wait_for_payment_spend(&self, tx: &UtxoTransaction, vout: usize, wait_until: u64, from_block: u64) -> Result<UtxoTransaction, String>;

    // TODO This operation is synchronous because it's currently simpler to do it this way.
    // Might consider refactoring when async/await is released.
    fn wait_for_confirmations(&self, tx: &UtxoTransaction, confirmations: u32, wait_until: u64) -> Result<(), String> {
        loop {
            if now_ms() / 1000 > wait_until {
                return ERR!("Waited too long until {} for transaction {:?} to be confirmed {} times", wait_until, tx, confirmations);
            }

            match self.get_verbose_transaction(tx.hash().reversed().into()).wait() {
                Ok(t) => {
                    if t.confirmations >= confirmations {
                        return Ok(());
                    } else {
                        log!({"Waiting for tx {:?} confirmations, now {}, required {}", tx.hash().reversed(), t.confirmations, confirmations});
                    }
                },
                Err(e) => log!("Error " [e] " getting the transaction " [tx.hash().reversed()] ", retrying in 10 seconds"),
            }

            thread::sleep(Duration::from_secs(10));
        }
    }

    fn display_balance(&self, address: Address, decimals: u8) -> RpcRes<BigDecimal>;

    /// returns fee estimation per KByte in satoshis
    fn estimate_fee_sat(&self, decimals: u8, fee_method: &EstimateFeeMethod) -> RpcRes<u64>;
}

#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct NativeUnspent {
    pub txid: H256Json,
    pub vout: u32,
    pub address: String,
    pub account: Option<String>,
    #[serde(rename = "scriptPubKey")]
    pub script_pub_key: BytesJson,
    pub amount: f64,
    pub confirmations: u64,
    pub spendable: bool
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
    pub is_mine: bool,
    #[serde(rename = "iswatchonly")]
    pub is_watch_only: bool,
    #[serde(rename = "isscript")]
    pub is_script: bool,
    pub account: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListTransactionsItem {
    pub account: String,
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

#[derive(Debug)]
pub enum EstimateFeeMethod {
    /// estimatefee, deprecated in many coins: https://bitcoincore.org/en/doc/0.16.0/rpc/util/estimatefee/
    Standard,
    /// estimatesmartfee added since 0.16.0 bitcoind RPC: https://bitcoincore.org/en/doc/0.16.0/rpc/util/estimatesmartfee/
    SmartFee,
}

/// RPC client for UTXO based coins
/// https://bitcoin.org/en/developer-reference#rpc-quick-reference - Bitcoin RPC API reference
/// Other coins have additional methods or miss some of these
/// This description will be updated with more info
#[derive(Clone, Debug)]
pub struct NativeClientImpl {
    /// The uri to send requests to
    pub uri: String,
    /// Value of Authorization header, e.g. "Basic base64(user:password)"
    pub auth: String,
}

#[derive(Debug)]
pub struct NativeClient(pub Arc<NativeClientImpl>);
impl Deref for NativeClient {type Target = NativeClientImpl; fn deref (&self) -> &NativeClientImpl {&*self.0}}

impl JsonRpcClient for NativeClientImpl {
    fn version(&self) -> &'static str { "1.0" }

    fn next_id(&self) -> String { "0".into() }

    fn transport(&self, request: JsonRpcRequest) -> JsonRpcResponseFut {
        let request_body = try_fus!(json::to_string(&request));

        let http_request = try_fus!(
            Request::builder()
                    .method("POST")
                    .header(
                        AUTHORIZATION,
                        self.auth.clone()
                    )
                    .uri(self.uri.clone())
                    .body(Vec::from(request_body))
        );
        Box::new(slurp_req(http_request).then(move |result| -> Result<JsonRpcResponse, String> {
            let res = try_s!(result);
            let body = try_s!(std::str::from_utf8(&res.2));
            if res.0 != StatusCode::OK {
                return ERR!("Rpc request {:?} failed with HTTP status code {}, response body: {}",
                        request, res.0, body);
            }
            Ok(try_s!(json::from_str(body)))
        }))
    }
}

impl UtxoRpcClientOps for NativeClient {
    fn list_unspent_ordered(&self, address: &Address) -> UtxoRpcRes<Vec<UnspentInfo>> {
        let clone = self.0.clone();
        Box::new(self.list_unspent(0, 999999, vec![address.to_string()]).map_err(|e| ERRL!("{}", e)).and_then(move |unspents| {
            let mut futures = vec![];
            for unspent in unspents.iter() {
                let delay_f = Delay::new(Duration::from_millis(10)).map_err(|e| ERRL!("{}", e));
                let tx_id = unspent.txid.clone();
                let vout = unspent.vout as usize;
                let arc = clone.clone();
                // The delay here is required to mitigate "Work queue depth exceeded" error from coin daemon.
                // It happens even when we run requests sequentially.
                // Seems like daemon need some time to clean up it's queue after response is sent.
                futures.push(delay_f.and_then(move |_| arc.output_amount(tx_id, vout).map_err(|e| ERRL!("{}", e))));
            }

            join_all_sequential(futures).map(move |amounts| {
                let zip_iter = amounts.iter().zip(unspents.iter());
                let mut result: Vec<UnspentInfo> = zip_iter.map(|(value, unspent)| UnspentInfo {
                    outpoint: OutPoint {
                        hash: unspent.txid.reversed().into(),
                        index: unspent.vout,
                    },
                    value: *value,
                }).collect();

                result.sort_unstable_by(|a, b| {
                    if a.value < b.value {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                });
                result
            })
        }))
    }

    fn send_transaction(&self, tx: &UtxoTransaction, _addr: Address) -> UtxoRpcRes<H256Json> {
        Box::new(self.send_raw_transaction(BytesJson::from(serialize(tx))).map_err(|e| ERRL!("{}", e)))
    }

    fn get_verbose_transaction(&self, txid: H256Json) -> RpcRes<RpcTransaction> {
        self.get_raw_transaction_verbose(txid)
    }

    fn get_transaction_bytes(&self, txid: H256Json) -> RpcRes<BytesJson> {
        self.get_raw_transaction_bytes(txid)
    }

    fn get_block_count(&self) -> RpcRes<u64> {
        self.0.get_block_count()
    }

    fn wait_for_payment_spend(&self, tx: &UtxoTransaction, vout: usize, wait_until: u64, from_block: u64) -> Result<UtxoTransaction, String> {
        let mut current_height = from_block;
        loop {
            let coin_height = match self.get_block_count().wait() {
                Ok(h) => h,
                Err(e) => {
                    log!("Error " (e) " getting block count");
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };
            while current_height <= coin_height {
                let block = match self.get_block(current_height.to_string()).wait() {
                    Ok(b) => b,
                    Err(e) => {
                        log!("Error " (e) " getting block " (current_height));
                        break;
                    }
                };
                let mut got_error = false;

                for tx_hash in block.tx.iter() {
                    let transaction = match self.get_raw_transaction_bytes(tx_hash.clone()).wait() {
                        Ok(tx) => tx,
                        Err(e) => {
                            log!("Error " (e) " getting transaction " [tx_hash]);
                            got_error = true;
                            break;
                        },
                    };

                    let maybe_spend_tx: UtxoTransaction = match deserialize(transaction.as_slice()) {
                        Ok(t) => t,
                        Err(e) => {
                            log!("Error " [e] " deserializing transaction " [transaction]);
                            got_error = true;
                            break;
                        }
                    };

                    for input in maybe_spend_tx.inputs.iter() {
                        if input.previous_output.hash == tx.hash() && input.previous_output.index == vout as u32 {
                            return Ok(maybe_spend_tx);
                        }
                    }
                };

                if !got_error {
                    current_height += 1;
                } else {
                    thread::sleep(Duration::from_secs(10));
                }
            }

            if now_ms() / 1000 > wait_until {
                return ERR!("Waited too long until {} for transaction {:?} {} to be spent ", wait_until, tx, vout);
            }
            thread::sleep(Duration::from_secs(10));
        }
    }

    fn display_balance(&self, address: Address, _decimals: u8) -> RpcRes<BigDecimal> {
        Box::new(self.list_unspent(0, 999999, vec![address.to_string()]).map(|unspents|
            unspents.iter().fold(0., |sum, unspent| sum + unspent.amount).into()
        ))
    }

    fn estimate_fee_sat(&self, decimals: u8, fee_method: &EstimateFeeMethod) -> RpcRes<u64> {
        match fee_method {
            EstimateFeeMethod::Standard => Box::new(self.estimate_fee().map(move |fee|
                if fee > 0.00001 {
                    (fee * 10.0_f64.powf(decimals as f64)) as u64
                } else {
                    1000
                }
            )),
            EstimateFeeMethod::SmartFee => Box::new(self.estimate_smart_fee().map(move |res|
                if res.fee_rate > 0.00001 {
                    (res.fee_rate * 10.0_f64.powf(decimals as f64)) as u64
                } else {
                    1000
                }
            )),
        }
    }

    /// https://bitcoin.org/en/developer-reference#sendrawtransaction
    fn send_raw_transaction(&self, tx: BytesJson) -> RpcRes<H256Json> {
        rpc_func!(self, "sendrawtransaction", tx)
    }
}

#[cfg_attr(test, mockable)]
impl NativeClientImpl {
    /// https://bitcoin.org/en/developer-reference#listunspent
    pub fn list_unspent(&self, min_conf: u64, max_conf: u64, addresses: Vec<String>) -> RpcRes<Vec<NativeUnspent>> {
        rpc_func!(self, "listunspent", min_conf, max_conf, addresses)
    }

    /// https://bitcoin.org/en/developer-reference#importaddress
    pub fn import_address(&self, address: &str, label: &str, rescan: bool) -> RpcRes<()> {
        rpc_func!(self, "importaddress", address, label, rescan)
    }

    /// https://bitcoin.org/en/developer-reference#validateaddress
    pub fn validate_address(&self, address: String) -> RpcRes<ValidateAddressRes> {
        rpc_func!(self, "validateaddress", address)
    }

    pub fn output_amount(&self, txid: H256Json, index: usize) -> UtxoRpcRes<u64> {
        let fut = self.get_raw_transaction_bytes(txid).map_err(|e| ERRL!("{}", e));
        Box::new(fut.and_then(move |bytes| {
            let tx: UtxoTransaction = try_s!(deserialize(bytes.as_slice()).map_err(|e| ERRL!("Error {:?} trying to deserialize the transaction {:?}", e, bytes)));
            Ok(tx.outputs[index].value)
        }))
    }

    /// https://bitcoin.org/en/developer-reference#getblock
    /// Always returns verbose block
    pub fn get_block(&self, height: String) -> RpcRes<VerboseBlockClient> {
        let verbose = true;
        rpc_func!(self, "getblock", height, verbose)
    }

    /// https://bitcoin.org/en/developer-reference#getblockcount
    pub fn get_block_count(&self) -> RpcRes<u64> {
        rpc_func!(self, "getblockcount")
    }

    /// https://bitcoin.org/en/developer-reference#getrawtransaction
    /// Always returns verbose transaction
    fn get_raw_transaction_verbose(&self, txid: H256Json) -> RpcRes<RpcTransaction> {
        let verbose = 1;
        rpc_func!(self, "getrawtransaction", txid, verbose)
    }

    /// https://bitcoin.org/en/developer-reference#getrawtransaction
    /// Always returns transaction bytes
    fn get_raw_transaction_bytes(&self, txid: H256Json) -> RpcRes<BytesJson> {
        let verbose = 0;
        rpc_func!(self, "getrawtransaction", txid, verbose)
    }

    /// https://bitcoin.org/en/developer-reference#estimatefee
    /// Always estimate fee for transaction to be confirmed in next block
    fn estimate_fee(&self) -> RpcRes<f64> {
        let n_blocks = 1;
        rpc_func!(self, "estimatefee", n_blocks)
    }

    /// https://bitcoincore.org/en/doc/0.18.0/rpc/util/estimatesmartfee/
    /// Always estimate fee for transaction to be confirmed in next block
    pub fn estimate_smart_fee(&self) -> RpcRes<EstimateSmartFeeRes> {
        let n_blocks = 1;
        rpc_func!(self, "estimatesmartfee", n_blocks)
    }

    /// https://bitcoin.org/en/developer-reference#listtransactions
    pub fn list_transactions(&self, count: u64, from: u64) -> RpcRes<Vec<ListTransactionsItem>> {
        let account = "*";
        let watch_only = true;
        rpc_func!(self, "listtransactions", account, count, from, watch_only)
    }

    /// https://bitcoin.org/en/developer-reference#listreceivedbyaddress
    pub fn list_received_by_address(&self, min_conf: u64, include_empty: bool, include_watch_only: bool) -> RpcRes<Vec<ReceivedByAddressItem>> {
        rpc_func!(self, "listreceivedbyaddress", min_conf, include_empty, include_watch_only)
    }

    pub fn detect_fee_method(&self) -> impl Future<Item=EstimateFeeMethod, Error=String> {
        let estimate_fee_fut = self.estimate_fee();
        self.estimate_smart_fee().then(move |res| -> Box<dyn Future<Item=EstimateFeeMethod, Error=String>> {
            match res {
                Ok(smart_fee) => if smart_fee.fee_rate > 0. {
                    Box::new(futures::future::ok(EstimateFeeMethod::SmartFee))
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
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
/// Deserializable Electrum protocol representation for RPC
pub enum ElectrumProtocol {
    /// TCP
    TCP,
    /// SSL/TLS
    SSL,
}

#[derive(Debug, Deserialize)]
/// Electrum request RPC representation
pub struct ElectrumRpcRequest {
    pub url: String,
    #[serde(default)]
    pub protocol: ElectrumProtocol,
    #[serde(default)]
    pub disable_cert_verification: bool,
}

impl Default for ElectrumProtocol {
    fn default() -> Self {
        ElectrumProtocol::TCP
    }
}

#[derive(Clone, Debug)]
/// Electrum client configuration
enum ElectrumConfig {
    TCP,
    SSL(DNSName, bool)
}

/// Attempts to proccess the request (parse url, etc), build up the config and create new electrum connection
pub fn spawn_electrum(
    req: &ElectrumRpcRequest
) -> Result<ElectrumConnection, String> {
    let mut addr = match req.url.to_socket_addrs() {
        Ok(a) => a,
        Err(e) => return ERR!("{} error {:?}", req.url, e),
    };
    let addr = match addr.next() {
        Some(a) => a,
        None => return ERR!("Socket addr from addr {} is None.", req.url),
    };
    let config = match req.protocol {
        ElectrumProtocol::TCP => ElectrumConfig::TCP,
        ElectrumProtocol::SSL => {
            let uri: Uri = try_s!(req.url.parse());
            let host = try_s!(uri.host().ok_or(ERRL!("Couldn't retrieve host from addr {}", req.url)));
            let dns = try_s!(DNSNameRef::try_from_ascii_str(host).map_err(|e| ERRL!("{:?}", e)));
            ElectrumConfig::SSL(dns.into(), req.disable_cert_verification)
        }
    };

    Ok(electrum_connect(addr, config))
}

#[derive(Clone, Debug)]
/// Represents the active Electrum connection to selected address
pub struct ElectrumConnection {
    /// The client connected to this SocketAddr
    addr: SocketAddr,
    /// Configuration
    config: ElectrumConfig,
    /// The Sender forwarding requests to writing part of underlying stream
    tx: mpsc::Sender<Vec<u8>>,
    /// Responses are stored here
    responses: Arc<Mutex<HashMap<String, JsonRpcResponse>>>,
    /// Tracks the current connection state
    is_connected: Arc<AtomicBool>,
}

impl ElectrumConnection {
    fn is_connected(&self) -> bool {
        self.is_connected.load(AtomicOrdering::Relaxed)
    }
}

#[derive(Debug)]
pub struct ElectrumClientImpl {
    connections: Vec<ElectrumConnection>,
    next_id: Mutex<u64>,
}

impl ElectrumClientImpl {
    fn electrum_request_multi(
        &self,
        request: JsonRpcRequest,
    ) -> JsonRpcResponseFut {
        let mut futures = vec![];
        for connection in self.connections.iter() {
            if connection.is_connected() {
                futures.push(electrum_request(request.clone(), connection.tx.clone(), connection.responses.clone()));
            }
        }
        if futures.is_empty() {
            return Box::new(futures::future::err(ERRL!("All electrums are currently disconnected")));
        }
        if request.method != "server.ping" {
            Box::new(select_ok_sequential(futures).map_err(|e| ERRL!("{:?}", e)))
        } else {
            // server.ping must be sent to all servers to keep all connections alive
            Box::new(select_ok(futures).map(|(result, _)| result).map_err(|e| ERRL!("{:?}", e)))
        }
    }

    pub fn is_connected(&self) -> bool {
        for connection in self.connections.iter() {
            if connection.is_connected() {
                return true;
            }
        }
        false
    }
}

#[derive(Debug)]
pub struct ElectrumClient(pub Arc<ElectrumClientImpl>);
impl Deref for ElectrumClient {type Target = ElectrumClientImpl; fn deref (&self) -> &ElectrumClientImpl {&*self.0}}

const BLOCKCHAIN_HEADERS_SUB_ID: &'static str = "blockchain.headers.subscribe";

impl JsonRpcClient for ElectrumClientImpl {
    fn version(&self) -> &'static str { "2.0" }

    fn next_id(&self) -> String {
        let mut next = unwrap!(self.next_id.lock());
        *next += 1;
        next.to_string()
    }

    fn transport(&self, request: JsonRpcRequest) -> JsonRpcResponseFut {
        Box::new(self.electrum_request_multi(request))
    }
}

impl UtxoRpcClientOps for ElectrumClient {
    fn list_unspent_ordered(&self, address: &Address) -> UtxoRpcRes<Vec<UnspentInfo>> {
        let script = Builder::build_p2pkh(&address.hash);
        let script_hash = electrum_script_hash(&script);
        Box::new(self.scripthash_list_unspent(&hex::encode(script_hash)).map_err(|e| ERRL!("{}", e)).map(move |unspents| {
            let mut result: Vec<UnspentInfo> = unspents.iter().map(|unspent| UnspentInfo {
                outpoint: OutPoint {
                    hash: unspent.tx_hash.reversed().into(),
                    index: unspent.tx_pos,
                },
                value: unspent.value
            }).collect();

            result.sort_unstable_by(|a, b| {
                if a.value < b.value {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            });
            result
        }))
    }

    fn send_transaction(&self, tx: &UtxoTransaction, my_addr: Address) -> UtxoRpcRes<H256Json> {
        let bytes = BytesJson::from(serialize(tx));
        let inputs = tx.inputs.clone();
        let arc = self.0.clone();
        let script = Builder::build_p2pkh(&my_addr.hash);
        let script_hash = hex::encode(electrum_script_hash(&script));
        Box::new(self.blockchain_transaction_broadcast(bytes).map_err(|e| ERRL!("{}", e)).and_then(move |res| {
            // Check every second until Electrum server recognizes that used UTXOs are spent
            loop_fn((res, arc, script_hash, inputs), move |(res, arc, script_hash, inputs)| {
                let delay_f = Delay::new(Duration::from_secs(1)).map_err(|e| ERRL!("{}", e));
                delay_f.and_then(move |_res| {
                    arc.scripthash_list_unspent(&script_hash).then(move |unspents| {
                        let unspents = match unspents {
                            Ok(unspents) => unspents,
                            Err(e) => {
                                log!("Error getting Electrum unspents " [e]);
                                // we can just keep looping in case of error hoping it will go away
                                return Ok(Loop::Continue((res, arc, script_hash, inputs)));
                            }
                        };

                        for input in inputs.iter() {
                            let find = unspents.iter().find(|unspent| {
                                unspent.tx_hash == input.previous_output.hash.reversed().into() && unspent.tx_pos == input.previous_output.index
                            });
                            // Check again if at least 1 spent outpoint is still there
                            if find.is_some() {
                                return Ok(Loop::Continue((res, arc, script_hash, inputs)));
                            }
                        }

                        Ok(Loop::Break(res))
                    })
                })
            })
        }))
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-get
    /// returns verbose transaction by default
    fn get_verbose_transaction(&self, txid: H256Json) -> RpcRes<RpcTransaction> {
        let verbose = true;
        rpc_func!(self, "blockchain.transaction.get", txid, verbose)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-get
    /// returns transaction bytes by default
    fn get_transaction_bytes(&self, txid: H256Json) -> RpcRes<BytesJson> {
        let verbose = false;
        rpc_func!(self, "blockchain.transaction.get", txid, verbose)
    }

    fn get_block_count(&self) -> RpcRes<u64> {
        Box::new(self.blockchain_headers_subscribe().map(|r| r.block_height()))
    }

    /// This function is assumed to be used to search for spend of swap payment.
    /// For this case we can just wait that address history contains 2 or more records: the payment itself and spending transaction.
    fn wait_for_payment_spend(&self, tx: &UtxoTransaction, vout: usize, wait_until: u64, _from_block: u64) -> Result<UtxoTransaction, String> {
        let script_hash = hex::encode(electrum_script_hash(&tx.outputs[vout].script_pubkey));

        loop {
            let history = match self.scripthash_get_history(&script_hash).wait() {
                Ok(h) => h,
                Err(e) => {
                    log!("Error {} " (e) " getting scripthash history");
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };
            if history.len() < 2 {
                if now_ms() / 1000 > wait_until {
                    return ERR!("Waited too long until {} for output {:?} to be spent ", wait_until, tx.outputs[vout]);
                }
                thread::sleep(Duration::from_secs(10));
                continue;
            }

            for item in history.iter() {
                let transaction = match self.get_transaction_bytes(item.tx_hash.clone()).wait() {
                    Ok(tx) => tx,
                    Err(e) => {
                        log!("Error " (e) " getting transaction " [item.tx_hash]);
                        continue;
                    },
                };

                let maybe_spend_tx: UtxoTransaction = match deserialize(transaction.as_slice()) {
                    Ok(t) => t,
                    Err(e) => {
                        log!("Error " [e] " deserializing transaction " [transaction]);
                        continue;
                    }
                };

                for input in maybe_spend_tx.inputs.iter() {
                    if input.previous_output.hash == tx.hash() && input.previous_output.index == vout as u32 {
                        return Ok(maybe_spend_tx);
                    }
                }
            }

            if now_ms() / 1000 > wait_until {
                return ERR!("Waited too long until {} for output {:?} to be spent ", wait_until, tx.outputs[vout]);
            }
            thread::sleep(Duration::from_secs(10));
        }
    }

    fn display_balance(&self, address: Address, decimals: u8) -> RpcRes<BigDecimal> {
        let hash = electrum_script_hash(&Builder::build_p2pkh(&address.hash));
        let hash_str = hex::encode(hash);
        Box::new(self.scripthash_get_balance(&hash_str).map(move |result| {
            BigDecimal::from(result.confirmed + result.unconfirmed) / BigDecimal::from(10u64.pow(decimals as u32))
        }))
    }

    fn estimate_fee_sat(&self, decimals: u8, _fee_method: &EstimateFeeMethod) -> RpcRes<u64> {
        Box::new(self.estimate_fee().map(move |fee|
            if fee > 0.00001 {
                (fee * 10.0_f64.powf(decimals as f64)) as u64
            } else {
                1000
            }
        ))
    }

    fn send_raw_transaction(&self, tx: BytesJson) -> RpcRes<H256Json> {
        self.blockchain_transaction_broadcast(tx)
    }
}

#[cfg_attr(test, mockable)]
impl ElectrumClientImpl {
    pub fn new() -> ElectrumClientImpl {
        ElectrumClientImpl {
            connections: vec![],
            next_id: Mutex::new(0),
        }
    }

    pub fn add_server(&mut self, req: &ElectrumRpcRequest) -> Result<(), String> {
        let connection = try_s!(spawn_electrum(req));
        self.connections.push(connection);
        Ok(())
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#server-ping
    pub fn server_ping(&self) -> RpcRes<()> {
        rpc_func!(self, "server.ping")
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-listunspent
    /// It can return duplicates sometimes: https://github.com/artemii235/SuperNET/issues/269
    /// We should remove them to build valid transactions
    fn scripthash_list_unspent(&self, hash: &str) -> RpcRes<Vec<ElectrumUnspent>> {
        Box::new(rpc_func!(self, "blockchain.scripthash.listunspent", hash).and_then(move |unspents: Vec<ElectrumUnspent>| {
            let mut map: HashMap<(H256Json, u32), bool> = HashMap::new();
            let unspents = unspents.into_iter().filter(|unspent| {
                match map.entry((unspent.tx_hash.clone(), unspent.tx_pos)) {
                    Entry::Occupied(_) => false,
                    Entry::Vacant(e) => {
                        e.insert(true);
                        true
                    },
                }
            }).collect();
            Ok(unspents)
        }))
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-get-history
    pub fn scripthash_get_history(&self, hash: &str) -> RpcRes<Vec<ElectrumTxHistoryItem>> {
        rpc_func!(self, "blockchain.scripthash.get_history", hash)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-gethistory
    fn scripthash_get_balance(&self, hash: &str) -> RpcRes<ElectrumBalance> {
        rpc_func!(self, "blockchain.scripthash.get_balance", hash)
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
    fn estimate_fee(&self) -> RpcRes<f64> {
        let n_blocks = 1;
        rpc_func!(self, "blockchain.estimatefee", n_blocks)
    }
}

/// Helper function casting mpsc::Receiver as Stream.
fn rx_to_stream(rx: mpsc::Receiver<Vec<u8>>) -> impl Stream<Item = Vec<u8>, Error = io::Error> {
    rx.map_err(|_| panic!("errors not possible on rx"))
}

fn electrum_process_chunk(chunk: &[u8], arc: Arc<Mutex<HashMap<String, JsonRpcResponse>>>) {
    // we should split the received chunk because we can get several responses in 1 chunk.
    let split = chunk.split(|item| *item == '\n' as u8);

    for chunk in split {
        // split returns empty slice if it ends with separator which is our case
        if chunk.len() > 0 {
            let raw_json: Json = match json::from_slice(chunk) {
                Ok(json) => json,
                Err(e) => {
                    log!([e]);
                    return;
                }
            };

            // detect if we got standard JSONRPC response or subscription response as JSONRPC request
            if raw_json["method"].is_null() && raw_json["params"].is_null() {
                let response: JsonRpcResponse = match json::from_value(raw_json) {
                    Ok(res) => res,
                    Err(e) => {
                        log!([e]);
                        return;
                    }
                };
                (*arc.lock().unwrap()).insert(response.id.to_string(), response);
            } else {
                let request: JsonRpcRequest = match json::from_value(raw_json) {
                    Ok(res) => res,
                    Err(e) => {
                        log!([e]);
                        return;
                    }
                };
                let id = match request.method.as_ref() {
                    BLOCKCHAIN_HEADERS_SUB_ID => BLOCKCHAIN_HEADERS_SUB_ID,
                    _ => {
                        log!("Couldn't get id of request " [request]);
                        return;
                    }
                };

                let response = JsonRpcResponse {
                    id: id.into(),
                    jsonrpc: "2.0".into(),
                    result: request.params[0].clone(),
                    error: Json::Null,
                };
                (*arc.lock().unwrap()).insert(id.into(), response);
            }
        }
    }
}

macro_rules! try_loop {
    ($e:expr, $rx: ident, $connection: ident, $delay: ident) => {
        match $e {
            Ok(res) => res,
            Err(e) => {
                log!([$connection.addr] " error " [e]);
                if $delay < 30 {
                    $delay += 5;
                }
                return Box::new(futures::future::ok(Loop::Continue(($rx, $connection, $delay))));
            }
        }
    };
}

/// The enum wrapping possible variants of underlying Streams
enum ElectrumStream<S> {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream, S>),
}

impl<S> AsRef<TcpStream> for ElectrumStream<S> {
    fn as_ref(&self) -> &TcpStream {
        match self {
            ElectrumStream::Tcp(stream) => stream,
            ElectrumStream::Tls(stream) => stream.get_ref().0
        }
    }
}

impl<S: Session> std::io::Read for ElectrumStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            ElectrumStream::Tcp(stream) => stream.read(buf),
            ElectrumStream::Tls(stream) => stream.read(buf),
        }
    }
}

impl<S: Session> AsyncRead for ElectrumStream<S> {}

impl<S: Session> std::io::Write for ElectrumStream<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            ElectrumStream::Tcp(stream) => stream.write(buf),
            ElectrumStream::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            ElectrumStream::Tcp(stream) => stream.flush(),
            ElectrumStream::Tls(stream) => stream.flush(),
        }
    }
}

impl<S: Session> AsyncWrite for ElectrumStream<S> {
    fn shutdown(&mut self) -> Poll<(), std::io::Error> {
        match self {
            ElectrumStream::Tcp(stream) => stream.shutdown(),
            ElectrumStream::Tls(stream) => stream.shutdown(),
        }
    }
}

const ELECTRUM_TIMEOUT: u64 = 60;

/// Builds up the electrum connection, spawns endless loop that attempts to reconnect to the server
/// in case of connection errors
fn electrum_connect(
    addr: SocketAddr,
    config: ElectrumConfig
) -> ElectrumConnection {
    let (tx, rx) = mpsc::channel(0);
    let rx = rx_to_stream(rx);
    let connection = ElectrumConnection {
        addr,
        config,
        is_connected: Arc::new(AtomicBool::new(false)),
        tx,
        responses: Arc::new(Mutex::new(HashMap::new())),
    };

    let connect_loop = loop_fn((rx, connection.clone(), 0), move |(rx, connection, mut delay)| {
        let connect_f = match &connection.config {
            ElectrumConfig::TCP => Either::A(TcpStream::connect(&connection.addr).map(|stream| ElectrumStream::Tcp(stream))),
            ElectrumConfig::SSL(dns, skip_validation) => {
                let dns = dns.clone();
                let mut ssl_config = ClientConfig::new();
                ssl_config.root_store.add_server_trust_anchors(&TLS_SERVER_ROOTS);
                if *skip_validation {
                    ssl_config
                        .dangerous()
                        .set_certificate_verifier(Arc::new(NoCertificateVerification {}));
                }
                let tls_connector = TlsConnector::from(Arc::new(ssl_config));

                Either::B(TcpStream::connect(&connection.addr).and_then(move |stream| {
                    tls_connector.connect(dns.as_ref(), stream).map(|stream| ElectrumStream::Tls(stream))
                }))
            }
        };

        let with_delay_f = if delay > 0 {
            Either::A(Delay::new(Duration::from_secs(delay)).and_then(move |_| connect_f))
        } else {
            Either::B(connect_f)
        };
        with_delay_f.then(move |stream| -> Box<dyn Future<Item=Loop<(), _>, Error=_> + Send> {
            let stream = try_loop!(stream, rx, connection, delay);
            try_loop!(stream.as_ref().set_nodelay(true), rx, connection, delay);
            connection.is_connected.store(true, AtomicOrdering::Relaxed);
            // reset the delay if we've connected successfully
            delay = 5;
            let stream_clone = try_loop!(stream.as_ref().try_clone(), rx, connection, delay);
            let last_chunk = Arc::new(AtomicU64::new(now_ms()));
            let interval = Interval::new(Duration::from_secs(ELECTRUM_TIMEOUT)).map_err(|e| { log!([e]); () });
            CORE.spawn({
                let last_chunk = last_chunk.clone();
                move |_| interval.for_each(move |_| {
                    let last = last_chunk.load(AtomicOrdering::Relaxed);
                    if now_ms() > last + ELECTRUM_TIMEOUT * 1000 {
                        log!([addr] " Didn't receive any data since " (last / 1000) ". Shutting down the connection.");
                        if let Err(e) = stream_clone.shutdown(Shutdown::Both) {
                            log!([addr] " error shutting down the connection " [e]);
                        }
                        // return err to shutdown interval execution
                        return futures::future::err(());
                    };
                    futures::future::ok(())
                })
            });
            let (sink, stream) = Bytes.framed(stream).split();
            // this forwards the messages from rx to sink (write) part of tcp stream
            let send_all = SendAll::new(sink, rx);
            CORE.spawn({
                let responses = connection.responses.clone();
                move |_| stream
                    .for_each(move |chunk| {
                        last_chunk.store(now_ms(), AtomicOrdering::Relaxed);
                        electrum_process_chunk(&chunk, responses.clone());
                        futures::future::ok(())
                    })
                    .map_err(move |e| {
                        log!([e]);
                        ()
                    })
            });

            Box::new(send_all.then(move |result| {
                connection.is_connected.store(false, AtomicOrdering::Relaxed);
                if let Err((rx, e)) = result {
                    log!([addr] " failed to write to socket " [e]);
                    return Ok(Loop::Continue((rx, connection, delay)));
                }
                Ok(Loop::Break(()))
            }))
        })
    });

    CORE.spawn(|_| connect_loop);
    connection
}

/// A simple `Codec` implementation that reads buffer until \n according to Electrum protocol specification:
/// https://electrumx.readthedocs.io/en/latest/protocol-basics.html#message-stream
///
/// Implementation adopted from https://github.com/tokio-rs/tokio/blob/master/examples/connect.rs#L84
pub struct Bytes;

impl Decoder for Bytes {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<BytesMut>> {
        let len = buf.len();
        if len > 0 && buf[len - 1] == '\n' as u8 {
            Ok(Some(buf.split_to(len)))
        } else {
            Ok(None)
        }
    }
}

impl Encoder for Bytes {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn encode(&mut self, data: Vec<u8>, buf: &mut BytesMut) -> io::Result<()> {
        buf.extend_from_slice(&data);
        Ok(())
    }
}

struct ElectrumResponseFut {
    responses: Arc<Mutex<HashMap<String, JsonRpcResponse>>>,
    request_id: String,
}

impl Future for ElectrumResponseFut {
    type Item = JsonRpcResponse;
    type Error = String;

    fn poll(&mut self) -> Poll<JsonRpcResponse, String> {
        loop {
            let elem = try_s!(self.responses.lock()).remove(&self.request_id);
            if let Some(res) = elem {
                return Ok(Async::Ready(res))
            } else {
                let task = futures::task::current();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(200));
                    task.notify();
                });
                return Ok(Async::NotReady)
            }
        }
    }
}

fn electrum_request(
    request: JsonRpcRequest,
    tx: mpsc::Sender<Vec<u8>>,
    responses: Arc<Mutex<HashMap<String, JsonRpcResponse>>>
) -> JsonRpcResponseFut {
    let mut json = try_fus!(json::to_string(&request));
    // Electrum request and responses must end with \n
    // https://electrumx.readthedocs.io/en/latest/protocol-basics.html#message-stream
    json.push('\n');
    let request_id = request.get_id().to_string();
    let send_fut = tx.send(json.into_bytes())
        .map_err(|e| ERRL!("{}", e))
        .and_then(move |_res| {
            ElectrumResponseFut {
                request_id,
                responses,
            }
        })
        .map_err(|e| StringError(e))
        .timeout(Duration::from_secs(ELECTRUM_TIMEOUT));

    Box::new(send_fut.map_err(|e| ERRL!("{}", e.0)))
}

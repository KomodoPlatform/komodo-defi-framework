#![cfg_attr(target_arch = "wasm32", allow(unused_macros))]
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use crate::utxo::{output_script, sat_from_big_decimal};
use crate::{big_decimal_from_sat_unsigned, NumConversError, RpcTransportEventHandler, RpcTransportEventHandlerShared};
use async_trait::async_trait;
use chain::{BlockHeader, BlockHeaderBits, BlockHeaderNonce, OutPoint, Transaction as UtxoTx};
use common::custom_futures::{select_ok_sequential, FutureTimerExt};
use common::custom_iter::{CollectInto, TryIntoGroupMap};
use common::executor::{spawn, Timer};
use common::jsonrpc_client::{JsonRpcBatchClient, JsonRpcBatchResponse, JsonRpcClient, JsonRpcError, JsonRpcErrorType,
                             JsonRpcId, JsonRpcMultiClient, JsonRpcRemoteAddr, JsonRpcRequest, JsonRpcRequestEnum,
                             JsonRpcResponse, JsonRpcResponseEnum, JsonRpcResponseFut, RpcRes};
use common::log::{error, info, warn};
use common::{median, now_float, now_ms, OrdRange};
use derive_more::Display;
use futures::channel::oneshot as async_oneshot;
use futures::compat::{Future01CompatExt, Stream01CompatExt};
use futures::future::{select as select_func, FutureExt, TryFutureExt};
use futures::lock::Mutex as AsyncMutex;
use futures::{select, StreamExt};
use futures01::future::select_ok;
use futures01::sync::{mpsc, oneshot};
use futures01::{Future, Sink, Stream};
use http::Uri;
use itertools::Itertools;
use keys::hash::H256;
use keys::{Address, Type as ScriptType};
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, BigInt, MmNumber};
#[cfg(test)] use mocktopus::macros::*;
use rpc::v1::types::{Bytes as BytesJson, Transaction as RpcTransaction, H256 as H256Json};
use serde_json::{self as json, Value as Json};
use serialization::{deserialize, serialize, serialize_with_flags, CoinVariant, CompactInteger, Reader,
                    SERIALIZE_TRANSACTION_WITNESS};
use sha2::{Digest, Sha256};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::num::NonZeroU64;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Duration;

cfg_native! {
    use futures::future::Either;
    use futures::io::Error;
    use http::header::AUTHORIZATION;
    use http::{Request, StatusCode};
    use rustls::client::ServerCertVerified;
    use rustls::{Certificate, ClientConfig, ServerName, OwnedTrustAnchor, RootCertStore};
    use std::convert::TryFrom;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::SystemTime;
    use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf};
    use tokio::net::TcpStream;
    use tokio_rustls::{client::TlsStream, TlsConnector};
    use tokio_rustls::webpki::DnsNameRef;
    use webpki_roots::TLS_SERVER_ROOTS;
}

pub type AddressesByLabelResult = HashMap<String, AddressPurpose>;
pub type JsonRpcPendingRequestsShared = Arc<AsyncMutex<JsonRpcPendingRequests>>;
pub type JsonRpcPendingRequests = HashMap<JsonRpcId, async_oneshot::Sender<JsonRpcResponseEnum>>;
pub type UnspentMap = HashMap<Address, Vec<UnspentInfo>>;

type ElectrumScriptHash = String;
type ScriptHashUnspents = Vec<ElectrumUnspent>;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AddressPurpose {
    purpose: String,
}

/// Skips the server certificate verification on TLS connection
pub struct NoCertificateVerification {}

#[cfg(not(target_arch = "wasm32"))]
impl rustls::client::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _: &Certificate,
        _: &[Certificate],
        _: &ServerName,
        _: &mut dyn Iterator<Item = &[u8]>,
        _: &[u8],
        _: SystemTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
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
        tx_hash: H256Json,
        expiry_height: u32,
        confirmations: u32,
        requires_notarization: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let selfi = self.clone();
        let fut = async move {
            loop {
                if now_ms() / 1000 > wait_until {
                    return ERR!(
                        "Waited too long until {} for transaction {:?} to be confirmed {} times",
                        wait_until,
                        tx_hash,
                        confirmations
                    );
                }

                match selfi.get_verbose_transaction(&tx_hash).compat().await {
                    Ok(t) => {
                        let tx_confirmations = if requires_notarization {
                            t.confirmations
                        } else {
                            t.rawconfirmations.unwrap_or(t.confirmations)
                        };
                        if tx_confirmations >= confirmations {
                            return Ok(());
                        } else {
                            info!(
                                "Waiting for tx {:?} confirmations, now {}, required {}, requires_notarization {}",
                                tx_hash, tx_confirmations, confirmations, requires_notarization
                            )
                        }
                    },
                    Err(e) => {
                        if expiry_height > 0 {
                            let block = match selfi.get_block_count().compat().await {
                                Ok(b) => b,
                                Err(e) => {
                                    error!("Error {} getting block number, retrying in 10 seconds", e);
                                    Timer::sleep(check_every as f64).await;
                                    continue;
                                },
                            };

                            if block > expiry_height as u64 {
                                return ERR!("The transaction {:?} has expired, current block {}", tx_hash, block);
                            }
                        }
                        error!(
                            "Error {:?} getting the transaction {:?}, retrying in 10 seconds",
                            e, tx_hash
                        )
                    },
                }

                Timer::sleep(check_every as f64).await;
            }
        };
        Box::new(fut.boxed().compat())
    }

    #[inline]
    pub fn is_native(&self) -> bool {
        match self {
            UtxoRpcClientEnum::Native(_) => true,
            UtxoRpcClientEnum::Electrum(_) => false,
        }
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

impl From<ElectrumUnspent> for UnspentInfo {
    fn from(electrum: ElectrumUnspent) -> UnspentInfo {
        UnspentInfo {
            outpoint: OutPoint {
                hash: electrum.tx_hash.reversed().into(),
                index: electrum.tx_pos,
            },
            value: electrum.value,
            height: electrum.height,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum BlockHashOrHeight {
    Height(i64),
    Hash(H256Json),
}

#[derive(Debug, PartialEq)]
pub struct SpentOutputInfo {
    // The transaction spending the output
    pub spending_tx: UtxoTx,
    // The input index that spends the output
    pub input_index: usize,
    // The block hash or height the includes the spending transaction
    // For electrum clients the block height will be returned, for native clients the block hash will be returned
    pub spent_in_block: BlockHashOrHeight,
}

pub type UtxoRpcResult<T> = Result<T, MmError<UtxoRpcError>>;
pub type UtxoRpcFut<T> = Box<dyn Future<Item = T, Error = MmError<UtxoRpcError>> + Send + 'static>;

#[derive(Debug, Display)]
pub enum UtxoRpcError {
    Transport(JsonRpcError),
    ResponseParseError(JsonRpcError),
    InvalidResponse(String),
    Internal(String),
}

impl From<JsonRpcError> for UtxoRpcError {
    fn from(e: JsonRpcError) -> Self {
        match e.error {
            JsonRpcErrorType::InvalidRequest(_) => UtxoRpcError::Internal(e.to_string()),
            JsonRpcErrorType::Transport(_) => UtxoRpcError::Transport(e),
            JsonRpcErrorType::Parse(_, _) | JsonRpcErrorType::Response(_, _) => UtxoRpcError::ResponseParseError(e),
        }
    }
}

impl From<serialization::Error> for UtxoRpcError {
    fn from(e: serialization::Error) -> Self { UtxoRpcError::InvalidResponse(format!("{:?}", e)) }
}

impl From<NumConversError> for UtxoRpcError {
    fn from(e: NumConversError) -> Self { UtxoRpcError::Internal(e.to_string()) }
}

/// Common operations that both types of UTXO clients have but implement them differently
#[async_trait]
pub trait UtxoRpcClientOps: fmt::Debug + Send + Sync + 'static {
    /// Returns available unspents for the given `address`.
    fn list_unspent(&self, address: &Address, decimals: u8) -> UtxoRpcFut<Vec<UnspentInfo>>;

    /// Returns available unspents for every given `addresses`.
    fn list_unspent_group(&self, addresses: Vec<Address>, decimals: u8) -> UtxoRpcFut<UnspentMap>;

    /// Submits the given `tx` transaction to blockchain network.
    fn send_transaction(&self, tx: &UtxoTx) -> UtxoRpcFut<H256Json>;

    /// Submits the raw `tx` transaction (serialized, hex-encoded) to blockchain network.
    fn send_raw_transaction(&self, tx: BytesJson) -> UtxoRpcFut<H256Json>;

    /// Returns raw transaction (serialized, hex-encoded) by the given `txid`.
    fn get_transaction_bytes(&self, txid: &H256Json) -> UtxoRpcFut<BytesJson>;

    /// Returns verbose transaction by the given `txid`.
    fn get_verbose_transaction(&self, txid: &H256Json) -> UtxoRpcFut<RpcTransaction>;

    /// Returns verbose transactions in the same order they were requested.
    fn get_verbose_transactions(&self, tx_ids: &[H256Json]) -> UtxoRpcFut<Vec<RpcTransaction>>;

    /// Returns the height of the most-work fully-validated chain.
    fn get_block_count(&self) -> UtxoRpcFut<u64>;

    /// Requests balance of the given `address`.
    fn display_balance(&self, address: Address, decimals: u8) -> RpcRes<BigDecimal>;

    /// Requests balances of the given `addresses`.
    /// The pairs `(Address, BigDecimal)` are guaranteed to be in the same order in which they were requested.
    fn display_balances(&self, addresses: Vec<Address>, decimals: u8) -> UtxoRpcFut<Vec<(Address, BigDecimal)>>;

    /// Returns fee estimation per KByte in satoshis.
    fn estimate_fee_sat(
        &self,
        decimals: u8,
        fee_method: &EstimateFeeMethod,
        mode: &Option<EstimateFeeMode>,
        n_blocks: u32,
    ) -> UtxoRpcFut<u64>;

    /// Returns the minimum fee a low-priority transaction must pay in order to be accepted to the daemon’s memory pool.
    fn get_relay_fee(&self) -> RpcRes<BigDecimal>;

    /// Tries to find a transaction that spends the specified `vout` output of the `tx_hash` transaction.
    fn find_output_spend(
        &self,
        tx_hash: H256,
        script_pubkey: &[u8],
        vout: usize,
        from_block: BlockHashOrHeight,
    ) -> Box<dyn Future<Item = Option<SpentOutputInfo>, Error = String> + Send>;

    /// Get median time past for `count` blocks in the past including `starting_block`
    fn get_median_time_past(
        &self,
        starting_block: u64,
        count: NonZeroU64,
        coin_variant: CoinVariant,
    ) -> UtxoRpcFut<u32>;

    /// Returns block time in seconds since epoch (Jan 1 1970 GMT).
    async fn get_block_timestamp(&self, height: u64) -> Result<u64, MmError<UtxoRpcError>>;
}

#[derive(Clone, Deserialize, Debug)]
#[cfg_attr(test, derive(Default))]
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
#[cfg_attr(test, derive(Default))]
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
    #[allow(dead_code)]
    last_block: H256Json,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct NetworkInfoLocalAddress {
    address: String,
    port: u16,
    score: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct NetworkInfoNetwork {
    name: String,
    limited: bool,
    reachable: bool,
    proxy: String,
    proxy_randomize_credentials: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BlockNonce {
    String(String),
    U64(u64),
}

#[derive(Debug, Deserialize)]
pub struct VerboseBlock {
    /// Block hash
    pub hash: H256Json,
    /// Number of confirmations. -1 if block is on the side chain
    pub confirmations: i64,
    /// Block size
    pub size: u32,
    /// Block size, excluding witness data
    pub strippedsize: Option<u32>,
    /// Block weight
    pub weight: Option<u32>,
    /// Block height
    pub height: Option<u32>,
    /// Block version
    pub version: u32,
    /// Block version as hex
    #[serde(rename = "versionHex")]
    pub version_hex: Option<String>,
    /// Merkle root of this block
    pub merkleroot: H256Json,
    /// Transactions ids
    pub tx: Vec<H256Json>,
    /// Block time in seconds since epoch (Jan 1 1970 GMT)
    pub time: u32,
    /// Median block time in seconds since epoch (Jan 1 1970 GMT)
    pub mediantime: Option<u32>,
    /// Block nonce
    pub nonce: BlockNonce,
    /// Block nbits
    pub bits: String,
    /// Block difficulty
    pub difficulty: f64,
    /// Expected number of hashes required to produce the chain up to this block (in hex)
    pub chainwork: H256Json,
    /// Hash of previous block
    pub previousblockhash: Option<H256Json>,
    /// Hash of next block
    pub nextblockhash: Option<H256Json>,
    #[serde(rename = "finalsaplingroot")]
    pub final_sapling_root: Option<H256Json>,
}

pub type RpcReqSub<T> = async_oneshot::Sender<Result<T, JsonRpcError>>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ListUnspentArgs {
    min_conf: i32,
    max_conf: i32,
    addresses: Vec<String>,
}

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
    pub list_unspent_concurrent_map: ConcurrentRequestMap<ListUnspentArgs, Vec<NativeUnspent>>,
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
            list_unspent_concurrent_map: ConcurrentRequestMap::new(),
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

    #[cfg(target_arch = "wasm32")]
    fn transport(&self, _request: JsonRpcRequestEnum) -> JsonRpcResponseFut {
        Box::new(futures01::future::err(ERRL!(
            "'NativeClientImpl' must be used in native mode only"
        )))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn transport(&self, request: JsonRpcRequestEnum) -> JsonRpcResponseFut {
        use mm2_net::transport::slurp_req;

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
            move |result| -> Result<(JsonRpcRemoteAddr, JsonRpcResponseEnum), String> {
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

impl JsonRpcBatchClient for NativeClientImpl {}

// if mockable is placed before async_trait there is `munmap_chunk(): invalid pointer` error on async fn mocking attempt
#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoRpcClientOps for NativeClient {
    fn list_unspent(&self, address: &Address, decimals: u8) -> UtxoRpcFut<Vec<UnspentInfo>> {
        let fut = self
            .list_unspent_impl(0, std::i32::MAX, vec![address.to_string()])
            .map_to_mm_fut(UtxoRpcError::from)
            .and_then(move |unspents| {
                let unspents: UtxoRpcResult<Vec<_>> = unspents
                    .into_iter()
                    .map(|unspent| {
                        Ok(UnspentInfo {
                            outpoint: OutPoint {
                                hash: unspent.txid.reversed().into(),
                                index: unspent.vout,
                            },
                            value: sat_from_big_decimal(&unspent.amount.to_decimal(), decimals)?,
                            height: None,
                        })
                    })
                    .collect();
                unspents
            });
        Box::new(fut)
    }

    fn list_unspent_group(&self, addresses: Vec<Address>, decimals: u8) -> UtxoRpcFut<UnspentMap> {
        let mut addresses_str = Vec::with_capacity(addresses.len());
        let mut addresses_map = HashMap::with_capacity(addresses.len());
        for addr in addresses {
            let addr_str = addr.to_string();
            addresses_str.push(addr_str.clone());
            addresses_map.insert(addr_str, addr);
        }

        let fut = self
            .list_unspent_impl(0, std::i32::MAX, addresses_str)
            .map_to_mm_fut(UtxoRpcError::from)
            .and_then(move |unspents| {
                unspents
                    .into_iter()
                    // Convert `Vec<NativeUnspent>` into `UnspentMap`.
                    .map(|unspent| {
                        let orig_address = addresses_map
                            .get(&unspent.address)
                            .or_mm_err(|| {
                                UtxoRpcError::InvalidResponse(format!("Unexpected address '{}'", unspent.address))
                            })?
                            .clone();
                        let unspent_info = UnspentInfo {
                            outpoint: OutPoint {
                                hash: unspent.txid.reversed().into(),
                                index: unspent.vout,
                            },
                            value: sat_from_big_decimal(&unspent.amount.to_decimal(), decimals)?,
                            height: None,
                        };
                        Ok((orig_address, unspent_info))
                    })
                    // Collect `(Address, UnspentInfo)` items into `HashMap<Address, Vec<UnspentInfo>>` grouped by the addresses.
                    .try_into_group_map()
            });
        Box::new(fut)
    }

    fn send_transaction(&self, tx: &UtxoTx) -> UtxoRpcFut<H256Json> {
        let tx_bytes = if tx.has_witness() {
            BytesJson::from(serialize_with_flags(tx, SERIALIZE_TRANSACTION_WITNESS))
        } else {
            BytesJson::from(serialize(tx))
        };
        Box::new(self.send_raw_transaction(tx_bytes))
    }

    /// https://developer.bitcoin.org/reference/rpc/sendrawtransaction
    fn send_raw_transaction(&self, tx: BytesJson) -> UtxoRpcFut<H256Json> {
        Box::new(rpc_func!(self, "sendrawtransaction", tx).map_to_mm_fut(UtxoRpcError::from))
    }

    fn get_transaction_bytes(&self, txid: &H256Json) -> UtxoRpcFut<BytesJson> {
        Box::new(self.get_raw_transaction_bytes(txid).map_to_mm_fut(UtxoRpcError::from))
    }

    fn get_verbose_transaction(&self, txid: &H256Json) -> UtxoRpcFut<RpcTransaction> {
        Box::new(self.get_raw_transaction_verbose(txid).map_to_mm_fut(UtxoRpcError::from))
    }

    fn get_verbose_transactions(&self, tx_ids: &[H256Json]) -> UtxoRpcFut<Vec<RpcTransaction>> {
        Box::new(
            self.get_raw_transaction_verbose_batch(tx_ids)
                .map_to_mm_fut(UtxoRpcError::from),
        )
    }

    fn get_block_count(&self) -> UtxoRpcFut<u64> {
        Box::new(self.0.get_block_count().map_to_mm_fut(UtxoRpcError::from))
    }

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

    fn display_balances(&self, addresses: Vec<Address>, decimals: u8) -> UtxoRpcFut<Vec<(Address, BigDecimal)>> {
        let this = self.clone();
        let fut = async move {
            let unspent_map = this.list_unspent_group(addresses.clone(), decimals).compat().await?;
            let balances = addresses
                .into_iter()
                .map(|address| {
                    let balance = address_balance_from_unspent_map(&address, &unspent_map, decimals);
                    (address, balance)
                })
                .collect();
            Ok(balances)
        };
        Box::new(fut.boxed().compat())
    }

    fn estimate_fee_sat(
        &self,
        decimals: u8,
        fee_method: &EstimateFeeMethod,
        mode: &Option<EstimateFeeMode>,
        n_blocks: u32,
    ) -> UtxoRpcFut<u64> {
        match fee_method {
            EstimateFeeMethod::Standard => Box::new(self.estimate_fee(n_blocks).map(move |fee| {
                if fee > 0.00001 {
                    (fee * 10.0_f64.powf(decimals as f64)) as u64
                } else {
                    1000
                }
            })),
            EstimateFeeMethod::SmartFee => Box::new(self.estimate_smart_fee(mode, n_blocks).map(move |res| {
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
        tx_hash: H256,
        _script_pubkey: &[u8],
        vout: usize,
        from_block: BlockHashOrHeight,
    ) -> Box<dyn Future<Item = Option<SpentOutputInfo>, Error = String> + Send> {
        let selfi = self.clone();
        let fut = async move {
            let from_block_hash = match from_block {
                BlockHashOrHeight::Height(h) => try_s!(selfi.get_block_hash(h as u64).compat().await),
                BlockHashOrHeight::Hash(h) => h,
            };
            let list_since_block: ListSinceBlockRes = try_s!(selfi.list_since_block(from_block_hash).compat().await);
            for transaction in list_since_block
                .transactions
                .into_iter()
                .filter(|tx| !tx.is_conflicting())
            {
                let maybe_spend_tx_bytes = try_s!(selfi.get_raw_transaction_bytes(&transaction.txid).compat().await);
                let maybe_spend_tx: UtxoTx =
                    try_s!(deserialize(maybe_spend_tx_bytes.as_slice()).map_err(|e| ERRL!("{:?}", e)));

                for (index, input) in maybe_spend_tx.inputs.iter().enumerate() {
                    if input.previous_output.hash == tx_hash && input.previous_output.index == vout as u32 {
                        return Ok(Some(SpentOutputInfo {
                            spending_tx: maybe_spend_tx,
                            input_index: index,
                            spent_in_block: BlockHashOrHeight::Hash(transaction.blockhash),
                        }));
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
        _coin_variant: CoinVariant,
    ) -> UtxoRpcFut<u32> {
        let selfi = self.clone();
        let fut = async move {
            let starting_block_hash = selfi.get_block_hash(starting_block).compat().await?;
            let starting_block_data = selfi.get_block(starting_block_hash).compat().await?;
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
                let block_hash = selfi.get_block_hash(block_n).compat().await?;
                let block_data = selfi.get_block(block_hash).compat().await?;
                block_timestamps.push(block_data.time);
            }
            // can unwrap because count is non zero
            Ok(median(block_timestamps.as_mut_slice()).unwrap())
        };
        Box::new(fut.boxed().compat())
    }

    async fn get_block_timestamp(&self, height: u64) -> Result<u64, MmError<UtxoRpcError>> {
        let block = self.get_block_by_height(height).await?;
        Ok(block.time as u64)
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
        let request_fut = rpc_func!(self, "listunspent", &min_conf, &max_conf, &addresses);
        let arc = self.clone();
        let args = ListUnspentArgs {
            min_conf,
            max_conf,
            addresses,
        };
        let fut = async move { arc.list_unspent_concurrent_map.wrap_request(args, request_fut).await };
        Box::new(fut.boxed().compat())
    }

    pub fn list_all_transactions(&self, step: u64) -> RpcRes<Vec<ListTransactionsItem>> {
        let selfi = self.clone();
        let fut = async move {
            let mut from = 0;
            let mut transaction_list = Vec::new();

            loop {
                let transactions = selfi.list_transactions(step, from).compat().await?;
                if transactions.is_empty() {
                    return Ok(transaction_list);
                }

                transaction_list.extend(transactions.into_iter());
                from += step;
            }
        };
        Box::new(fut.boxed().compat())
    }
}

impl NativeClient {
    pub async fn get_block_by_height(&self, height: u64) -> UtxoRpcResult<VerboseBlock> {
        let block_hash = self.get_block_hash(height).compat().await?;
        self.get_block(block_hash).compat().await
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

    pub fn output_amount(
        &self,
        txid: H256Json,
        index: usize,
    ) -> Box<dyn Future<Item = u64, Error = String> + Send + 'static> {
        let fut = self.get_raw_transaction_bytes(&txid).map_err(|e| ERRL!("{}", e));
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
    pub fn get_block(&self, hash: H256Json) -> UtxoRpcFut<VerboseBlock> {
        let verbose = true;
        Box::new(rpc_func!(self, "getblock", hash, verbose).map_to_mm_fut(UtxoRpcError::from))
    }

    /// https://developer.bitcoin.org/reference/rpc/getblockhash.html
    pub fn get_block_hash(&self, height: u64) -> UtxoRpcFut<H256Json> {
        Box::new(rpc_func!(self, "getblockhash", height).map_to_mm_fut(UtxoRpcError::from))
    }

    /// https://developer.bitcoin.org/reference/rpc/getblockcount.html
    pub fn get_block_count(&self) -> RpcRes<u64> { rpc_func!(self, "getblockcount") }

    /// https://developer.bitcoin.org/reference/rpc/getrawtransaction.html
    /// Always returns verbose transaction
    fn get_raw_transaction_verbose(&self, txid: &H256Json) -> RpcRes<RpcTransaction> {
        let verbose = 1;
        rpc_func!(self, "getrawtransaction", txid, verbose)
    }

    /// https://developer.bitcoin.org/reference/rpc/getrawtransaction.html
    /// Always returns verbose transactions in the same order they were requested.
    fn get_raw_transaction_verbose_batch(&self, tx_ids: &[H256Json]) -> RpcRes<Vec<RpcTransaction>> {
        let verbose = 1;
        let requests = tx_ids
            .iter()
            .map(|txid| rpc_req!(self, "getrawtransaction", txid, verbose));
        self.batch_rpc(requests)
    }

    /// https://developer.bitcoin.org/reference/rpc/getrawtransaction.html
    /// Always returns transaction bytes
    pub fn get_raw_transaction_bytes(&self, txid: &H256Json) -> RpcRes<BytesJson> {
        let verbose = 0;
        rpc_func!(self, "getrawtransaction", txid, verbose)
    }

    /// https://developer.bitcoin.org/reference/rpc/estimatefee.html
    /// It is recommended to set n_blocks as low as possible.
    /// However, in some cases, n_blocks = 1 leads to an unreasonably high fee estimation.
    /// https://github.com/KomodoPlatform/atomicDEX-API/issues/656#issuecomment-743759659
    pub fn estimate_fee(&self, n_blocks: u32) -> UtxoRpcFut<f64> {
        Box::new(rpc_func!(self, "estimatefee", n_blocks).map_to_mm_fut(UtxoRpcError::from))
    }

    /// https://developer.bitcoin.org/reference/rpc/estimatesmartfee.html
    /// It is recommended to set n_blocks as low as possible.
    /// However, in some cases, n_blocks = 1 leads to an unreasonably high fee estimation.
    /// https://github.com/KomodoPlatform/atomicDEX-API/issues/656#issuecomment-743759659
    pub fn estimate_smart_fee(&self, mode: &Option<EstimateFeeMode>, n_blocks: u32) -> UtxoRpcFut<EstimateSmartFeeRes> {
        match mode {
            Some(m) => Box::new(rpc_func!(self, "estimatesmartfee", n_blocks, m).map_to_mm_fut(UtxoRpcError::from)),
            None => Box::new(rpc_func!(self, "estimatesmartfee", n_blocks).map_to_mm_fut(UtxoRpcError::from)),
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
        let estimate_fee_fut = self.estimate_fee(1);
        self.estimate_smart_fee(&None, 1).then(move |res| -> Box<dyn Future<Item=EstimateFeeMethod, Error=String> + Send> {
            match res {
                Ok(smart_fee) => if smart_fee.fee_rate > 0. {
                    Box::new(futures01::future::ok(EstimateFeeMethod::SmartFee))
                } else {
                    info!("fee_rate from smart fee should be above zero, but got {:?}, trying estimatefee", smart_fee);
                    Box::new(estimate_fee_fut.map_err(|e| ERRL!("{}", e)).and_then(|res| if res > 0. {
                        Ok(EstimateFeeMethod::Standard)
                    } else {
                        ERR!("Estimate fee result should be above zero, but got {}, consider setting txfee in config", res)
                    }))
                },
                Err(e) => {
                    error!("Error {} on estimate smart fee, trying estimatefee", e);
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

    /// https://developer.bitcoin.org/reference/rpc/getblockheader.html
    pub fn get_block_header_bytes(&self, block_hash: H256Json) -> RpcRes<BytesJson> {
        let verbose = 0;
        rpc_func!(self, "getblockheader", block_hash, verbose)
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
pub struct ElectrumUnspent {
    pub height: Option<u64>,
    pub tx_hash: H256Json,
    pub tx_pos: u32,
    pub value: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ElectrumNonce {
    Number(u64),
    Hash(H256Json),
}

#[allow(clippy::from_over_into)]
impl Into<BlockHeaderNonce> for ElectrumNonce {
    fn into(self) -> BlockHeaderNonce {
        match self {
            ElectrumNonce::Number(n) => BlockHeaderNonce::U32(n as u32),
            ElectrumNonce::Hash(h) => BlockHeaderNonce::H256(h.into()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ElectrumBlockHeadersRes {
    pub count: u64,
    pub hex: BytesJson,
    #[allow(dead_code)]
    max: u64,
}

/// The block header compatible with Electrum 1.2
#[derive(Clone, Debug, Deserialize)]
pub struct ElectrumBlockHeaderV12 {
    pub bits: u64,
    pub block_height: u64,
    pub merkle_root: H256Json,
    pub nonce: ElectrumNonce,
    pub prev_block_hash: H256Json,
    pub timestamp: u64,
    pub version: u64,
}

impl ElectrumBlockHeaderV12 {
    fn as_block_header(&self) -> BlockHeader {
        BlockHeader {
            version: self.version as u32,
            previous_header_hash: self.prev_block_hash.into(),
            merkle_root_hash: self.merkle_root.into(),
            hash_final_sapling_root: None,
            time: self.timestamp as u32,
            bits: BlockHeaderBits::U32(self.bits as u32),
            nonce: self.nonce.clone().into(),
            solution: None,
            aux_pow: None,
            prog_pow: None,
            mtp_pow: None,
            is_verus: false,
            hash_state_root: None,
            hash_utxo_root: None,
            prevout_stake: None,
            vch_block_sig_dlgt: None,
            n_height: None,
            n_nonce_u64: None,
            mix_hash: None,
        }
    }

    #[inline]
    pub fn as_hex(&self) -> String {
        let block_header = self.as_block_header();
        let serialized = serialize(&block_header);
        hex::encode(serialized)
    }

    #[inline]
    pub fn hash(&self) -> H256Json {
        let block_header = self.as_block_header();
        BlockHeader::hash(&block_header).into()
    }
}

/// The block header compatible with Electrum 1.4
#[derive(Clone, Debug, Deserialize)]
pub struct ElectrumBlockHeaderV14 {
    pub height: u64,
    pub hex: BytesJson,
}

impl ElectrumBlockHeaderV14 {
    pub fn hash(&self) -> H256Json { self.hex.clone().into_vec()[..].into() }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ElectrumBlockHeader {
    V12(ElectrumBlockHeaderV12),
    V14(ElectrumBlockHeaderV14),
}

/// The merkle branch of a confirmed transaction
#[derive(Clone, Debug, Deserialize)]
pub struct TxMerkleBranch {
    pub merkle: Vec<H256Json>,
    pub block_height: u64,
    pub pos: usize,
}

#[derive(Debug, PartialEq)]
pub struct BestBlock {
    pub height: u64,
    pub hash: H256Json,
}

impl From<ElectrumBlockHeader> for BestBlock {
    fn from(block_header: ElectrumBlockHeader) -> Self {
        BestBlock {
            height: block_header.block_height(),
            hash: block_header.block_hash(),
        }
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Deserialize, Serialize)]
pub enum EstimateFeeMode {
    ECONOMICAL,
    CONSERVATIVE,
    UNSET,
}

impl ElectrumBlockHeader {
    pub fn block_height(&self) -> u64 {
        match self {
            ElectrumBlockHeader::V12(h) => h.block_height,
            ElectrumBlockHeader::V14(h) => h.height,
        }
    }

    fn block_hash(&self) -> H256Json {
        match self {
            ElectrumBlockHeader::V12(h) => h.hash(),
            ElectrumBlockHeader::V14(h) => h.hash(),
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
pub struct ElectrumBalance {
    pub(crate) confirmed: i128,
    pub(crate) unconfirmed: i128,
}

impl ElectrumBalance {
    #[inline]
    pub fn to_big_decimal(&self, decimals: u8) -> BigDecimal {
        let balance_sat = BigInt::from(self.confirmed) + BigInt::from(self.unconfirmed);
        BigDecimal::from(balance_sat) / BigDecimal::from(10u64.pow(decimals as u32))
    }
}

#[inline]
fn sha_256(input: &[u8]) -> Vec<u8> {
    let mut sha = Sha256::new();
    sha.update(input);
    sha.finalize().to_vec()
}

#[inline]
pub fn electrum_script_hash(script: &[u8]) -> Vec<u8> {
    let mut result = sha_256(script);
    result.reverse();
    result
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, Deserialize, Serialize)]
/// Deserializable Electrum protocol representation for RPC
pub enum ElectrumProtocol {
    /// TCP
    TCP,
    /// SSL/TLS
    SSL,
    /// Insecure WebSocket.
    WS,
    /// Secure WebSocket.
    WSS,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for ElectrumProtocol {
    fn default() -> Self { ElectrumProtocol::TCP }
}

#[cfg(target_arch = "wasm32")]
impl Default for ElectrumProtocol {
    fn default() -> Self { ElectrumProtocol::WS }
}

#[derive(Debug, Deserialize, Serialize)]
/// Deserializable Electrum protocol version representation for RPC
/// https://electrumx-spesmilo.readthedocs.io/en/latest/protocol-methods.html#server.version
pub struct ElectrumProtocolVersion {
    pub server_software_version: String,
    pub protocol_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
/// Electrum request RPC representation
pub struct ElectrumRpcRequest {
    pub url: String,
    #[serde(default)]
    pub protocol: ElectrumProtocol,
    #[serde(default)]
    pub disable_cert_verification: bool,
}

/// Electrum client configuration
#[allow(clippy::upper_case_acronyms)]
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Serialize)]
enum ElectrumConfig {
    TCP,
    SSL { dns_name: String, skip_validation: bool },
}

/// Electrum client configuration
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Serialize)]
enum ElectrumConfig {
    WS,
    WSS,
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
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_electrum(
    req: &ElectrumRpcRequest,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<ElectrumConnection, String> {
    let config = match req.protocol {
        ElectrumProtocol::TCP => ElectrumConfig::TCP,
        ElectrumProtocol::SSL => {
            let uri: Uri = try_s!(req.url.parse());
            let host = uri
                .host()
                .ok_or(ERRL!("Couldn't retrieve host from addr {}", req.url))?;

            // check the dns name
            try_s!(DnsNameRef::try_from_ascii_str(host));

            ElectrumConfig::SSL {
                dns_name: host.into(),
                skip_validation: req.disable_cert_verification,
            }
        },
        ElectrumProtocol::WS | ElectrumProtocol::WSS => {
            return ERR!("'ws' and 'wss' protocols are not supported yet. Consider using 'TCP' or 'SSL'")
        },
    };

    Ok(electrum_connect(req.url.clone(), config, event_handlers))
}

/// Attempts to process the request (parse url, etc), build up the config and create new electrum connection
#[cfg(target_arch = "wasm32")]
pub fn spawn_electrum(
    req: &ElectrumRpcRequest,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<ElectrumConnection, String> {
    let mut url = req.url.clone();
    let uri: Uri = try_s!(req.url.parse());

    if uri.scheme().is_some() {
        return ERR!(
            "There has not to be a scheme in the url: {}. \
            'ws://' scheme is used by default. \
            Consider using 'protocol: \"WSS\"' in the electrum request to switch to the 'wss://' scheme.",
            url
        );
    }

    let config = match req.protocol {
        ElectrumProtocol::WS => {
            url.insert_str(0, "ws://");
            ElectrumConfig::WS
        },
        ElectrumProtocol::WSS => {
            url.insert_str(0, "wss://");
            ElectrumConfig::WSS
        },
        ElectrumProtocol::TCP | ElectrumProtocol::SSL => {
            return ERR!("'TCP' and 'SSL' are not supported in a browser. Please use 'WS' or 'WSS' protocols");
        },
    };

    Ok(electrum_connect(url, config, event_handlers))
}

#[derive(Debug)]
/// Represents the active Electrum connection to selected address
pub struct ElectrumConnection {
    /// The client connected to this SocketAddr
    addr: String,
    /// Configuration
    #[allow(dead_code)]
    config: ElectrumConfig,
    /// The Sender forwarding requests to writing part of underlying stream
    tx: Arc<AsyncMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    /// The Sender used to shutdown the background connection loop when ElectrumConnection is dropped
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Responses are stored here
    responses: JsonRpcPendingRequestsShared,
    /// Selected protocol version. The value is initialized after the server.version RPC call.
    protocol_version: AsyncMutex<Option<f32>>,
}

impl ElectrumConnection {
    async fn is_connected(&self) -> bool { self.tx.lock().await.is_some() }

    async fn set_protocol_version(&self, version: f32) { self.protocol_version.lock().await.replace(version); }
}

impl Drop for ElectrumConnection {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            if shutdown_tx.send(()).is_err() {
                warn!("electrum_connection_drop] Warning, shutdown_tx already closed");
            }
        }
    }
}

#[derive(Debug)]
struct ConcurrentRequestState<V> {
    is_running: bool,
    subscribers: Vec<RpcReqSub<V>>,
}

impl<V> ConcurrentRequestState<V> {
    fn new() -> Self {
        ConcurrentRequestState {
            is_running: false,
            subscribers: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct ConcurrentRequestMap<K, V> {
    inner: AsyncMutex<HashMap<K, ConcurrentRequestState<V>>>,
}

impl<K, V> Default for ConcurrentRequestMap<K, V> {
    fn default() -> Self {
        ConcurrentRequestMap {
            inner: AsyncMutex::new(HashMap::new()),
        }
    }
}

impl<K: Clone + Eq + std::hash::Hash, V: Clone> ConcurrentRequestMap<K, V> {
    pub fn new() -> ConcurrentRequestMap<K, V> { ConcurrentRequestMap::default() }

    async fn wrap_request(&self, request_arg: K, request_fut: RpcRes<V>) -> Result<V, JsonRpcError> {
        let mut map = self.inner.lock().await;
        let state = map
            .entry(request_arg.clone())
            .or_insert_with(ConcurrentRequestState::new);
        if state.is_running {
            let (tx, rx) = async_oneshot::channel();
            state.subscribers.push(tx);
            // drop here to avoid holding the lock during await
            drop(map);
            rx.await.unwrap()
        } else {
            // drop here to avoid holding the lock during await
            drop(map);
            let request_res = request_fut.compat().await;
            let mut map = self.inner.lock().await;
            let state = map.get_mut(&request_arg).unwrap();
            for sub in state.subscribers.drain(..) {
                if sub.send(request_res.clone()).is_err() {
                    warn!("subscriber is dropped");
                }
            }
            state.is_running = false;
            request_res
        }
    }
}

#[derive(Debug)]
pub struct ElectrumClientImpl {
    coin_ticker: String,
    connections: AsyncMutex<Vec<ElectrumConnection>>,
    next_id: AtomicU64,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    protocol_version: OrdRange<f32>,
    get_balance_concurrent_map: ConcurrentRequestMap<String, ElectrumBalance>,
    list_unspent_concurrent_map: ConcurrentRequestMap<String, Vec<ElectrumUnspent>>,
}

async fn electrum_request_multi(
    client: ElectrumClient,
    request: JsonRpcRequestEnum,
) -> Result<(JsonRpcRemoteAddr, JsonRpcResponseEnum), String> {
    let mut futures = vec![];
    let connections = client.connections.lock().await;
    for (i, connection) in connections.iter().enumerate() {
        let connection_addr = connection.addr.clone();
        match &*connection.tx.lock().await {
            Some(tx) => {
                let fut = electrum_request(
                    request.clone(),
                    tx.clone(),
                    connection.responses.clone(),
                    ELECTRUM_TIMEOUT / (connections.len() - i) as u64,
                )
                .map(|response| (JsonRpcRemoteAddr(connection_addr), response));
                futures.push(fut)
            },
            None => (),
        }
    }
    drop(connections);
    if futures.is_empty() {
        return ERR!("All electrums are currently disconnected");
    }

    match request {
        JsonRpcRequestEnum::Single(single) if single.method == "server.ping" => {
            // server.ping must be sent to all servers to keep all connections alive
            return select_ok(futures)
                .map(|(result, _)| result)
                .map_err(|e| ERRL!("{:?}", e))
                .compat()
                .await;
        },
        _ => (),
    }

    let (res, no_of_failed_requests) = select_ok_sequential(futures)
        .compat()
        .await
        .map_err(|e| ERRL!("{:?}", e))?;
    client.rotate_servers(no_of_failed_requests).await;
    Ok(res)
}

async fn electrum_request_to(
    client: ElectrumClient,
    request: JsonRpcRequestEnum,
    to_addr: String,
) -> Result<(JsonRpcRemoteAddr, JsonRpcResponseEnum), String> {
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

    let response = try_s!(
        electrum_request(request.clone(), tx, responses, ELECTRUM_TIMEOUT)
            .compat()
            .await
    );
    Ok((JsonRpcRemoteAddr(to_addr.to_owned()), response))
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

    /// Moves the Electrum servers that fail in a multi request to the end.
    pub async fn rotate_servers(&self, no_of_rotations: usize) {
        let mut connections = self.connections.lock().await;
        connections.rotate_left(no_of_rotations);
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

    fn transport(&self, request: JsonRpcRequestEnum) -> JsonRpcResponseFut {
        Box::new(electrum_request_multi(self.clone(), request).boxed().compat())
    }
}

impl JsonRpcBatchClient for ElectrumClient {}

impl JsonRpcMultiClient for ElectrumClient {
    fn transport_exact(&self, to_addr: String, request: JsonRpcRequestEnum) -> JsonRpcResponseFut {
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
    pub fn scripthash_list_unspent(&self, hash: &str) -> RpcRes<Vec<ElectrumUnspent>> {
        let request_fut = Box::new(rpc_func!(self, "blockchain.scripthash.listunspent", hash).and_then(
            move |unspents: Vec<ElectrumUnspent>| {
                let mut map: HashMap<(H256Json, u32), bool> = HashMap::new();
                let unspents = unspents
                    .into_iter()
                    .filter(|unspent| match map.entry((unspent.tx_hash, unspent.tx_pos)) {
                        Entry::Occupied(_) => false,
                        Entry::Vacant(e) => {
                            e.insert(true);
                            true
                        },
                    })
                    .collect();
                Ok(unspents)
            },
        ));
        let arc = self.clone();
        let hash = hash.to_owned();
        let fut = async move { arc.list_unspent_concurrent_map.wrap_request(hash, request_fut).await };
        Box::new(fut.boxed().compat())
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-listunspent
    /// It can return duplicates sometimes: https://github.com/artemii235/SuperNET/issues/269
    /// We should remove them to build valid transactions.
    /// Please note the function returns `ScriptHashUnspents` elements in the same order in which they were requested.
    pub fn scripthash_list_unspent_batch(&self, hashes: Vec<ElectrumScriptHash>) -> RpcRes<Vec<ScriptHashUnspents>> {
        let requests = hashes
            .iter()
            .map(|hash| rpc_req!(self, "blockchain.scripthash.listunspent", hash));
        Box::new(self.batch_rpc(requests).map(move |unspents: Vec<ScriptHashUnspents>| {
            unspents
                .into_iter()
                .map(|hash_unspents| {
                    hash_unspents
                        .into_iter()
                        .unique_by(|unspent| (unspent.tx_hash, unspent.tx_pos))
                        .collect::<Vec<_>>()
                })
                .collect()
        }))
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-get-history
    pub fn scripthash_get_history(&self, hash: &str) -> RpcRes<Vec<ElectrumTxHistoryItem>> {
        rpc_func!(self, "blockchain.scripthash.get_history", hash)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-gethistory
    pub fn scripthash_get_balance(&self, hash: &str) -> RpcRes<ElectrumBalance> {
        let arc = self.clone();
        let hash = hash.to_owned();
        let fut = async move {
            let request = rpc_func!(arc, "blockchain.scripthash.get_balance", &hash);
            arc.get_balance_concurrent_map.wrap_request(hash, request).await
        };
        Box::new(fut.boxed().compat())
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-scripthash-gethistory
    /// Requests balances in a batch and returns them in the same order they were requested.
    pub fn scripthash_get_balances<I>(&self, hashes: I) -> RpcRes<Vec<ElectrumBalance>>
    where
        I: IntoIterator<Item = String>,
    {
        let requests = hashes
            .into_iter()
            .map(|hash| rpc_req!(self, "blockchain.scripthash.get_balance", &hash));
        self.batch_rpc(requests)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-headers-subscribe
    pub fn blockchain_headers_subscribe(&self) -> RpcRes<ElectrumBlockHeader> {
        rpc_func!(self, "blockchain.headers.subscribe")
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-broadcast
    pub fn blockchain_transaction_broadcast(&self, tx: BytesJson) -> RpcRes<H256Json> {
        rpc_func!(self, "blockchain.transaction.broadcast", tx)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-estimatefee
    /// It is recommended to set n_blocks as low as possible.
    /// However, in some cases, n_blocks = 1 leads to an unreasonably high fee estimation.
    /// https://github.com/KomodoPlatform/atomicDEX-API/issues/656#issuecomment-743759659
    pub fn estimate_fee(&self, mode: &Option<EstimateFeeMode>, n_blocks: u32) -> UtxoRpcFut<f64> {
        match mode {
            Some(m) => {
                Box::new(rpc_func!(self, "blockchain.estimatefee", n_blocks, m).map_to_mm_fut(UtxoRpcError::from))
            },
            None => Box::new(rpc_func!(self, "blockchain.estimatefee", n_blocks).map_to_mm_fut(UtxoRpcError::from)),
        }
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-block-header
    pub fn blockchain_block_header(&self, height: u64) -> RpcRes<BytesJson> {
        rpc_func!(self, "blockchain.block.header", height)
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-block-headers
    pub fn blockchain_block_headers(&self, start_height: u64, count: NonZeroU64) -> RpcRes<ElectrumBlockHeadersRes> {
        rpc_func!(self, "blockchain.block.headers", start_height, count)
    }

    pub fn retrieve_last_headers(
        &self,
        blocks_limit_to_check: NonZeroU64,
        block_height: u64,
    ) -> UtxoRpcFut<(HashMap<u64, BlockHeader>, Vec<BlockHeader>)> {
        let (from, count) = {
            let from = if block_height < blocks_limit_to_check.get() {
                0
            } else {
                block_height - blocks_limit_to_check.get()
            };
            (from, blocks_limit_to_check)
        };
        Box::new(
            self.blockchain_block_headers(from, count)
                .map_to_mm_fut(UtxoRpcError::from)
                .and_then(move |headers| {
                    let (block_registry, block_headers) = {
                        if headers.count == 0 {
                            return MmError::err(UtxoRpcError::Internal("No headers available".to_string()));
                        }
                        let len = CompactInteger::from(headers.count);
                        let mut serialized = serialize(&len).take();
                        serialized.extend(headers.hex.0.into_iter());
                        let mut reader = Reader::new_with_coin_variant(serialized.as_slice(), CoinVariant::Standard);
                        let maybe_block_headers = reader.read_list::<BlockHeader>();
                        let block_headers = match maybe_block_headers {
                            Ok(headers) => headers,
                            Err(e) => return MmError::err(UtxoRpcError::InvalidResponse(format!("{:?}", e))),
                        };
                        let mut block_registry: HashMap<u64, BlockHeader> = HashMap::new();
                        let mut starting_height = from;
                        for block_header in &block_headers {
                            block_registry.insert(starting_height, block_header.clone());
                            starting_height += 1;
                        }
                        (block_registry, block_headers)
                    };
                    Ok((block_registry, block_headers))
                }),
        )
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-get-merkle
    pub fn blockchain_transaction_get_merkle(&self, txid: H256Json, height: u64) -> RpcRes<TxMerkleBranch> {
        rpc_func!(self, "blockchain.transaction.get_merkle", txid, height)
    }
}

// if mockable is placed before async_trait there is `munmap_chunk(): invalid pointer` error on async fn mocking attempt
#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoRpcClientOps for ElectrumClient {
    fn list_unspent(&self, address: &Address, _decimals: u8) -> UtxoRpcFut<Vec<UnspentInfo>> {
        let script = output_script(address, ScriptType::P2PKH);
        let script_hash = electrum_script_hash(&script);
        Box::new(
            self.scripthash_list_unspent(&hex::encode(script_hash))
                .map_to_mm_fut(UtxoRpcError::from)
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

    fn list_unspent_group(&self, addresses: Vec<Address>, _decimals: u8) -> UtxoRpcFut<UnspentMap> {
        let script_hashes = addresses
            .iter()
            .map(|addr| {
                let script = output_script(addr, ScriptType::P2PKH);
                let script_hash = electrum_script_hash(&script);
                hex::encode(script_hash)
            })
            .collect();

        let this = self.clone();
        let fut = async move {
            let unspents = this.scripthash_list_unspent_batch(script_hashes).compat().await?;

            let unspent_map = addresses
                .into_iter()
                // `scripthash_list_unspent_batch` returns `ScriptHashUnspents` elements in the same order in which they were requested.
                // So we can zip `addresses` and `unspents` into one iterator.
                .zip(unspents)
                // Map `(Address, Vec<ElectrumUnspent>)` pairs into `(Address, Vec<UnspentInfo>)`.
                .map(|(address, electrum_unspents)| (address, electrum_unspents.collect_into()))
                .collect();
            Ok(unspent_map)
        };
        Box::new(fut.boxed().compat())
    }

    fn send_transaction(&self, tx: &UtxoTx) -> UtxoRpcFut<H256Json> {
        let bytes = if tx.has_witness() {
            BytesJson::from(serialize_with_flags(tx, SERIALIZE_TRANSACTION_WITNESS))
        } else {
            BytesJson::from(serialize(tx))
        };
        Box::new(
            self.blockchain_transaction_broadcast(bytes)
                .map_to_mm_fut(UtxoRpcError::from),
        )
    }

    fn send_raw_transaction(&self, tx: BytesJson) -> UtxoRpcFut<H256Json> {
        Box::new(
            self.blockchain_transaction_broadcast(tx)
                .map_to_mm_fut(UtxoRpcError::from),
        )
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-get
    /// returns transaction bytes by default
    fn get_transaction_bytes(&self, txid: &H256Json) -> UtxoRpcFut<BytesJson> {
        let verbose = false;
        Box::new(rpc_func!(self, "blockchain.transaction.get", txid, verbose).map_to_mm_fut(UtxoRpcError::from))
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-get
    /// returns verbose transaction by default
    fn get_verbose_transaction(&self, txid: &H256Json) -> UtxoRpcFut<RpcTransaction> {
        let verbose = true;
        Box::new(rpc_func!(self, "blockchain.transaction.get", txid, verbose).map_to_mm_fut(UtxoRpcError::from))
    }

    /// https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-transaction-get
    /// Returns verbose transactions in a batch.
    fn get_verbose_transactions(&self, tx_ids: &[H256Json]) -> UtxoRpcFut<Vec<RpcTransaction>> {
        let verbose = true;
        let requests = tx_ids
            .iter()
            .map(|txid| rpc_req!(self, "blockchain.transaction.get", txid, verbose));
        Box::new(self.batch_rpc(requests).map_to_mm_fut(UtxoRpcError::from))
    }

    fn get_block_count(&self) -> UtxoRpcFut<u64> {
        Box::new(
            self.blockchain_headers_subscribe()
                .map(|r| r.block_height())
                .map_to_mm_fut(UtxoRpcError::from),
        )
    }

    fn display_balance(&self, address: Address, decimals: u8) -> RpcRes<BigDecimal> {
        let hash = electrum_script_hash(&output_script(&address, ScriptType::P2PKH));
        let hash_str = hex::encode(hash);
        Box::new(
            self.scripthash_get_balance(&hash_str)
                .map(move |electrum_balance| electrum_balance.to_big_decimal(decimals)),
        )
    }

    fn display_balances(&self, addresses: Vec<Address>, decimals: u8) -> UtxoRpcFut<Vec<(Address, BigDecimal)>> {
        let this = self.clone();
        let fut = async move {
            let hashes = addresses.iter().map(|address| {
                let hash = electrum_script_hash(&output_script(address, ScriptType::P2PKH));
                hex::encode(hash)
            });

            let electrum_balances = this.scripthash_get_balances(hashes).compat().await?;
            let balances = electrum_balances
                .into_iter()
                // `scripthash_get_balances` returns `ElectrumBalance` elements in the same order in which they were requested.
                // So we can zip `addresses` and the balances into one iterator.
                .zip(addresses)
                .map(|(electrum_balance, address)| (address, electrum_balance.to_big_decimal(decimals)))
                .collect();
            Ok(balances)
        };

        Box::new(fut.boxed().compat())
    }

    fn estimate_fee_sat(
        &self,
        decimals: u8,
        _fee_method: &EstimateFeeMethod,
        mode: &Option<EstimateFeeMode>,
        n_blocks: u32,
    ) -> UtxoRpcFut<u64> {
        Box::new(self.estimate_fee(mode, n_blocks).map(move |fee| {
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
        tx_hash: H256,
        script_pubkey: &[u8],
        vout: usize,
        _from_block: BlockHashOrHeight,
    ) -> Box<dyn Future<Item = Option<SpentOutputInfo>, Error = String> + Send> {
        let selfi = self.clone();
        let script_hash = hex::encode(electrum_script_hash(script_pubkey));
        let fut = async move {
            let history = try_s!(selfi.scripthash_get_history(&script_hash).compat().await);

            if history.len() < 2 {
                return Ok(None);
            }

            for item in history.iter() {
                let transaction = try_s!(selfi.get_transaction_bytes(&item.tx_hash).compat().await);

                let maybe_spend_tx: UtxoTx = try_s!(deserialize(transaction.as_slice()).map_err(|e| ERRL!("{:?}", e)));

                for (index, input) in maybe_spend_tx.inputs.iter().enumerate() {
                    if input.previous_output.hash == tx_hash && input.previous_output.index == vout as u32 {
                        return Ok(Some(SpentOutputInfo {
                            spending_tx: maybe_spend_tx,
                            input_index: index,
                            spent_in_block: BlockHashOrHeight::Height(item.height),
                        }));
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
        coin_variant: CoinVariant,
    ) -> UtxoRpcFut<u32> {
        let from = if starting_block <= count.get() {
            0
        } else {
            starting_block - count.get() + 1
        };
        Box::new(
            self.blockchain_block_headers(from, count)
                .map_to_mm_fut(UtxoRpcError::from)
                .and_then(|res| {
                    if res.count == 0 {
                        return MmError::err(UtxoRpcError::InvalidResponse("Server returned zero count".to_owned()));
                    }
                    let len = CompactInteger::from(res.count);
                    let mut serialized = serialize(&len).take();
                    serialized.extend(res.hex.0.into_iter());
                    let mut reader = Reader::new_with_coin_variant(serialized.as_slice(), coin_variant);
                    let headers = reader.read_list::<BlockHeader>()?;
                    let mut timestamps: Vec<_> = headers.into_iter().map(|block| block.time).collect();
                    // can unwrap because count is non zero
                    Ok(median(timestamps.as_mut_slice()).unwrap())
                }),
        )
    }

    async fn get_block_timestamp(&self, height: u64) -> Result<u64, MmError<UtxoRpcError>> {
        let header_bytes = self.blockchain_block_header(height).compat().await?;
        let header: BlockHeader =
            deserialize(header_bytes.0.as_slice()).map_to_mm(|e| UtxoRpcError::InvalidResponse(format!("{:?}", e)))?;
        Ok(header.time as u64)
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
            get_balance_concurrent_map: ConcurrentRequestMap::new(),
            list_unspent_concurrent_map: ConcurrentRequestMap::new(),
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

async fn electrum_process_json(raw_json: Json, arc: &JsonRpcPendingRequestsShared) {
    // detect if we got standard JSONRPC response or subscription response as JSONRPC request
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ElectrumRpcResponseEnum {
        /// The standard JSONRPC single response.
        SingleResponse(JsonRpcResponse),
        /// The batch of standard JSONRPC responses.
        BatchResponses(JsonRpcBatchResponse),
        /// The subscription response as JSONRPC request.
        SubscriptionNotification(JsonRpcRequest),
    }

    let response: ElectrumRpcResponseEnum = match json::from_value(raw_json) {
        Ok(res) => res,
        Err(e) => {
            error!("{}", e);
            return;
        },
    };

    let response = match response {
        ElectrumRpcResponseEnum::SingleResponse(single) => JsonRpcResponseEnum::Single(single),
        ElectrumRpcResponseEnum::BatchResponses(batch) => JsonRpcResponseEnum::Batch(batch),
        ElectrumRpcResponseEnum::SubscriptionNotification(req) => {
            let id = match req.method.as_ref() {
                BLOCKCHAIN_HEADERS_SUB_ID => BLOCKCHAIN_HEADERS_SUB_ID,
                _ => {
                    error!("Couldn't get id of request {:?}", req);
                    return;
                },
            };
            JsonRpcResponseEnum::Single(JsonRpcResponse {
                id: id.into(),
                jsonrpc: "2.0".into(),
                result: req.params[0].clone(),
                error: Json::Null,
            })
        },
    };

    // the corresponding sender may not exist, receiver may be dropped
    // these situations are not considered as errors so we just silently skip them
    let mut pending = arc.lock().await;
    if let Some(tx) = pending.remove(&response.rpc_id()) {
        tx.send(response).ok();
    }
}

async fn electrum_process_chunk(chunk: &[u8], arc: &JsonRpcPendingRequestsShared) {
    // we should split the received chunk because we can get several responses in 1 chunk.
    let split = chunk.split(|item| *item == b'\n');
    for chunk in split {
        // split returns empty slice if it ends with separator which is our case
        if !chunk.is_empty() {
            let raw_json: Json = match json::from_slice(chunk) {
                Ok(json) => json,
                Err(e) => {
                    error!("{}", e);
                    return;
                },
            };
            electrum_process_json(raw_json, arc).await
        }
    }
}

fn increase_delay(delay: &AtomicU64) {
    if delay.load(AtomicOrdering::Relaxed) < 60 {
        delay.fetch_add(5, AtomicOrdering::Relaxed);
    }
}

macro_rules! try_loop {
    ($e:expr, $addr: ident, $delay: ident) => {
        match $e {
            Ok(res) => res,
            Err(e) => {
                error!("{:?} error {:?}", $addr, e);
                increase_delay(&$delay);
                continue;
            },
        }
    };
}

/// The enum wrapping possible variants of underlying Streams
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::large_enum_variant)]
enum ElectrumStream {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
}

#[cfg(not(target_arch = "wasm32"))]
impl AsRef<TcpStream> for ElectrumStream {
    fn as_ref(&self) -> &TcpStream {
        match self {
            ElectrumStream::Tcp(stream) => stream,
            ElectrumStream::Tls(stream) => stream.get_ref().0,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AsyncRead for ElectrumStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ElectrumStream::Tcp(stream) => AsyncRead::poll_read(Pin::new(stream), cx, buf),
            ElectrumStream::Tls(stream) => AsyncRead::poll_read(Pin::new(stream), cx, buf),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
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
            warn!(
                "Didn't receive any data since {}. Shutting down the connection.",
                last as i64
            );
            break;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn rustls_client_config(unsafe_conf: bool) -> Arc<ClientConfig> {
    let mut cert_store = RootCertStore::empty();

    cert_store.add_server_trust_anchors(
        TLS_SERVER_ROOTS
            .0
            .iter()
            .map(|ta| OwnedTrustAnchor::from_subject_spki_name_constraints(ta.subject, ta.spki, ta.name_constraints)),
    );

    let mut tls_config = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(cert_store)
        .with_no_client_auth();

    if unsafe_conf {
        tls_config
            .dangerous()
            .set_certificate_verifier(Arc::new(NoCertificateVerification {}));
    }
    Arc::new(tls_config)
}

#[cfg(not(target_arch = "wasm32"))]
lazy_static! {
    static ref SAFE_TLS_CONFIG: Arc<ClientConfig> = rustls_client_config(false);
    static ref UNSAFE_TLS_CONFIG: Arc<ClientConfig> = rustls_client_config(true);
}

#[cfg(not(target_arch = "wasm32"))]
async fn connect_loop(
    config: ElectrumConfig,
    addr: String,
    responses: JsonRpcPendingRequestsShared,
    connection_tx: Arc<AsyncMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<(), ()> {
    let delay = Arc::new(AtomicU64::new(0));

    loop {
        let current_delay = delay.load(AtomicOrdering::Relaxed);
        if current_delay > 0 {
            Timer::sleep(current_delay as f64).await;
        };

        let socket_addr = try_loop!(addr_to_socket_addr(&addr), addr, delay);

        let connect_f = match config.clone() {
            ElectrumConfig::TCP => Either::Left(TcpStream::connect(&socket_addr).map_ok(ElectrumStream::Tcp)),
            ElectrumConfig::SSL {
                dns_name,
                skip_validation,
            } => {
                let tls_connector = if skip_validation {
                    TlsConnector::from(UNSAFE_TLS_CONFIG.clone())
                } else {
                    TlsConnector::from(SAFE_TLS_CONFIG.clone())
                };

                Either::Right(TcpStream::connect(&socket_addr).and_then(move |stream| {
                    // Can use `unwrap` cause `dns_name` is pre-checked.
                    let dns = ServerName::try_from(dns_name.as_str())
                        .map_err(|e| format!("{:?}", e))
                        .unwrap();
                    tls_connector.connect(dns, stream).map_ok(ElectrumStream::Tls)
                }))
            },
        };

        let stream = try_loop!(connect_f.await, addr, delay);
        try_loop!(stream.as_ref().set_nodelay(true), addr, delay);
        info!("Electrum client connected to {}", addr);
        try_loop!(event_handlers.on_connected(addr.clone()), addr, delay);
        let last_chunk = Arc::new(AtomicU64::new(now_ms()));
        let mut last_chunk_f = electrum_last_chunk_loop(last_chunk.clone()).boxed().fuse();

        let (tx, rx) = mpsc::channel(0);
        *connection_tx.lock().await = Some(tx);
        let rx = rx_to_stream(rx).inspect(|data| {
            // measure the length of each sent packet
            event_handlers.on_outgoing_request(data);
        });

        let (read, mut write) = tokio::io::split(stream);
        let recv_f = {
            let delay = delay.clone();
            let addr = addr.clone();
            let responses = responses.clone();
            let event_handlers = event_handlers.clone();
            async move {
                let mut buffer = String::with_capacity(1024);
                let mut buf_reader = BufReader::new(read);
                loop {
                    match buf_reader.read_line(&mut buffer).await {
                        Ok(c) => {
                            if c == 0 {
                                info!("EOF from {}", addr);
                                break;
                            }
                            // reset the delay if we've connected successfully and only if we received some data from connection
                            delay.store(0, AtomicOrdering::Relaxed);
                        },
                        Err(e) => {
                            error!("Error on read {} from {}", e, addr);
                            break;
                        },
                    };
                    // measure the length of each incoming packet
                    event_handlers.on_incoming_response(buffer.as_bytes());
                    last_chunk.store(now_ms(), AtomicOrdering::Relaxed);

                    electrum_process_chunk(buffer.as_bytes(), &responses).await;
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
                        error!("Write error {} to {}", e, addr);
                    }
                }
            }
        };
        let mut send_f = Box::pin(send_f).fuse();
        macro_rules! reset_tx_and_continue {
            () => {
                info!("{} connection dropped", addr);
                *connection_tx.lock().await = None;
                increase_delay(&delay);
                continue;
            };
        }

        select! {
            _last_chunk = last_chunk_f => { reset_tx_and_continue!(); },
            _recv = recv_f => { reset_tx_and_continue!(); },
            _send = send_f => { reset_tx_and_continue!(); },
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn connect_loop(
    _config: ElectrumConfig,
    addr: String,
    responses: JsonRpcPendingRequestsShared,
    connection_tx: Arc<AsyncMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<(), ()> {
    use std::sync::atomic::AtomicUsize;

    lazy_static! {
        static ref CONN_IDX: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    }

    use mm2_net::wasm_ws::ws_transport;

    let delay = Arc::new(AtomicU64::new(0));
    loop {
        let current_delay = delay.load(AtomicOrdering::Relaxed);
        if current_delay > 0 {
            Timer::sleep(current_delay as f64).await;
        }

        let conn_idx = CONN_IDX.fetch_add(1, AtomicOrdering::Relaxed);
        let (mut transport_tx, mut transport_rx) = try_loop!(ws_transport(conn_idx, &addr).await, addr, delay);

        info!("Electrum client connected to {}", addr);
        try_loop!(event_handlers.on_connected(addr.clone()), addr, delay);

        let last_chunk = Arc::new(AtomicU64::new(now_ms()));
        let mut last_chunk_fut = electrum_last_chunk_loop(last_chunk.clone()).boxed().fuse();

        let (outgoing_tx, outgoing_rx) = mpsc::channel(0);
        *connection_tx.lock().await = Some(outgoing_tx);

        let incoming_fut = {
            let delay = delay.clone();
            let addr = addr.clone();
            let responses = responses.clone();
            let event_handlers = event_handlers.clone();
            async move {
                while let Some(incoming_res) = transport_rx.next().await {
                    last_chunk.store(now_ms(), AtomicOrdering::Relaxed);
                    match incoming_res {
                        Ok(incoming_json) => {
                            // reset the delay if we've connected successfully and only if we received some data from connection
                            delay.store(0, AtomicOrdering::Relaxed);
                            // measure the length of each incoming packet
                            let incoming_str = incoming_json.to_string();
                            event_handlers.on_incoming_response(incoming_str.as_bytes());

                            electrum_process_json(incoming_json, &responses).await;
                        },
                        Err(e) => {
                            error!("{} error: {:?}", addr, e);
                        },
                    }
                }
            }
        };
        let mut incoming_fut = Box::pin(incoming_fut).fuse();

        let outgoing_fut = {
            let addr = addr.clone();
            let mut outgoing_rx = rx_to_stream(outgoing_rx).compat();
            let event_handlers = event_handlers.clone();
            async move {
                while let Some(Ok(data)) = outgoing_rx.next().await {
                    let raw_json: Json = match json::from_slice(&data) {
                        Ok(js) => js,
                        Err(e) => {
                            error!("Error {} deserializing the outgoing data: {:?}", e, data);
                            continue;
                        },
                    };
                    // measure the length of each sent packet
                    event_handlers.on_outgoing_request(&data);

                    if let Err(e) = transport_tx.send(raw_json).await {
                        error!("Error sending to {}: {:?}", addr, e);
                    }
                }
            }
        };
        let mut outgoing_fut = Box::pin(outgoing_fut).fuse();

        macro_rules! reset_tx_and_continue {
            () => {
                info!("{} connection dropped", addr);
                *connection_tx.lock().await = None;
                increase_delay(&delay);
                continue;
            };
        }

        select! {
            _last_chunk = last_chunk_fut => { reset_tx_and_continue!(); },
            _incoming = incoming_fut => { reset_tx_and_continue!(); },
            _outgoing = outgoing_fut => { reset_tx_and_continue!(); },
        }
    }
}

/// Builds up the electrum connection, spawns endless loop that attempts to reconnect to the server
/// in case of connection errors
fn electrum_connect(
    addr: String,
    config: ElectrumConfig,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> ElectrumConnection {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let responses = Arc::new(AsyncMutex::new(JsonRpcPendingRequests::default()));
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
        protocol_version: AsyncMutex::new(None),
    }
}

fn electrum_request(
    request: JsonRpcRequestEnum,
    tx: mpsc::Sender<Vec<u8>>,
    responses: JsonRpcPendingRequestsShared,
    timeout: u64,
) -> Box<dyn Future<Item = JsonRpcResponseEnum, Error = String> + Send + 'static> {
    let send_fut = async move {
        let mut json = try_s!(json::to_string(&request));
        #[cfg(not(target_arch = "wasm"))]
        {
            // Electrum request and responses must end with \n
            // https://electrumx.readthedocs.io/en/latest/protocol-basics.html#message-stream
            json.push('\n');
        }

        let (req_tx, resp_rx) = async_oneshot::channel();
        responses.lock().await.insert(request.rpc_id(), req_tx);
        try_s!(tx.send(json.into_bytes()).compat().await);
        let resps = try_s!(resp_rx.await);
        Ok(resps)
    };
    let send_fut = send_fut
        .boxed()
        .timeout(Duration::from_secs(timeout))
        .compat()
        .then(|res| match res {
            Ok(response) => response,
            Err(timeout_error) => ERR!("{}", timeout_error),
        })
        .map_err(|e| ERRL!("{}", e));
    Box::new(send_fut)
}

fn address_balance_from_unspent_map(address: &Address, unspent_map: &UnspentMap, decimals: u8) -> BigDecimal {
    let unspents = match unspent_map.get(address) {
        Some(unspents) => unspents,
        // If `balances` doesn't contain `address`, there are no unspents related to the address.
        // Consider the balance of that address equal to 0.
        None => return BigDecimal::from(0),
    };
    unspents.iter().fold(BigDecimal::from(0), |sum, unspent| {
        sum + big_decimal_from_sat_unsigned(unspent.value, decimals)
    })
}

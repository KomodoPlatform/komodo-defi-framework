use crate::utxo::rpc_clients::{NativeClient, UtxoRpcClientEnum, UtxoRpcError, UtxoRpcFut};
use bigdecimal::BigDecimal;
use common::jsonrpc_client::{JsonRpcClient, JsonRpcRequest};
use common::mm_error::prelude::*;
use common::mm_number::MmNumber;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json, H264 as H264Json};
use serde_json::{self as json, Value as Json};

#[derive(Debug, Serialize)]
pub struct ZSendManyItem {
    pub amount: BigDecimal,
    #[serde(rename = "opreturn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_return: Option<BytesJson>,
    pub address: String,
}

#[derive(Debug, Deserialize)]
pub struct ZOperationTxid {
    pub txid: H256Json,
}

#[derive(Debug, Deserialize)]
pub struct ZOperationHex {
    pub hex: BytesJson,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status")]
#[serde(rename_all = "lowercase")]
pub enum ZOperationStatus<T> {
    Queued {
        id: String,
        creation_time: u64,
        method: String,
        params: Json,
    },
    Executing {
        id: String,
        creation_time: u64,
        method: String,
        params: Json,
    },
    Success {
        id: String,
        creation_time: u64,
        result: T,
        execution_secs: f64,
        method: String,
        params: Json,
    },
    Failed {
        id: String,
        creation_time: u64,
        method: String,
        params: Json,
        error: Json,
    },
}

#[derive(Debug, Serialize)]
pub struct ZSendManyHtlcParams {
    pub pubkey: H264Json,
    pub refund_pubkey: H264Json,
    pub secret_hash: BytesJson,
    pub input_txid: H256Json,
    pub input_index: usize,
    pub input_amount: BigDecimal,
    pub locktime: u32,
}

#[derive(Debug, Deserialize)]
pub struct ZUnspent {
    pub txid: H256Json,
    #[serde(rename = "outindex")]
    pub out_index: u32,
    pub confirmations: u32,
    #[serde(rename = "raw_confirmations")]
    pub raw_confirmations: Option<u32>,
    pub spendable: bool,
    pub address: String,
    pub amount: MmNumber,
    pub memo: BytesJson,
    pub change: bool,
}

pub trait ZRpcOps {
    fn z_get_balance(&self, address: &str, min_conf: u32) -> UtxoRpcFut<MmNumber>;

    fn z_get_send_many_status(&self, op_ids: &[&str]) -> UtxoRpcFut<Vec<ZOperationStatus<ZOperationTxid>>>;

    fn z_list_unspent(
        &self,
        min_conf: u32,
        max_conf: u32,
        watch_only: bool,
        addresses: &[&str],
    ) -> UtxoRpcFut<Vec<ZUnspent>>;

    fn z_send_many(&self, from_address: &str, send_to: Vec<ZSendManyItem>) -> UtxoRpcFut<String>;

    fn z_import_key(&self, key: &str) -> UtxoRpcFut<()>;
}

impl ZRpcOps for NativeClient {
    fn z_get_balance(&self, address: &str, min_conf: u32) -> UtxoRpcFut<MmNumber> {
        let fut = rpc_func!(self, "z_getbalance", address, min_conf);
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }

    fn z_get_send_many_status(&self, op_ids: &[&str]) -> UtxoRpcFut<Vec<ZOperationStatus<ZOperationTxid>>> {
        let fut = rpc_func!(self, "z_getoperationstatus", op_ids);
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }

    fn z_list_unspent(
        &self,
        min_conf: u32,
        max_conf: u32,
        watch_only: bool,
        addresses: &[&str],
    ) -> UtxoRpcFut<Vec<ZUnspent>> {
        let fut = rpc_func!(self, "z_listunspent", min_conf, max_conf, watch_only, addresses);
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }

    fn z_send_many(&self, from_address: &str, send_to: Vec<ZSendManyItem>) -> UtxoRpcFut<String> {
        let fut = rpc_func!(self, "z_sendmany", from_address, send_to);
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }

    fn z_import_key(&self, key: &str) -> UtxoRpcFut<()> {
        let fut = rpc_func!(self, "z_importkey", key, "no");
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }
}

impl AsRef<dyn ZRpcOps + Send + Sync> for UtxoRpcClientEnum {
    fn as_ref(&self) -> &(dyn ZRpcOps + Send + Sync + 'static) {
        match self {
            UtxoRpcClientEnum::Native(native) => native,
            UtxoRpcClientEnum::Electrum(_) => panic!("Electrum client does not support ZRpcOps"),
        }
    }
}

mod z_coin_grpc {
    tonic::include_proto!("cash.z.wallet.sdk.rpc");
}

#[test]
// This is a temporary test used to experiment with librustzcash and lightwalletd
fn try_grpc() {
    use common::block_on;
    use db_common::sqlite::rusqlite::{self, Connection, NO_PARAMS};
    use prost::Message;
    use rustls::ClientConfig;
    use tonic::transport::{Channel, ClientTlsConfig};
    use z_coin_grpc::compact_tx_streamer_client::CompactTxStreamerClient;
    use z_coin_grpc::{BlockId, BlockRange};
    use zcash_client_backend::data_api::{chain::{scan_cached_blocks, validate_chain},
                                         error::Error,
                                         BlockSource, WalletRead, WalletWrite};
    use zcash_client_sqlite::{error::SqliteClientError, wallet::init::init_wallet_db, wallet::rewind_to_height,
                              BlockDb, WalletDb};
    use zcash_primitives::consensus::{BlockHeight, Network, Parameters};

    pub fn init_cache_database(db_cache: &Connection) -> Result<(), rusqlite::Error> {
        db_cache.execute(
            "CREATE TABLE IF NOT EXISTS compactblocks (
            height INTEGER PRIMARY KEY,
            data BLOB NOT NULL
        )",
            NO_PARAMS,
        )?;
        Ok(())
    }

    fn insert_into_cache(db_cache: &Connection, height: u32, cb_bytes: Vec<u8>) {
        db_cache
            .prepare("INSERT INTO compactblocks (height, data) VALUES (?, ?)")
            .unwrap()
            .execute(db_common::sqlite::rusqlite::params![height, cb_bytes])
            .unwrap();
    }

    let mut config = ClientConfig::new();
    config
        .root_store
        .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
    config.set_protocols(&["h2".to_string().into()]);
    let tls = ClientTlsConfig::new().rustls_client_config(config);

    let channel = block_on(
        Channel::from_static("http://zombie.sirseven.me:443")
            .tls_config(tls)
            .unwrap()
            .connect(),
    )
    .unwrap();
    let mut client = CompactTxStreamerClient::new(channel);

    let request = tonic::Request::new(BlockRange {
        start: Some(BlockId {
            height: 1,
            hash: Vec::new(),
        }),
        end: Some(BlockId {
            height: 31598,
            hash: Vec::new(),
        }),
    });

    let mut response = block_on(client.get_block_range(request)).unwrap();

    let cache_file = "test_cache_zombie.db";
    let cache_sql = Connection::open(cache_file).unwrap();
    init_cache_database(&cache_sql).unwrap();

    while let Ok(Some(block)) = block_on(response.get_mut().message()) {
        insert_into_cache(&cache_sql, block.height as u32, block.encode_to_vec());
    }
    drop(cache_sql);

    let network = Network::TestNetwork;
    let db_cache = BlockDb::for_path(cache_file).unwrap();
    let db_file = "test_wallet_zombie.db";
    let db_read = WalletDb::for_path(db_file, network).unwrap();
    init_wallet_db(&db_read).unwrap();

    let mut db_data = db_read.get_update_ops().unwrap();

    // 1) Download new CompactBlocks into db_cache.

    // 2) Run the chain validator on the received blocks.
    //
    // Given that we assume the server always gives us correct-at-the-time blocks, any
    // errors are in the blocks we have previously cached or scanned.
    if let Err(e) = validate_chain(&network, &db_cache, db_data.get_max_height_hash().unwrap()) {
        match e {
            SqliteClientError::BackendError(Error::InvalidChain(lower_bound, _)) => {
                // a) Pick a height to rewind to.
                //
                // This might be informed by some external chain reorg information, or
                // heuristics such as the platform, available bandwidth, size of recent
                // CompactBlocks, etc.
                let rewind_height = lower_bound - 10;

                // b) Rewind scanned block information.
                db_data.rewind_to_height(rewind_height);
                // c) Delete cached blocks from rewind_height onwards.
                //
                // This does imply that assumed-valid blocks will be re-downloaded, but it
                // is also possible that in the intervening time, a chain reorg has
                // occurred that orphaned some of those blocks.

                // d) If there is some separate thread or service downloading
                // CompactBlocks, tell it to go back and download from rewind_height
                // onwards.
            },
            e => {
                // Handle or return other errors.
                panic!("{:?}", e);
            },
        }
    }

    // 3) Scan (any remaining) cached blocks.
    //
    // At this point, the cache and scanned data are locally consistent (though not
    // necessarily consistent with the latest chain tip - this would be discovered the
    // next time this codepath is executed after new blocks are received).
    scan_cached_blocks(&network, &db_cache, &mut db_data, None).unwrap();
}

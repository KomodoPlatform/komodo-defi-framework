use crate::utxo::rpc_clients::{NativeClient, UtxoRpcClientEnum, UtxoRpcError, UtxoRpcFut};
use bigdecimal::BigDecimal;
use common::async_blocking;
use common::jsonrpc_client::{JsonRpcClient, JsonRpcRequest};
use common::mm_error::prelude::*;
use common::mm_number::MmNumber;
use db_common::sqlite::rusqlite::{params, Connection, Error};
use http::Uri;
use prost::Message;
use protobuf::Message as ProtobufMessage;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json, H264 as H264Json};
use rustls::ClientConfig;
use serde_json::{self as json, Value as Json};
use std::path::Path;
use tonic::transport::{Channel, ClientTlsConfig};
use tonic::Status;
use web3::types::Res;
use zcash_client_backend::data_api::{BlockSource, WalletRead};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::wallet::AccountId;
use zcash_client_sqlite::error::SqliteClientError;
use zcash_client_sqlite::wallet::init::init_accounts_table;
use zcash_client_sqlite::WalletDb;
use zcash_primitives::consensus::NetworkUpgrade;
use zcash_primitives::consensus::{BlockHeight, Parameters};
use zcash_primitives::constants::mainnet as z_mainnet_constants;
use zcash_primitives::transaction::components::Amount;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use zcash_proofs::prover::LocalTxProver;

mod z_coin_grpc {
    tonic::include_proto!("cash.z.wallet.sdk.rpc");
}
use z_coin_grpc::compact_tx_streamer_client::CompactTxStreamerClient;
use z_coin_grpc::{BlockId, BlockRange, ChainSpec, RawTransaction};

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

struct CompactBlockRow {
    height: BlockHeight,
    data: Vec<u8>,
}

/// A wrapper for the SQLite connection to the block cache database.
pub struct BlockDb(Connection);

impl BlockDb {
    /// Opens a connection to the wallet database stored at the specified path.
    pub fn for_path<P: AsRef<Path>>(path: P) -> Result<Self, db_common::sqlite::rusqlite::Error> {
        Connection::open(path).map(BlockDb)
    }

    fn with_blocks<F>(
        &self,
        from_height: BlockHeight,
        limit: Option<u32>,
        mut with_row: F,
    ) -> Result<(), SqliteClientError>
    where
        F: FnMut(CompactBlock) -> Result<(), SqliteClientError>,
    {
        // Fetch the CompactBlocks we need to scan
        let mut stmt_blocks = self
            .0
            .prepare("SELECT height, data FROM compactblocks WHERE height > ? ORDER BY height ASC LIMIT ?")?;

        let rows = stmt_blocks.query_map(
            params![u32::from(from_height), limit.unwrap_or(u32::max_value()),],
            |row| {
                Ok(CompactBlockRow {
                    height: BlockHeight::from_u32(row.get(0)?),
                    data: row.get(1)?,
                })
            },
        )?;

        for row_result in rows {
            let cbr = row_result?;
            let block = CompactBlock::parse_from_bytes(&cbr.data)
                .map_err(zcash_client_backend::data_api::error::Error::from)?;

            if block.height() != cbr.height {
                return Err(SqliteClientError::CorruptedData(format!(
                    "Block height {} did not match row's height field value {}",
                    block.height(),
                    cbr.height
                )));
            }

            with_row(block)?;
        }

        Ok(())
    }

    async fn get_latest_block(&self) -> Result<i64, MmError<db_common::sqlite::rusqlite::Error>> {
        self.0
            .query_row(
                "SELECT height FROM compactblocks ORDER BY height DESC LIMIT 1",
                db_common::sqlite::rusqlite::NO_PARAMS,
                |row| row.get(0),
            )
            .map_err(MmError::new)
    }

    async fn insert_block(
        &self,
        height: u32,
        cb_bytes: Vec<u8>,
    ) -> Result<usize, MmError<db_common::sqlite::rusqlite::Error>> {
        self.0
            .prepare("INSERT INTO compactblocks (height, data) VALUES (?, ?)")
            .map_err(MmError::new)?
            .execute(params![height, cb_bytes])
            .map_err(MmError::new)
    }
}

impl BlockSource for BlockDb {
    type Error = SqliteClientError;

    fn with_blocks<F>(&self, from_height: BlockHeight, limit: Option<u32>, with_row: F) -> Result<(), Self::Error>
    where
        F: FnMut(CompactBlock) -> Result<(), Self::Error>,
    {
        self.with_blocks(from_height, limit, with_row)
    }
}

pub struct ZcoinLightClient {
    grpc_client: CompactTxStreamerClient<Channel>,
    blocks_db: BlockDb,
    wallet_db: WalletDb<ZombieNetwork>,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ZcoinLightClientInitError {
    TlsConfigFailure(tonic::transport::Error),
    ConnectionFailure(tonic::transport::Error),
    BlocksDbInitFailure(db_common::sqlite::rusqlite::Error),
    WalletDbInitFailure(db_common::sqlite::rusqlite::Error),
}

#[derive(Debug)]
#[non_exhaustive]
pub enum UpdateBlocksCacheErr {
    GrpcError(tonic::Status),
    DbError(db_common::sqlite::rusqlite::Error),
}

impl From<tonic::Status> for UpdateBlocksCacheErr {
    fn from(err: tonic::Status) -> Self { UpdateBlocksCacheErr::GrpcError(err) }
}

impl From<db_common::sqlite::rusqlite::Error> for UpdateBlocksCacheErr {
    fn from(err: Error) -> Self { UpdateBlocksCacheErr::DbError(err) }
}

impl ZcoinLightClient {
    pub async fn init(
        lightwalletd_url: Uri,
        cache_db_path: impl AsRef<Path> + Send + 'static,
        wallet_db_path: impl AsRef<Path> + Send + 'static,
    ) -> Result<Self, MmError<ZcoinLightClientInitError>> {
        let mut config = ClientConfig::new();
        config
            .root_store
            .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
        config.set_protocols(&["h2".to_string().into()]);
        let tls = ClientTlsConfig::new().rustls_client_config(config);

        let channel = Channel::builder(lightwalletd_url)
            .tls_config(tls)
            .map_to_mm(ZcoinLightClientInitError::TlsConfigFailure)?
            .connect()
            .await
            .map_to_mm(ZcoinLightClientInitError::ConnectionFailure)?;

        let blocks_db = async_blocking(|| {
            BlockDb::for_path(cache_db_path).map_to_mm(ZcoinLightClientInitError::BlocksDbInitFailure)
        })
        .await?;

        let wallet_db = async_blocking(|| {
            WalletDb::for_path(wallet_db_path, ZombieNetwork).map_to_mm(ZcoinLightClientInitError::WalletDbInitFailure)
        })
        .await?;

        Ok(ZcoinLightClient {
            grpc_client: CompactTxStreamerClient::new(channel),
            blocks_db,
            wallet_db,
        })
    }

    pub async fn get_latest_block(&mut self) -> Result<BlockId, MmError<tonic::Status>> {
        let request = tonic::Request::new(ChainSpec {});
        self.grpc_client
            .get_latest_block(request)
            .await
            .map(|res| res.into_inner())
            .map_err(MmError::new)
    }

    pub async fn update_blocks_cache(&mut self) -> Result<u64, MmError<UpdateBlocksCacheErr>> {
        let current_blockchain_block = self.get_latest_block().await?;
        let current_block_in_db = self.blocks_db.get_latest_block().await?;

        let from_block = current_block_in_db as u64 + 1;
        let current_block: u64 = current_blockchain_block.height;

        if current_block >= from_block {
            let request = tonic::Request::new(BlockRange {
                start: Some(BlockId {
                    height: from_block,
                    hash: Vec::new(),
                }),
                end: Some(BlockId {
                    height: current_block,
                    hash: Vec::new(),
                }),
            });

            let mut response = self.grpc_client.get_block_range(request).await?;

            while let Ok(Some(block)) = response.get_mut().message().await {
                println!("Got block {:?}", block);
                self.blocks_db
                    .insert_block(block.height as u32, block.encode_to_vec())
                    .await?;
            }
        }
        Ok(current_block)
    }
}

#[derive(Copy, Clone)]
struct ZombieNetwork;

impl Parameters for ZombieNetwork {
    fn activation_height(&self, nu: NetworkUpgrade) -> Option<BlockHeight> {
        match nu {
            NetworkUpgrade::Sapling => Some(BlockHeight::from(1)),
            _ => None,
        }
    }

    fn coin_type(&self) -> u32 { z_mainnet_constants::COIN_TYPE }

    fn hrp_sapling_extended_spending_key(&self) -> &str { z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY }

    fn hrp_sapling_extended_full_viewing_key(&self) -> &str {
        z_mainnet_constants::HRP_SAPLING_EXTENDED_FULL_VIEWING_KEY
    }

    fn hrp_sapling_payment_address(&self) -> &str { z_mainnet_constants::HRP_SAPLING_PAYMENT_ADDRESS }

    fn b58_pubkey_address_prefix(&self) -> [u8; 2] { z_mainnet_constants::B58_PUBKEY_ADDRESS_PREFIX }

    fn b58_script_address_prefix(&self) -> [u8; 2] { z_mainnet_constants::B58_SCRIPT_ADDRESS_PREFIX }
}

#[test]
// This is a temporary test used to experiment with librustzcash and lightwalletd
fn try_grpc() {
    use common::block_on;
    use db_common::sqlite::rusqlite::{self, Connection, NO_PARAMS};
    use prost::Message;
    use std::str::FromStr;
    use z_coin_grpc::{BlockId, BlockRange, ChainSpec, RawTransaction};
    use zcash_client_backend::data_api::{chain::{scan_cached_blocks, validate_chain},
                                         error::Error,
                                         BlockSource, WalletRead, WalletWrite};
    use zcash_client_backend::encoding::decode_extended_spending_key;
    use zcash_client_sqlite::{error::SqliteClientError, wallet::init::init_wallet_db, wallet::rewind_to_height,
                              BlockDb, WalletDb};

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

    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();

    let uri = Uri::from_str("http://zombie.sirseven.me:443").unwrap();
    let mut client = block_on(ZcoinLightClient::init(
        uri,
        "test_cache_zombie.db",
        "test_wallet_zombie.db",
    ))
    .unwrap();

    let latest_block = block_on(client.get_latest_block()).unwrap();
    let latest_block_height = latest_block.height;
    let current_block = block_on(client.update_blocks_cache()).unwrap();

    // init_cache_database(&cache_sql).unwrap();

    let network = ZombieNetwork {};
    init_wallet_db(&client.wallet_db).unwrap();

    // init_accounts_table(&db_read, &[(&z_key).into()]).unwrap();
    let mut db_data = client.wallet_db.get_update_ops().unwrap();

    // 1) Download new CompactBlocks into db_cache.

    // 2) Run the chain validator on the received blocks.
    //
    // Given that we assume the server always gives us correct-at-the-time blocks, any
    // errors are in the blocks we have previously cached or scanned.
    if let Err(e) = validate_chain(&network, &client.blocks_db, db_data.get_max_height_hash().unwrap()) {
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
    scan_cached_blocks(&network, &client.blocks_db, &mut db_data, None).unwrap();

    let balance = client
        .wallet_db
        .get_balance_at(AccountId(0), BlockHeight::from_u32(current_block as u32))
        .unwrap();

    println!("balance {:?}", balance);

    let notes = client
        .wallet_db
        .get_spendable_notes(AccountId(0), BlockHeight::from_u32(current_block as u32))
        .unwrap();

    use zcash_primitives::consensus;
    use zcash_primitives::transaction::builder::Builder as ZTxBuilder;

    let mut tx_builder = ZTxBuilder::new(ZombieNetwork {}, BlockHeight::from_u32(current_block as u32));

    let from_key = ExtendedFullViewingKey::from(&z_key);
    let (_, from_addr) = from_key.default_address().unwrap();
    let amount_to_send = notes[0].note_value - Amount::from_i64(1000).unwrap();
    for spendable_note in notes.into_iter().take(1) {
        let note = from_addr
            .create_note(spendable_note.note_value.into(), spendable_note.rseed)
            .unwrap();
        tx_builder
            .add_sapling_spend(
                z_key.clone(),
                spendable_note.diversifier,
                note,
                spendable_note.witness.path().unwrap(),
            )
            .unwrap();
    }

    tx_builder
        .add_sapling_output(None, from_addr, amount_to_send, None)
        .unwrap();

    let prover = LocalTxProver::with_default_location().unwrap();
    let (transaction, _) = tx_builder.build(consensus::BranchId::Sapling, &prover).unwrap();

    println!("{:?}", transaction);

    let mut tx_bytes = Vec::with_capacity(1024);
    transaction.write(&mut tx_bytes).expect("Write should not fail");

    let request = tonic::Request::new(RawTransaction {
        data: tx_bytes,
        height: 0,
    });
    let response = block_on(client.grpc_client.send_transaction(request)).unwrap();

    println!("{:?}", response);
}

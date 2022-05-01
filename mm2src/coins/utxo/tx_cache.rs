use common::fs::{read_json, write_json, FsJsonError};
use common::log::LogOnError;
use common::mm_error::prelude::*;
use derive_more::Display;
use futures::lock::Mutex as AsyncMutex;
use futures::FutureExt;
use parking_lot::Mutex as PaMutex;
use rpc::v1::types::{Transaction as RpcTransaction, H256 as H256Json};
use std::collections::hash_map::RawEntryMut;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub type TxCacheResult<T> = MmResult<T, TxCacheError>;

lazy_static! {
    static ref TX_CACHE_LOCK: TxCacheLock = TxCacheLock::default();
}

#[derive(Debug, Display)]
pub enum TxCacheError {
    ErrorLoading(String),
    ErrorSaving(String),
    ErrorDeserializing(String),
    ErrorSerializing(String),
}

impl From<FsJsonError> for TxCacheError {
    fn from(e: FsJsonError) -> Self {
        match e {
            FsJsonError::IoReading(loading) => TxCacheError::ErrorLoading(loading.to_string()),
            FsJsonError::IoWriting(writing) => TxCacheError::ErrorSaving(writing.to_string()),
            FsJsonError::Serializing(ser) => TxCacheError::ErrorSerializing(ser.to_string()),
            FsJsonError::Deserializing(de) => TxCacheError::ErrorDeserializing(de.to_string()),
        }
    }
}

#[derive(Default)]
struct TxCacheLock {
    /// The collection of `Ticker -> Mutex` pairs.
    mutexes: PaMutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl TxCacheLock {
    /// Get the mutex corresponding to the specified `ticker`.
    pub fn mutex_by_ticker(&self, ticker: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self.mutexes.lock();

        match locks.raw_entry_mut().from_key(ticker) {
            RawEntryMut::Occupied(mutex) => mutex.get().clone(),
            RawEntryMut::Vacant(vacant_mutex) => {
                let (_key, mutex) = vacant_mutex.insert(ticker.to_owned(), Arc::new(AsyncMutex::new(())));
                mutex.clone()
            },
        }
    }
}

/// The cache instance that assigned to a specified coin.
///
/// Please note [`UtxoVerboseCache::ticker`] may not equal to [`Coin::ticker`].
/// In particular, `QRC20` tokens have the same transactions as `Qtum` coin,
/// so [`Qrc20Coin::platform_ticker`] is used as [`UtxoVerboseCache::ticker`].
#[derive(Debug)]
pub struct UtxoVerboseCache {
    ticker: String,
    tx_cache_path: PathBuf,
}

impl UtxoVerboseCache {
    pub fn new(ticker: String, tx_cache_path: PathBuf) -> UtxoVerboseCache {
        UtxoVerboseCache { ticker, tx_cache_path }
    }

    /// Tries to load transactions from cache concurrently.
    /// Note 1: `tx.confirmations` can be out-of-date.
    /// Note 2: this function locks the `TX_CACHE_LOCK` mutex to avoid reading and writing the same files at the same time.
    pub async fn load_transactions_from_cache_concurrently<I>(
        &self,
        tx_ids: I,
    ) -> HashMap<H256Json, TxCacheResult<Option<RpcTransaction>>>
    where
        I: IntoIterator<Item = H256Json>,
    {
        let mutex = TX_CACHE_LOCK.mutex_by_ticker(&self.ticker);
        let _lock = mutex.lock().await;

        let it = tx_ids
            .into_iter()
            .map(|txid| self.load_transaction_from_cache(txid).map(move |res| (txid, res)));
        futures::future::join_all(it).await.into_iter().collect()
    }

    /// Uploads transactions to cache concurrently.
    /// Note: this function locks the `TX_CACHE_LOCK` mutex and takes `txs` as the Hash map
    /// to avoid reading and writing the same files at the same time.
    pub async fn cache_transactions_concurrently(&self, txs: &HashMap<H256Json, RpcTransaction>) {
        let mutex = TX_CACHE_LOCK.mutex_by_ticker(&self.ticker);
        let _lock = mutex.lock().await;

        let it = txs.iter().map(|(_txid, tx)| self.cache_transaction(tx));
        futures::future::join_all(it)
            .await
            .into_iter()
            .for_each(|tx| tx.error_log());
    }

    /// Tries to load transaction from cache.
    /// Note: `tx.confirmations` can be out-of-date.
    async fn load_transaction_from_cache(&self, txid: H256Json) -> TxCacheResult<Option<RpcTransaction>> {
        let path = self.cached_transaction_path(&txid);
        read_json(&path).await.mm_err(TxCacheError::from)
    }

    /// Uploads transaction to cache.
    async fn cache_transaction(&self, tx: &RpcTransaction) -> TxCacheResult<()> {
        const USE_TMP_FILE: bool = true;

        let path = self.cached_transaction_path(&tx.txid);
        write_json(tx, &path, USE_TMP_FILE).await.mm_err(TxCacheError::from)
    }

    fn cached_transaction_path(&self, txid: &H256Json) -> PathBuf { self.tx_cache_path.join(format!("{:?}", txid)) }
}

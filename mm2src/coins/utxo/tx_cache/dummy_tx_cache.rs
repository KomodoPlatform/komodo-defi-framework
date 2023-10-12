use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use rpc::v1::types::{Transaction as RpcTransaction, H256 as H256Json};

use crate::utxo::tx_cache::{TxCacheResult, UtxoVerboseCacheOps};

/// The dummy TX cache.
#[derive(Debug, Default)]
pub struct DummyVerboseCache;

#[async_trait]
impl UtxoVerboseCacheOps for DummyVerboseCache {
    async fn load_transactions_from_cache_concurrently(
        &self,
        tx_ids: HashSet<H256Json>,
    ) -> HashMap<H256Json, TxCacheResult<Option<RpcTransaction>>> {
        tx_ids.into_iter().map(|txid| (txid, Ok(None))).collect()
    }

    async fn cache_transactions_concurrently(&self, _txs: &HashMap<H256Json, RpcTransaction>) {}
}

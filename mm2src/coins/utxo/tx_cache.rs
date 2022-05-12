use mm2_io::fs::safe_slurp;
use futures::lock::Mutex as AsyncMutex;
use rpc::v1::types::{Transaction as RpcTransaction, H256 as H256Json};
use std::path::{Path, PathBuf};

lazy_static! {
    static ref TX_CACHE_LOCK: AsyncMutex<()> = AsyncMutex::new(());
}

/// Try load transaction from cache.
/// Note: tx.confirmations can be out-of-date.
pub async fn load_transaction_from_cache(
    tx_cache_path: &Path,
    txid: &H256Json,
) -> Result<Option<RpcTransaction>, String> {
    let _lock = TX_CACHE_LOCK.lock().await;

    let path = cached_transaction_path(tx_cache_path, txid);
    let data = try_s!(safe_slurp(&path));
    if data.is_empty() {
        // couldn't find corresponding file
        return Ok(None);
    }

    let data = try_s!(String::from_utf8(data));
    serde_json::from_str(&data).map(Some).map_err(|e| ERRL!("{}", e))
}

/// Upload transaction to cache.
pub async fn cache_transaction(tx_cache_path: &Path, tx: &RpcTransaction) -> Result<(), String> {
    let _lock = TX_CACHE_LOCK.lock().await;
    let path = cached_transaction_path(tx_cache_path, &tx.txid);
    let tmp_path = format!("{}.tmp", path.display());

    let content = try_s!(serde_json::to_string(tx));

    try_s!(std::fs::write(&tmp_path, content));
    try_s!(std::fs::rename(tmp_path, path));
    Ok(())
}

fn cached_transaction_path(tx_cache_path: &Path, txid: &H256Json) -> PathBuf {
    tx_cache_path.join(format!("{:?}", txid))
}

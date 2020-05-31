use common::mm_ctx::MmArc;
use crate::utxo::UtxoMmCoin;
use futures::lock::{Mutex as AsyncMutex};
use rpc::v1::types::{H256 as H256Json, Transaction as RpcTransaction};
use std::io::Read;
use std::path::Path;

lazy_static! {static ref TX_CACHE_LOCK: AsyncMutex<()> = AsyncMutex::new(());}

/// Try load transaction from cache.
/// The function may panic if there is a problem with the fs.
/// Note: tx.confirmations can be out-of-date.
pub async fn load_transaction_from_cache<T>(coin: &T, ctx: &MmArc, txid: H256Json) -> Result<Option<RpcTransaction>, String>
    where T: UtxoMmCoin {
    let _lock = TX_CACHE_LOCK.lock().await;

    let path = coin.cached_transaction_path(ctx, &txid);
    let data = try_s!(safe_slurp(&path));
    if data.is_empty() {
        // couldn't find corresponding file
        return Ok(None);
    }

    let data = try_s!(String::from_utf8(data));
    serde_json::from_str(&data)
        .map(|x| Some(x))
        .map_err(|e| ERRL!("{}", e))
}

/// Upload transaction to cache.
/// The function may panic if there is a problem with the fs.
pub async fn cache_transaction<T>(coin: &T, ctx: &MmArc, tx: &RpcTransaction) -> Result<(), String>
    where T: UtxoMmCoin {
    let _lock = TX_CACHE_LOCK.lock().await;
    let path = coin.cached_transaction_path(ctx, &tx.txid);
    let tmp_path = format!("{}.tmp", path.display());

    let content = try_s!(serde_json::to_string(tx));

    try_s!(std::fs::write(&tmp_path, content));
    try_s!(std::fs::rename(tmp_path, path));
    Ok(())
}

fn safe_slurp(path: &dyn AsRef<Path>) -> Result<Vec<u8>, String> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(ref err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return ERR!("Can't open {:?}: {}", path.as_ref(), err),
    };
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).expect("!read");
    Ok(buf)
}

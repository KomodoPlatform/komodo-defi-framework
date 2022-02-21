use crate::common::Future01CompatExt;
use coins::{lp_coinfind, GetRawTransactionError};
use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RawTransactionRes {
    pub tx_hex: String,
}

pub async fn get_raw_transaction(ctx: MmArc, req: Json) -> Result<RawTransactionRes, MmError<GetRawTransactionError>> {
    let ticker = req["coin"].as_str().ok_or(GetRawTransactionError::NoCoinField)?;
    let coin = lp_coinfind(&ctx, ticker)
        .await
        .map_err(GetRawTransactionError::Internal)?
        .ok_or_else(|| GetRawTransactionError::InvalidCoin(ticker.to_string()))?;
    let bytes_string = req["tx_hash"].as_str().ok_or(GetRawTransactionError::NoTxHashField)?;
    let res = coin.get_raw_tx(bytes_string).compat().await?;
    Ok(RawTransactionRes { tx_hex: res })
}

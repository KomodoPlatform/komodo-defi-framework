use crate::common::Future01CompatExt;
use coins::{lp_coinfind, GetRawTransactionError};
use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RawTransactionResponse {
    pub tx_hex: String,
}

#[derive(Deserialize, Serialize)]
pub struct GetRawTransactionRequest {
    pub coin: String,
    pub tx_hash: String,
}

pub async fn get_raw_transaction(
    ctx: MmArc,
    req: GetRawTransactionRequest,
) -> Result<RawTransactionResponse, MmError<GetRawTransactionError>> {
    let ticker = req.coin;
    let coin = lp_coinfind(&ctx, &ticker)
        .await
        .map_err(GetRawTransactionError::Internal)?
        .ok_or_else(|| GetRawTransactionError::CoinIsNotActive(ticker.to_string()))?;
    let bytes_string = req.tx_hash;
    let res = coin.get_raw_tx(&bytes_string).compat().await?;
    Ok(RawTransactionResponse { tx_hex: res })
}

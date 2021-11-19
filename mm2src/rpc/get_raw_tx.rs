use coins::lp_coinfind;
use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use serde_json::{self as json, Value as Json};

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetRawTxError {
    Internal(String),
}

#[derive(Serialize)]
pub struct GetRawTxResponse {
    tx_hex: String,
}

#[derive(Serialize, Deserialize)]
pub struct TxInfo {
    pub coin: String,
    pub tx_hash: String,
}

impl From<serde_json::Error> for GetRawTxError {
    fn from(e: serde_json::Error) -> Self { GetRawTxError::Internal(format!("Json parse error: {}", e)) }
}

pub type GetRawTxRpcResult<T> = Result<T, MmError<GetRawTxError>>;

impl HttpStatusCode for GetRawTxError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetRawTxError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn get_raw_tx(ctx: MmArc, req: Json) -> GetRawTxRpcResult<GetRawTxResponse> {
    let tx_info: TxInfo = json::from_value(req)?;
    let coin = match lp_coinfind(&ctx, tx_info.coin.as_str()).await {
        Ok(Some(t)) => t,
        Ok(None) => return Err(MmError::new(GetRawTxError::Internal("No such coin".to_string()))),
        Err(e) => {
            return Err(MmError::new(GetRawTxError::Internal(format!(
                "!lp_coinfind error: {}",
                e
            ))))
        },
    };
    let tx_hex = match coin.get_raw_tx(tx_info.tx_hash) {
        Ok(v) => v,
        Err(e) => return Err(MmError::new(GetRawTxError::Internal(e))),
    };
    Ok(GetRawTxResponse { tx_hex })
}

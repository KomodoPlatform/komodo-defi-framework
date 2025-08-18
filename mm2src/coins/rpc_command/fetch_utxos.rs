use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::{MapMmError, MmResult, MmResultExt};
use mm2_number::BigDecimal;

use crate::{
    hd_wallet::{AddrToString, HDWalletOps},
    lp_coinfind_or_err,
    utxo::{utxo_common::big_decimal_from_sat_unsigned, GetUtxoListOps},
    CoinFindError, DerivationMethod, MmCoinEnum,
};

#[derive(Deserialize)]
pub struct FetchUtxosRequest {
    pub coin: String,
}

#[derive(Serialize)]
pub struct FetchUtxosResponse {
    pub address: String,
    pub count: usize,
    pub utxos: Vec<UnspentOutputs>,
}

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum FetchUtxosError {
    NoSuchCoin,
    CoinNotSupported,
    InvalidAddress(String),
    Internal(String),
}

impl HttpStatusCode for FetchUtxosError {
    fn status_code(&self) -> StatusCode {
        match self {
            FetchUtxosError::NoSuchCoin => StatusCode::NOT_FOUND,
            FetchUtxosError::CoinNotSupported => StatusCode::BAD_REQUEST,
            FetchUtxosError::InvalidAddress(_) => StatusCode::BAD_REQUEST,
            FetchUtxosError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for FetchUtxosError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { .. } => FetchUtxosError::NoSuchCoin,
        }
    }
}

#[derive(Serialize)]
pub struct UnspentOutputs {
    outpoint: String,
    value: BigDecimal,
}

pub async fn fetch_utxos_rpc(ctx: MmArc, req: FetchUtxosRequest) -> MmResult<FetchUtxosResponse, FetchUtxosError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await.map_mm_err()?;

    match coin {
        MmCoinEnum::UtxoCoin(coin) => {
            let from_address = match &coin.as_ref().derivation_method {
                DerivationMethod::SingleAddress(my_address) => my_address.clone(),
                DerivationMethod::HDWallet(wallet) => {
                    let hd_address = wallet.get_enabled_address().await.ok_or_else(|| {
                        FetchUtxosError::InvalidAddress("No enabled address found in HD wallet".to_string())
                    })?;
                    hd_address.address
                },
            };

            let (unspents, _) = coin
                .get_unspent_ordered_list(&from_address)
                .await
                .mm_err(|e| FetchUtxosError::Internal(format!("Couldn't fetch unspent UTXOs: {e}")))?;

            Ok(FetchUtxosResponse {
                address: from_address.addr_to_string(),
                count: unspents.len(),
                utxos: unspents
                    .into_iter()
                    .map(|unspent| UnspentOutputs {
                        outpoint: format!("{}:{}", unspent.outpoint.hash, unspent.outpoint.index),
                        value: big_decimal_from_sat_unsigned(unspent.value, coin.as_ref().decimals),
                    })
                    .collect(),
            })
        },
        _ => Err(FetchUtxosError::CoinNotSupported.into()),
    }
}

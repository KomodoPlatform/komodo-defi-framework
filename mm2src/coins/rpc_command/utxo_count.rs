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
pub struct UtxoCountRequest {
    pub coin: String,
}

#[derive(Serialize)]
pub struct UtxoCountResponse {
    pub address: String,
    pub utxos: Vec<UnspentOutputs>,
}

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum UtxoCountError {
    NoSuchCoin,
    CoinNotSupported,
    InvalidAddress(String),
    Internal(String),
}

impl HttpStatusCode for UtxoCountError {
    fn status_code(&self) -> StatusCode {
        match self {
            UtxoCountError::NoSuchCoin => StatusCode::NOT_FOUND,
            UtxoCountError::CoinNotSupported => StatusCode::BAD_REQUEST,
            UtxoCountError::InvalidAddress(_) => StatusCode::BAD_REQUEST,
            UtxoCountError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for UtxoCountError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { .. } => UtxoCountError::NoSuchCoin,
        }
    }
}

#[derive(Serialize)]
pub struct UnspentOutputs {
    outpoint: String,
    value: BigDecimal,
}

pub async fn utxo_count_rpc(ctx: MmArc, req: UtxoCountRequest) -> MmResult<UtxoCountResponse, UtxoCountError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await.map_mm_err()?;

    match coin {
        MmCoinEnum::UtxoCoin(coin) => {
            let from_address = match &coin.as_ref().derivation_method {
                DerivationMethod::SingleAddress(my_address) => my_address.clone(),
                DerivationMethod::HDWallet(wallet) => {
                    let hd_address = wallet.get_enabled_address().await.ok_or_else(|| {
                        UtxoCountError::InvalidAddress("No enabled address found in HD wallet".to_string())
                    })?;
                    hd_address.address
                },
            };

            let (unspents, _) = coin
                .get_unspent_ordered_list(&from_address)
                .await
                .mm_err(|e| UtxoCountError::Internal(format!("Couldn't fetch unspent UTXOs: {e}")))?;

            Ok(UtxoCountResponse {
                address: from_address.addr_to_string(),
                utxos: unspents
                    .into_iter()
                    .map(|unspent| UnspentOutputs {
                        outpoint: format!("{}:{}", unspent.outpoint.hash, unspent.outpoint.index),
                        value: big_decimal_from_sat_unsigned(unspent.value, coin.as_ref().decimals),
                    })
                    .collect(),
            })
        },
        _ => Err(UtxoCountError::CoinNotSupported.into()),
    }
}

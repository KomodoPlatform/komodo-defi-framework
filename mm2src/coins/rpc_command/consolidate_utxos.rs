use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::{MmResult, MmResultExt};

use crate::{
    hd_wallet::HDWalletOps,
    lp_coinfind_or_err,
    utxo::{output_script, utxo_builder::merge_utxos},
    CoinFindError, DerivationMethod, MmCoinEnum,
};

#[derive(Deserialize)]
pub struct ConsolidateUtxoRequest {
    coin: String,
}

#[derive(Serialize)]
pub struct ConsolidateUtxoResponse {
    tx_hash: String,
}

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum ConsolidateUtxoError {
    NoSuchCoin,
    CoinNotSupported,
    InvalidAddress(String),
    MergeError(String),
}

impl HttpStatusCode for ConsolidateUtxoError {
    fn status_code(&self) -> StatusCode {
        match self {
            ConsolidateUtxoError::NoSuchCoin => StatusCode::NOT_FOUND,
            ConsolidateUtxoError::CoinNotSupported => StatusCode::BAD_REQUEST,
            ConsolidateUtxoError::InvalidAddress(_) => StatusCode::BAD_REQUEST,
            ConsolidateUtxoError::MergeError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for ConsolidateUtxoError {
    fn from(err: CoinFindError) -> Self {
        match err {
            CoinFindError::NoSuchCoin { .. } => ConsolidateUtxoError::NoSuchCoin,
        }
    }
}

pub async fn consolidate_utxos_rpc(
    ctx: MmArc,
    request: ConsolidateUtxoRequest,
) -> MmResult<ConsolidateUtxoResponse, ConsolidateUtxoError> {
    let coin = lp_coinfind_or_err(&ctx, &request.coin).await.map_mm_err()?;
    match coin {
        MmCoinEnum::UtxoCoin(coin) => {
            let from_address = match &coin.as_ref().derivation_method {
                DerivationMethod::SingleAddress(my_address) => my_address.clone(),
                DerivationMethod::HDWallet(wallet) => {
                    let hd_address = wallet.get_enabled_address().await.ok_or_else(|| {
                        ConsolidateUtxoError::InvalidAddress("No enabled address found in HD wallet".to_string())
                    })?;
                    hd_address.address
                },
            };
            let to_script_pubkey = output_script(&from_address).map_err(|e| {
                ConsolidateUtxoError::InvalidAddress(format!("Failed to convert `to_address` to a script_pubkey: {e}"))
            })?;

            let transaction = merge_utxos(&coin, &from_address, &to_script_pubkey, None)
                .await
                .map_err(|e| ConsolidateUtxoError::MergeError(format!("Failed to merge UTXOs: {e}")))?;

            Ok(ConsolidateUtxoResponse {
                tx_hash: transaction.hash().reversed().to_string(),
            })
        },
        _ => Err(ConsolidateUtxoError::CoinNotSupported.into()),
    }
}

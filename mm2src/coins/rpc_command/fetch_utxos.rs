use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use keys::Address;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::{MapMmError, MmResult, MmResultExt};
use mm2_number::BigDecimal;

use crate::{
    hd_wallet::{AddrToString, HDWalletOps},
    lp_coinfind_or_err,
    utxo::{utxo_common::big_decimal_from_sat_unsigned, utxo_standard::UtxoStandardCoin, GetUtxoListOps},
    CoinFindError, DerivationMethod, MmCoinEnum,
};

#[derive(Deserialize)]
pub struct FetchUtxosRequest {
    pub coin: String,
}

#[derive(Serialize)]
pub struct AddressUtxos {
    pub address: String,
    pub count: usize,
    pub utxos: Vec<UnspentOutputs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation_path: Option<String>,
}

#[derive(Serialize)]
pub struct FetchUtxosResponse {
    pub total_count: usize,
    pub addresses: Vec<AddressUtxos>,
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
    txid: String,
    vout: u32,
    value: BigDecimal,
}

pub async fn fetch_utxos_rpc(ctx: MmArc, req: FetchUtxosRequest) -> MmResult<FetchUtxosResponse, FetchUtxosError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await.map_mm_err()?;

    match coin {
        MmCoinEnum::UtxoCoin(coin) => match &coin.as_ref().derivation_method {
            DerivationMethod::SingleAddress(my_address) => {
                let utxos = get_utxos(&coin, my_address).await?;
                Ok(FetchUtxosResponse {
                    total_count: utxos.count,
                    addresses: vec![utxos],
                })
            },
            DerivationMethod::HDWallet(wallet) => {
                let accounts = wallet.get_accounts().await;
                let mut total_count = 0;
                let mut addresses = Vec::new();
                for (_, account) in accounts {
                    let addresses_in_account = account.derived_addresses.lock().await.clone();
                    for (_, address) in addresses_in_account {
                        let mut utxos = get_utxos(&coin, &address.address).await?;
                        // Set the derivation path since this is an HD wallet address.
                        utxos.derivation_path = Some(address.derivation_path.to_string());
                        if utxos.count > 0 {
                            total_count += utxos.count;
                            addresses.push(utxos);
                        }
                    }
                }
                Ok(FetchUtxosResponse { total_count, addresses })
            },
        },
        _ => Err(FetchUtxosError::CoinNotSupported.into()),
    }
}

async fn get_utxos(coin: &UtxoStandardCoin, from_address: &Address) -> MmResult<AddressUtxos, FetchUtxosError> {
    let (unspents, _) = coin
        .get_unspent_ordered_list(from_address)
        .await
        .mm_err(|e| FetchUtxosError::Internal(format!("Couldn't fetch unspent UTXOs (address={from_address}): {e}")))?;

    Ok(AddressUtxos {
        address: from_address.addr_to_string(),
        count: unspents.len(),
        utxos: unspents
            .into_iter()
            .map(|unspent| UnspentOutputs {
                txid: unspent.outpoint.hash.reversed().to_string(),
                vout: unspent.outpoint.index,
                value: big_decimal_from_sat_unsigned(unspent.value, coin.as_ref().decimals),
            })
            .collect(),
        derivation_path: None,
    })
}

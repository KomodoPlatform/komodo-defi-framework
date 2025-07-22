use coins::lp_coinfind_any;
use coins::priv_key::{DerivePrivKeyError, DerivePrivKeyReq, DerivePrivKeyV2, DerivedPrivKey};
use coins::MmCoinEnum;
use crypto::Bip44Chain;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct DerivePrivKeyRequest {
    pub coin: String,
    pub account_id: u32,
    pub address_id: u32,
    #[serde(default)]
    pub chain: Option<Bip44Chain>,
}

pub async fn derive_priv_key(ctx: MmArc, req: DerivePrivKeyRequest) -> MmResult<DerivedPrivKey, DerivePrivKeyError> {
    let coin_ticker = req.coin.clone();
    let coin = lp_coinfind_any(&ctx, &req.coin)
        .await
        .map_err(|e| DerivePrivKeyError::Internal {
            reason: e.to_string(),
        })?
        .ok_or_else(|| DerivePrivKeyError::NoSuchCoin {
            ticker: req.coin.clone(),
        })?
        .inner;

    let req = DerivePrivKeyReq {
        account_id: req.account_id,
        chain: req.chain,
        address_id: req.address_id,
    };

    match coin {
        MmCoinEnum::UtxoCoin(c) => c.derive_priv_key(&req).await,
        MmCoinEnum::Bch(c) => c.derive_priv_key(&req).await,
        MmCoinEnum::QtumCoin(c) => c.derive_priv_key(&req).await,
        MmCoinEnum::EthCoin(c) => c.derive_priv_key(&req).await,
        _ => MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation {
            ticker: coin_ticker,
        }),
    }
}

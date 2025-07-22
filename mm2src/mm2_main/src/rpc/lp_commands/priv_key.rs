use coins::lp_coinfind_any;
use coins::priv_key::{derive_priv_key, DerivePrivKeyError, DerivePrivKeyReq, DerivedPrivKey};
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

pub async fn derive_priv_key_rpc(ctx: MmArc, req: DerivePrivKeyRequest) -> MmResult<DerivedPrivKey, DerivePrivKeyError> {
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

    derive_priv_key(coin, &req).await
}

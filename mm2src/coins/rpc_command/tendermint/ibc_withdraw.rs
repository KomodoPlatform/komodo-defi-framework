use common::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;
use mm2_number::BigDecimal;

use crate::{lp_coinfind_or_err, MmCoinEnum, WithdrawError, WithdrawFee, WithdrawFrom, WithdrawResult};

#[derive(Clone, Deserialize)]
pub struct IBCWithdrawRequest {
    pub(crate) ibc_source_channel: String,
    pub(crate) from: Option<WithdrawFrom>,
    pub(crate) coin: String,
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) amount: BigDecimal,
    #[serde(default)]
    pub(crate) max: bool,
    pub(crate) memo: Option<String>,
    pub(crate) fee: Option<WithdrawFee>,
}

pub async fn ibc_withdraw(ctx: MmArc, req: IBCWithdrawRequest) -> WithdrawResult {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::Tendermint(coin) => coin.ibc_withdraw(req).compat().await,
        MmCoinEnum::TendermintToken(token) => token.ibc_withdraw(req).compat().await,
        _ => MmError::err(WithdrawError::ActionNotAllowed(req.coin)),
    }
}

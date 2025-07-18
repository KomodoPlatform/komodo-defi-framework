use coins::{lp_coinfind_or_err, ReplaceTxError, ReplacementAction, TransactionDetails, WithdrawFee};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::Deserialize;

/// Request structure for the `replace_transaction` RPC.
#[derive(Deserialize)]
pub struct ReplaceTransactionRequest {
    /// The ticker of the coin for which the transaction is being replaced.
    pub coin: String,
    /// The hash of the transaction to be replaced.
    pub tx_hash: String,
    /// The new fee to be used for the replacement transaction.
    pub fee: WithdrawFee,
    /// The action to perform: either speed up or cancel the transaction.
    pub action: ReplacementAction,
    /// If `true`, the replacement transaction will be broadcast to the network.
    #[serde(default)]
    pub broadcast: bool,
}

/// Handles the `replace_transaction` RPC request.
/// This allows for speeding up or canceling a pending transaction by re-issuing it with the same nonce.
pub async fn replace_transaction(
    ctx: MmArc,
    req: ReplaceTransactionRequest,
) -> Result<TransactionDetails, MmError<ReplaceTxError>> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin)
        .await
        .map_err(|e| ReplaceTxError::InternalError(e.to_string()))?;

    // Call the trait method directly with the parameters from the request.
    coin.replace_transaction(req.tx_hash, req.fee, req.action, req.broadcast)
        .await
}

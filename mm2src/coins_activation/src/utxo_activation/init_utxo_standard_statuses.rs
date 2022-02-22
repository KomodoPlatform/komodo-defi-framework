use crate::standalone_coin::InitStandaloneCoinInitialStatus;
use crypto::hw_rpc_task::{HwRpcTaskAwaitingStatus, HwRpcTaskUserAction};
use serde_derive::Serialize;

pub type UtxoStandardAwaitingStatus = HwRpcTaskAwaitingStatus;
pub type UtxoStandardUserAction = HwRpcTaskUserAction;

#[derive(Clone, Serialize)]
pub enum UtxoStandardInProgressStatus {
    ActivatingCoin,
    RequestingWalletBalance,
    Finishing,
    /// This status doesn't require the user to send `UserAction`,
    /// but it tells the user that he should confirm/decline an address on his device.
    WaitingForTrezorToConnect,
    WaitingForUserToConfirmPubkey,
}

impl InitStandaloneCoinInitialStatus for UtxoStandardInProgressStatus {
    fn initial_status() -> Self { UtxoStandardInProgressStatus::ActivatingCoin }
}

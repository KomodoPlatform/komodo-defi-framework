use crate::standalone_coin::{InitStandaloneCoinActivationOps, InitStandaloneCoinTaskHandle};
use crate::utxo_activation::init_utxo_standard_activation_error::InitUtxoStandardError;
use crate::utxo_activation::init_utxo_standard_statuses::{UtxoStandardAwaitingStatus, UtxoStandardInProgressStatus};
use crate::utxo_activation::utxo_standard_activation_result::UtxoStandardActivationResult;
use coins::coin_balance::EnableCoinBalanceOps;
use coins::MarketCoinOps;
use common::mm_error::prelude::*;
use crypto::hw_rpc_task::HwConnectStatuses;
use futures::compat::Future01CompatExt;

pub async fn get_activation_result<Coin>(
    coin: &Coin,
    task_handle: &InitStandaloneCoinTaskHandle<Coin>,
) -> MmResult<UtxoStandardActivationResult, InitUtxoStandardError>
where
    Coin: InitStandaloneCoinActivationOps<
            ActivationError = InitUtxoStandardError,
            InProgressStatus = UtxoStandardInProgressStatus,
        > + EnableCoinBalanceOps
        + MarketCoinOps,
{
    let current_block =
        coin.current_block()
            .compat()
            .await
            .map_to_mm(|error| InitUtxoStandardError::CoinCreationError {
                ticker: coin.ticker().to_owned(),
                error,
            })?;

    task_handle.update_in_progress_status(UtxoStandardInProgressStatus::RequestingWalletBalance)?;
    let wallet_balance = coin
        .enable_coin_balance()
        .await
        .mm_err(|error| InitUtxoStandardError::CoinCreationError {
            ticker: coin.ticker().to_owned(),
            error: error.to_string(),
        })?;
    task_handle.update_in_progress_status(UtxoStandardInProgressStatus::ActivatingCoin)?;

    let result = UtxoStandardActivationResult {
        current_block,
        wallet_balance,
    };
    Ok(result)
}

pub fn xpub_extractor_rpc_statuses() -> HwConnectStatuses<UtxoStandardInProgressStatus, UtxoStandardAwaitingStatus> {
    HwConnectStatuses {
        on_connect: UtxoStandardInProgressStatus::WaitingForTrezorToConnect,
        on_connected: UtxoStandardInProgressStatus::ActivatingCoin,
        on_connection_failed: UtxoStandardInProgressStatus::Finishing,
        on_button_request: UtxoStandardInProgressStatus::WaitingForUserToConfirmPubkey,
        on_pin_request: UtxoStandardAwaitingStatus::WaitForTrezorPin,
        on_ready: UtxoStandardInProgressStatus::ActivatingCoin,
    }
}

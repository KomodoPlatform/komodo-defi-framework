use crate::standalone_coin::{InitStandaloneCoinActivationOps, InitStandaloneCoinTaskHandle};
use crate::utxo_activation::init_utxo_standard_activation_error::InitUtxoStandardError;
use crate::utxo_activation::init_utxo_standard_statuses::{UtxoStandardAwaitingStatus, UtxoStandardInProgressStatus,
                                                          UtxoStandardUserAction};
use crate::utxo_activation::utxo_standard_activation_result::UtxoStandardActivationResult;
use coins::coin_balance::EnableCoinBalanceOps;
use coins::hd_pubkey::RpcTaskXPubExtractor;
use coins::utxo::UtxoActivationParams;
use coins::{MarketCoinOps, PrivKeyActivationPolicy, PrivKeyBuildPolicy};
use crypto::hw_rpc_task::HwConnectStatuses;
use crypto::CryptoCtx;
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;

pub async fn get_activation_result<Coin>(
    ctx: &MmArc,
    coin: &Coin,
    task_handle: &InitStandaloneCoinTaskHandle<Coin>,
    activation_params: &UtxoActivationParams,
) -> MmResult<UtxoStandardActivationResult, InitUtxoStandardError>
where
    Coin: InitStandaloneCoinActivationOps<
            ActivationError = InitUtxoStandardError,
            InProgressStatus = UtxoStandardInProgressStatus,
            AwaitingStatus = UtxoStandardAwaitingStatus,
            UserAction = UtxoStandardUserAction,
        > + EnableCoinBalanceOps
        + MarketCoinOps,
{
    let ticker = coin.ticker().to_owned();
    let current_block = coin
        .current_block()
        .compat()
        .await
        .map_to_mm(InitUtxoStandardError::Transport)?;

    // Construct an Xpub extractor without checking if the MarketMaker supports HD wallet ops.
    // [`EnableCoinBalanceOps::enable_coin_balance`] won't just use `xpub_extractor`
    // if the coin has been initialized with an Iguana priv key.
    let xpub_extractor = RpcTaskXPubExtractor::new_unchecked(ctx, task_handle, xpub_extractor_rpc_statuses());
    task_handle.update_in_progress_status(UtxoStandardInProgressStatus::RequestingWalletBalance)?;
    let wallet_balance = coin
        .enable_coin_balance(&xpub_extractor, activation_params.scan_policy)
        .await
        .mm_err(|enable_err| InitUtxoStandardError::from_enable_coin_balance_err(enable_err, ticker.clone()))?;
    task_handle.update_in_progress_status(UtxoStandardInProgressStatus::ActivatingCoin)?;

    let result = UtxoStandardActivationResult {
        ticker,
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
        on_button_request: UtxoStandardInProgressStatus::FollowHwDeviceInstructions,
        on_pin_request: UtxoStandardAwaitingStatus::EnterTrezorPin,
        on_passphrase_request: UtxoStandardAwaitingStatus::EnterTrezorPassphrase,
        on_ready: UtxoStandardInProgressStatus::ActivatingCoin,
    }
}

pub fn priv_key_build_policy(crypto_ctx: &CryptoCtx, activation_policy: PrivKeyActivationPolicy) -> PrivKeyBuildPolicy {
    match activation_policy {
        PrivKeyActivationPolicy::IguanaPrivKey => PrivKeyBuildPolicy::iguana_priv_key(crypto_ctx),
        PrivKeyActivationPolicy::Trezor => PrivKeyBuildPolicy::Trezor,
    }
}

use async_trait::async_trait;
use bip32::DerivationPath;
use crypto::hw_rpc_task::HwConnectStatuses;
use crypto::trezor::trezor_rpc_task::{TrezorRequestStatuses, TrezorRpcTaskProcessor, TryIntoUserAction};
use crypto::trezor::{ProcessTrezorResponse, TrezorError, TrezorProcessingError};
use crypto::{CryptoCtx, CryptoCtxError, HardwareWalletArc, HwError, HwProcessingError};
use enum_from::{EnumFromInner, EnumFromStringify};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use rpc_task::{RpcTask, RpcTaskError, RpcTaskHandle};

const SHOW_ADDRESS_ON_DISPLAY: bool = true;

#[derive(EnumFromInner, EnumFromStringify)]
pub enum HDConfirmAddressError {
    HwContextNotInitialized,
    RpcTaskError(RpcTaskError),
    #[from_inner]
    HardwareWalletError(HwError),
    InvalidAddress {
        expected: String,
        found: String,
    },
    #[from_stringify("CryptoCtxError")]
    Internal(String),
}

impl From<TrezorError> for HDConfirmAddressError {
    fn from(e: TrezorError) -> Self { HDConfirmAddressError::HardwareWalletError(HwError::from(e)) }
}

impl From<TrezorProcessingError<RpcTaskError>> for HDConfirmAddressError {
    fn from(e: TrezorProcessingError<RpcTaskError>) -> Self {
        match e {
            TrezorProcessingError::TrezorError(trezor) => HDConfirmAddressError::from(HwError::from(trezor)),
            TrezorProcessingError::ProcessorError(rpc) => HDConfirmAddressError::RpcTaskError(rpc),
        }
    }
}

impl From<HwProcessingError<RpcTaskError>> for HDConfirmAddressError {
    fn from(e: HwProcessingError<RpcTaskError>) -> Self {
        match e {
            HwProcessingError::HwError(hw) => HDConfirmAddressError::from(hw),
            HwProcessingError::ProcessorError(rpc) => HDConfirmAddressError::RpcTaskError(rpc),
        }
    }
}

/// An `InProgress` status constructor.
pub trait ConfirmAddressStatus: Sized {
    /// Returns an `InProgress` RPC status that will be used to ask the user
    /// to confirm an `address` on his HW device.
    fn confirm_addr_status(address: String) -> Self;
}

/// An address confirmation interface.
#[async_trait]
pub trait HDConfirmAddress: Sync {
    /// Asks the user to confirm if the given `expected_address` is the same as on the HW display.
    async fn confirm_utxo_address(
        &self,
        trezor_utxo_coin: String,
        derivation_path: DerivationPath,
        expected_address: String,
    ) -> MmResult<(), HDConfirmAddressError>;
}

pub enum RpcTaskConfirmAddress<'task, Task: RpcTask> {
    Trezor {
        hw_ctx: HardwareWalletArc,
        task_handle: &'task RpcTaskHandle<Task>,
        statuses: HwConnectStatuses<Task::InProgressStatus, Task::AwaitingStatus>,
    },
}

#[async_trait]
impl<'task, Task> HDConfirmAddress for RpcTaskConfirmAddress<'task, Task>
where
    Task: RpcTask,
    Task::InProgressStatus: ConfirmAddressStatus,
    Task::UserAction: TryIntoUserAction + Send,
{
    async fn confirm_utxo_address(
        &self,
        trezor_utxo_coin: String,
        derivation_path: DerivationPath,
        expected_address: String,
    ) -> MmResult<(), HDConfirmAddressError> {
        match self {
            RpcTaskConfirmAddress::Trezor {
                hw_ctx,
                task_handle,
                statuses,
            } => {
                Self::confirm_utxo_address_with_trezor(
                    hw_ctx,
                    task_handle,
                    statuses,
                    trezor_utxo_coin,
                    derivation_path,
                    expected_address,
                )
                .await
            },
        }
    }
}

impl<'task, Task> RpcTaskConfirmAddress<'task, Task>
where
    Task: RpcTask,
    Task::InProgressStatus: ConfirmAddressStatus,
    Task::UserAction: TryIntoUserAction + Send,
{
    pub fn new(
        ctx: &MmArc,
        task_handle: &'task RpcTaskHandle<Task>,
        statuses: HwConnectStatuses<Task::InProgressStatus, Task::AwaitingStatus>,
    ) -> MmResult<RpcTaskConfirmAddress<'task, Task>, HDConfirmAddressError> {
        let crypto_ctx = CryptoCtx::from_ctx(ctx)?;
        let hw_ctx = crypto_ctx
            .hw_ctx()
            .or_mm_err(|| HDConfirmAddressError::HwContextNotInitialized)?;
        Ok(RpcTaskConfirmAddress::Trezor {
            hw_ctx,
            task_handle,
            statuses,
        })
    }

    async fn confirm_utxo_address_with_trezor(
        hw_ctx: &HardwareWalletArc,
        task_handle: &RpcTaskHandle<Task>,
        connect_statuses: &HwConnectStatuses<Task::InProgressStatus, Task::AwaitingStatus>,
        trezor_coin: String,
        derivation_path: DerivationPath,
        expected_address: String,
    ) -> MmResult<(), HDConfirmAddressError> {
        let mut trezor_session = hw_ctx.trezor().await?;

        let confirm_statuses = TrezorRequestStatuses {
            on_button_request: Task::InProgressStatus::confirm_addr_status(expected_address.clone()),
            ..connect_statuses.to_trezor_request_statuses()
        };

        let pubkey_processor = TrezorRpcTaskProcessor::new(task_handle, confirm_statuses);
        let address = trezor_session
            .get_utxo_address(derivation_path, trezor_coin, SHOW_ADDRESS_ON_DISPLAY)
            .await?
            .process(&pubkey_processor)
            .await?;

        if address != expected_address {
            return MmError::err(HDConfirmAddressError::InvalidAddress {
                expected: expected_address,
                found: address,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod for_tests {
    use mocktopus::macros::mockable;

    use super::*;

    #[derive(Default)]
    pub struct MockableConfirmAddress;

    #[async_trait]
    #[mockable]
    impl HDConfirmAddress for MockableConfirmAddress {
        async fn confirm_utxo_address(
            &self,
            _trezor_utxo_coin: String,
            _derivation_path: DerivationPath,
            _expected_address: String,
        ) -> MmResult<(), HDConfirmAddressError> {
            unimplemented!()
        }
    }
}

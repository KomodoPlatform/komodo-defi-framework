use derive_more::Display;
use hw_common::primitives::Bip32Error;
use mm2_err_handle::prelude::*;
use serde::Serialize;
use std::time::Duration;
use trezor::{OperationFailure, TrezorError, TrezorUserInteraction};

pub type HwResult<T> = Result<T, MmError<HwError>>;

#[derive(Clone, Debug, Display)]
pub enum HwError {
    #[display(fmt = "No Trezor device available")]
    NoTrezorDeviceAvailable,
    #[display(fmt = "Found multiple devices ({}). Please unplug unused devices", count)]
    CannotChooseDevice {
        count: usize,
    },
    #[display(fmt = "Couldn't connect to a Hardware Wallet device in {:?}", timeout)]
    ConnectionTimedOut {
        timeout: Duration,
    },
    #[display(fmt = "Found unexpected Hardware Wallet device")]
    FoundUnexpectedDevice,
    DeviceDisconnected,
    #[display(fmt = "'{}' transport not supported", transport)]
    TransportNotSupported {
        transport: String,
    },
    #[display(fmt = "Invalid xpub received from a device: '{}'", _0)]
    InvalidXpub(String),
    UnderlyingError(String),
    ProtocolError(String),
    UnexpectedUserInteractionRequest(TrezorUserInteraction),
    Internal(String),
    InvalidPin,
    UnexpectedMessage,
    ButtonExpected,
    DataError,
    PinExpected,
    InvalidSignature,
    ProcessError,
    NotEnoughFunds,
    NotInitialized,
    WipeCodeMismatch,
    InvalidSession,
    FirmwareError,
    FailureMessageNotFound,
    UserCancelled,
    PongMessageMismatch,
}

impl From<TrezorError> for HwError {
    fn from(e: TrezorError) -> Self {
        let error = e.to_string();
        match e {
            TrezorError::TransportNotSupported { transport } => HwError::TransportNotSupported { transport },
            TrezorError::ErrorRequestingAccessPermission(_) => HwError::NoTrezorDeviceAvailable,
            TrezorError::DeviceDisconnected => HwError::DeviceDisconnected,
            TrezorError::UnderlyingError(_) => HwError::UnderlyingError(error),
            TrezorError::ProtocolError(_) | TrezorError::UnexpectedMessageType(_) => HwError::Internal(error),
            TrezorError::Failure(failure) => match failure {
                OperationFailure::InvalidPin => HwError::InvalidPin,
                OperationFailure::UnexpectedMessage => HwError::UnexpectedMessage,
                OperationFailure::ButtonExpected => HwError::ButtonExpected,
                OperationFailure::DataError => HwError::DataError,
                OperationFailure::PinExpected => HwError::PinExpected,
                OperationFailure::InvalidSignature => HwError::InvalidSignature,
                OperationFailure::ProcessError => HwError::ProcessError,
                OperationFailure::NotEnoughFunds => HwError::NotEnoughFunds,
                OperationFailure::NotInitialized => HwError::NotInitialized,
                OperationFailure::WipeCodeMismatch => HwError::WipeCodeMismatch,
                OperationFailure::InvalidSession => HwError::InvalidSession,
                OperationFailure::FirmwareError => HwError::FirmwareError,
                OperationFailure::FailureMessageNotFound => HwError::FailureMessageNotFound,
                OperationFailure::UserCancelled => HwError::UserCancelled,
            },
            TrezorError::UnexpectedInteractionRequest(req) => HwError::UnexpectedUserInteractionRequest(req),
            TrezorError::Internal(_) => HwError::Internal(error),
            TrezorError::PongMessageMismatch => HwError::PongMessageMismatch,
        }
    }
}

impl From<Bip32Error> for HwError {
    fn from(e: Bip32Error) -> Self { HwError::InvalidXpub(e.to_string()) }
}

/// This error enumeration is involved to be used as a part of another RPC error.
/// This enum consists of error types that cli/GUI must handle correctly,
/// so please extend it if it's required **only**.
///
/// Please also note that this enum is fieldless.
#[derive(Clone, Debug, Display, Serialize, PartialEq)]
pub enum HwRpcError {
    #[display(fmt = "No Trezor device available")]
    NoTrezorDeviceAvailable = 0,
    #[display(fmt = "Found multiple devices. Please unplug unused devices")]
    FoundMultipleDevices,
    #[display(fmt = "Found unexpected device. Please re-initialize Hardware wallet")]
    FoundUnexpectedDevice,
}

/// The trait is implemented for those error enumerations that have `HwRpcError` variant.
pub trait WithHwRpcError {
    fn hw_rpc_error(hw_rpc_error: HwRpcError) -> Self;
}

/// Unfortunately, it's not possible to implementing `From<HwError>` for every type
/// that implements `WithHwRpcError`, `WithTimeout` and `WithInternal`.
/// So this function should be called from the `From<HwError>` implementation.
pub fn from_hw_error<T>(hw_error: HwError) -> T
where
    T: WithHwRpcError + WithTimeout + WithInternal,
{
    match hw_error {
        HwError::NoTrezorDeviceAvailable | HwError::DeviceDisconnected => {
            T::hw_rpc_error(HwRpcError::NoTrezorDeviceAvailable)
        },
        HwError::CannotChooseDevice { .. } => T::hw_rpc_error(HwRpcError::FoundMultipleDevices),
        HwError::ConnectionTimedOut { timeout } => T::timeout(timeout),
        HwError::FoundUnexpectedDevice => T::hw_rpc_error(HwRpcError::FoundUnexpectedDevice),
        other => T::internal(other.to_string()),
    }
}

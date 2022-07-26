use crate::HwPubkey;
use derive_more::Display;
use hw_common::primitives::Bip32Error;
use mm2_err_handle::prelude::*;
use serde::Serialize;
use std::time::Duration;
use trezor::{TrezorError, TrezorUserInteraction};

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
    #[display(
        fmt = "Expected a Hardware Wallet device with '{}' pubkey, found '{}'",
        expected_pubkey,
        actual_pubkey
    )]
    FoundUnexpectedDevice {
        actual_pubkey: HwPubkey,
        expected_pubkey: HwPubkey,
    },
    DeviceDisconnected,
    #[display(fmt = "'{}' transport not supported", transport)]
    TransportNotSupported {
        transport: String,
    },
    #[display(fmt = "Invalid xpub received from a device: '{}'", _0)]
    InvalidXpub(Bip32Error),
    Failure(String),
    UnderlyingError(String),
    ProtocolError(String),
    UnexpectedUserInteractionRequest(TrezorUserInteraction),
    Internal(String),
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
            // TODO handle the failure correctly later
            TrezorError::Failure(_) => HwError::Failure(error),
            TrezorError::UnexpectedInteractionRequest(req) => HwError::UnexpectedUserInteractionRequest(req),
            TrezorError::Internal(_) => HwError::Internal(error),
        }
    }
}

impl From<Bip32Error> for HwError {
    fn from(e: Bip32Error) -> Self { HwError::InvalidXpub(e) }
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

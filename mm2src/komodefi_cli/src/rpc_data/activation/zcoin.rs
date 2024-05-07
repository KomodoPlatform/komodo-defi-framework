use derive_more::Display;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use serde::{Deserialize, Serialize};
use skip_serializing_none::skip_serializing_none;
use std::time::Duration;

use common::one_thousand_u32;

use super::{CoinBalance, ElectrumRpcRequest, RpcTaskStatus, TaskId};
use crate::rpc_data::Bip44Chain;

#[skip_serializing_none]
#[derive(Deserialize, Serialize)]
pub(crate) struct ZcoinActivationParams {
    pub(crate) mode: ZcoinRpcMode,
    pub(crate) required_confirmations: Option<u64>,
    pub(crate) requires_notarization: Option<bool>,
    pub(crate) zcash_params_path: Option<String>,
    #[serde(default = "one_thousand_u32")]
    pub(crate) scan_blocks_per_iteration: u32,
    #[serde(default)]
    pub(crate) scan_interval_ms: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "rpc", content = "rpc_data")]
pub(crate) enum ZcoinRpcMode {
    #[cfg(not(target_arch = "wasm32"))]
    Native,
    Light {
        electrum_servers: Vec<ElectrumRpcRequest>,
        light_wallet_d_servers: Vec<String>,
    },
}

pub(crate) type ZCoinStatus =
    RpcTaskStatus<ZcoinActivationResult, InitStandaloneCoinError, ZcoinInProgressStatus, ZcoinAwaitingStatus>;

#[derive(Display, Deserialize)]
#[serde(tag = "error_type", content = "error_data")]
pub(crate) enum InitStandaloneCoinError {
    #[display(fmt = "No such task '{}'", _0)]
    NoSuchTask(TaskId),
    #[display(fmt = "Initialization task has timed out {:?}", duration)]
    TaskTimedOut { duration: Duration },
    #[display(fmt = "Coin {} is activated already", ticker)]
    CoinIsAlreadyActivated { ticker: String },
    #[display(fmt = "Coin {} config is not found", _0)]
    CoinConfigIsNotFound(String),
    #[display(fmt = "Coin {} protocol parsing failed: {}", ticker, error)]
    CoinProtocolParseError { ticker: String, error: String },
    #[display(fmt = "Unexpected platform protocol {:?} for {}", protocol, ticker)]
    UnexpectedCoinProtocol { ticker: String, protocol: CoinProtocol },
    #[display(fmt = "Error on platform coin {} creation: {}", ticker, error)]
    CoinCreationError { ticker: String, error: String },
    #[display(fmt = "{}", _0)]
    HwError(HwRpcError),
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

#[derive(Clone, Debug, Display, Serialize, Deserialize)]
pub(crate) enum HwRpcError {
    #[display(fmt = "No Trezor device available")]
    NoTrezorDeviceAvailable = 0,
    #[display(fmt = "Found multiple devices. Please unplug unused devices")]
    FoundMultipleDevices,
    #[display(fmt = "Found unexpected device. Please re-initialize Hardware wallet")]
    FoundUnexpectedDevice,
    #[display(fmt = "Pin is invalid")]
    InvalidPin,
    #[display(fmt = "Unexpected message")]
    UnexpectedMessage,
    #[display(fmt = "Button expected")]
    ButtonExpected,
    #[display(fmt = "Got data error")]
    DataError,
    #[display(fmt = "Pin expected")]
    PinExpected,
    #[display(fmt = "Invalid signature")]
    InvalidSignature,
    #[display(fmt = "Got process error")]
    ProcessError,
    #[display(fmt = "Not enough funds")]
    NotEnoughFunds,
    #[display(fmt = "Not initialized")]
    NotInitialized,
    #[display(fmt = "Wipe code mismatch")]
    WipeCodeMismatch,
    #[display(fmt = "Invalid session")]
    InvalidSession,
    #[display(fmt = "Got firmware error")]
    FirmwareError,
    #[display(fmt = "Failure message not found")]
    FailureMessageNotFound,
    #[display(fmt = "User cancelled action")]
    UserCancelled,
    #[display(fmt = "PONG message mismatch after ping")]
    PongMessageMismatch,
}

#[derive(Display, Deserialize)]
#[non_exhaustive]
pub(crate) enum ZcoinInProgressStatus {
    #[display(fmt = "Activating coin")]
    ActivatingCoin,
    #[display(
        fmt = "Updating block cache, current_scanned_block: {}, latest_block: {}",
        current_scanned_block,
        latest_block
    )]
    UpdatingBlocksCache {
        current_scanned_block: u64,
        latest_block: u64,
    },
    #[display(
        fmt = "Building wallet db, current_scanned_block: {}, latest_block: {}",
        current_scanned_block,
        latest_block
    )]
    BuildingWalletDb {
        current_scanned_block: u64,
        latest_block: u64,
    },
    #[display(fmt = "Temporary error: {}", _0)]
    TemporaryError(String),
    #[display(fmt = "Requesting wallet balance")]
    RequestingWalletBalance,
    #[display(fmt = "Finishing")]
    Finishing,
    #[display(fmt = "Waiting for trezor to connect")]
    WaitingForTrezorToConnect,
    #[display(fmt = "Waiting for user to confirm")]
    WaitingForUserToConfirmPubkey,
}

pub(crate) type ZcoinAwaitingStatus = HwRpcTaskAwaitingStatus;

#[derive(Display, Deserialize)]
pub(crate) enum HwRpcTaskAwaitingStatus {
    EnterTrezorPin,
    EnterTrezorPassphrase,
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "protocol_data")]
pub(crate) enum CoinProtocol {
    ZHTLC(ZcoinProtocolInfo),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ZcoinProtocolInfo {
    pub(crate) consensus_params: ZcoinConsensusParams,
    pub(crate) check_point_block: Option<CheckPointBlockInfo>,
    pub(crate) z_derivation_path: Option<StandardHDPathToCoin>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CheckPointBlockInfo {
    pub(crate) height: u32,
    pub(crate) hash: H256Json,
    pub(crate) time: u32,
    pub(crate) sapling_tree: BytesJson,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ZcoinConsensusParams {
    pub(crate) overwinter_activation_height: u32,
    pub(crate) sapling_activation_height: u32,
    pub(crate) blossom_activation_height: Option<u32>,
    pub(crate) heartwood_activation_height: Option<u32>,
    pub(crate) canopy_activation_height: Option<u32>,
    pub(crate) coin_type: u32,
    pub(crate) hrp_sapling_extended_spending_key: String,
    pub(crate) hrp_sapling_extended_full_viewing_key: String,
    pub(crate) hrp_sapling_payment_address: String,
    pub(crate) b58_pubkey_address_prefix: [u8; 2],
    pub(crate) b58_script_address_prefix: [u8; 2],
}

#[rustfmt::skip]
pub(crate) type StandardHDPathToCoin =
Bip32Child<Bip32PurposeValue, // `purpose`
    Bip32Child<HardenedValue, // `coin_type`
        Bip44Tail>>;

#[derive(Debug, Deserialize)]
pub(crate) struct Bip32Child<Value, Child> {
    pub(crate) value: Value,
    pub(crate) child: Child,
}

#[repr(u32)]
#[derive(Debug, Display, Deserialize)]
pub(crate) enum Bip43Purpose {
    Bip32 = 32,
    Bip44 = 44,
    Bip49 = 49,
    Bip84 = 84,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Bip32PurposeValue {
    pub(crate) purpose: Bip43Purpose,
}

pub(crate) type HardenedValue = AnyValue<true>;

#[derive(Debug, Deserialize)]
pub(crate) struct AnyValue<const HARDENED: bool> {
    pub(crate) number: u32,
}

#[derive(Debug, Display, Deserialize)]
pub(crate) struct Bip44Tail;

#[derive(Deserialize)]
pub(crate) struct ZcoinActivationResult {
    pub(crate) ticker: String,
    pub(crate) current_block: u64,
    pub(crate) wallet_balance: CoinBalanceReport,
}

#[derive(Deserialize)]
#[serde(tag = "wallet_type")]
pub(crate) enum CoinBalanceReport {
    Iguana(IguanaWalletBalance),
    HD(HDWalletBalance),
}

#[derive(Deserialize)]
pub(crate) struct IguanaWalletBalance {
    pub(crate) address: String,
    pub(crate) balance: CoinBalance,
}

#[derive(Deserialize)]
pub(crate) struct HDWalletBalance {
    pub(crate) accounts: Vec<HDAccountBalance>,
}

#[derive(Deserialize)]
pub(crate) struct HDAccountBalance {
    pub(crate) account_index: u32,
    pub(crate) derivation_path: RpcDerivationPath,
    pub(crate) total_balance: CoinBalance,
    pub(crate) addresses: Vec<HDAddressBalance>,
}

#[derive(Deserialize)]
pub(crate) struct HDAddressBalance {
    pub(crate) address: String,
    pub(crate) derivation_path: RpcDerivationPath,
    pub(crate) chain: Bip44Chain,
    pub(crate) balance: CoinBalance,
}

#[derive(Deserialize)]
pub(crate) struct RpcDerivationPath(pub(crate) DerivationPath);

#[derive(Deserialize)]
pub(crate) struct DerivationPath {
    pub(crate) path: Vec<ChildNumber>,
}

#[derive(Deserialize)]
pub(crate) struct ChildNumber(pub(crate) u32);

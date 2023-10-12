use common::jsonrpc_client::JsonRpcError;
#[cfg(not(target_arch = "wasm32"))]
use db_common::sqlite::rusqlite::Error as SqliteError;
use derive_more::Display;
use http::uri::InvalidUri;
use mm2_number::BigDecimal;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
#[cfg(not(target_arch = "wasm32"))]
use zcash_client_sqlite::error::SqliteClientError;
use zcash_primitives::transaction::builder::Error as ZTxBuilderError;

use crate::my_tx_history_v2::MyTxHistoryErrorV2;
use crate::utxo::rpc_clients::UtxoRpcError;
use crate::utxo::utxo_builder::UtxoCoinBuildError;
use crate::z_coin::storage::WalletDbError;
use crate::NumConversError;
use crate::PrivKeyPolicyNotAllowed;
use crate::WithdrawError;

/// Represents possible errors that might occur while interacting with Zcoin rpc.
#[derive(Debug, Display)]
#[non_exhaustive]
pub enum UpdateBlocksCacheErr {
    #[cfg(not(target_arch = "wasm32"))]
    GrpcError(tonic::Status),
    UtxoRpcError(UtxoRpcError),
    InternalError(String),
    JsonRpcError(JsonRpcError),
    GetLiveLightClientError(String),
    ZcashDBError(String),
    DecodeError(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<tonic::Status> for UpdateBlocksCacheErr {
    fn from(err: tonic::Status) -> Self { UpdateBlocksCacheErr::GrpcError(err) }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteError> for UpdateBlocksCacheErr {
    fn from(err: SqliteError) -> Self { UpdateBlocksCacheErr::ZcashDBError(err.to_string()) }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteClientError> for UpdateBlocksCacheErr {
    fn from(err: SqliteClientError) -> Self { UpdateBlocksCacheErr::ZcashDBError(err.to_string()) }
}

impl From<UtxoRpcError> for UpdateBlocksCacheErr {
    fn from(err: UtxoRpcError) -> Self { UpdateBlocksCacheErr::UtxoRpcError(err) }
}

impl From<JsonRpcError> for UpdateBlocksCacheErr {
    fn from(err: JsonRpcError) -> Self { UpdateBlocksCacheErr::JsonRpcError(err) }
}

/// This enum encompasses various error scenarios that may arise
/// when configuring and activating a Zcoin, such as invalid
/// configuration settings, network connectivity issues, or other
/// initialization failures.
#[derive(Debug, Display)]
#[non_exhaustive]
pub enum ZcoinClientInitError {
    ZcashDBError(String),
    EmptyLightwalletdUris,
    #[display(fmt = "Fail to init clients while iterating lightwalletd urls {:?}", _0)]
    UrlIterFailure(Vec<UrlIterError>),
    UpdateBlocksCacheErr(UpdateBlocksCacheErr),
    UtxoCoinBuildError(UtxoCoinBuildError),
}

impl From<UpdateBlocksCacheErr> for ZcoinClientInitError {
    fn from(err: UpdateBlocksCacheErr) -> Self { ZcoinClientInitError::UpdateBlocksCacheErr(err) }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteClientError> for ZcoinClientInitError {
    fn from(err: SqliteClientError) -> Self { ZcoinClientInitError::ZcashDBError(err.to_string()) }
}

#[derive(Debug, Display)]
pub enum UrlIterError {
    InvalidUri(InvalidUri),
    #[cfg(not(target_arch = "wasm32"))]
    TlsConfigFailure(tonic::transport::Error),
    #[cfg(not(target_arch = "wasm32"))]
    ConnectionFailure(tonic::transport::Error),
}

#[derive(Debug, Display)]
pub enum GenTxError {
    DecryptedOutputNotFound,
    GetWitnessErr(GetUnspentWitnessErr),
    FailedToGetMerklePath,
    #[display(
        fmt = "Not enough {} to generate a tx: available {}, required at least {}",
        coin,
        available,
        required
    )]
    InsufficientBalance {
        coin: String,
        available: BigDecimal,
        required: BigDecimal,
    },
    NumConversion(NumConversError),
    Rpc(UtxoRpcError),
    PrevTxNotConfirmed,
    TxBuilderError(ZTxBuilderError),
    #[display(fmt = "Failed to read ZCash tx from bytes {:?} with error {}", hex, err)]
    TxReadError {
        hex: BytesJson,
        err: std::io::Error,
    },
    BlockchainScanStopped,
    LightClientErr(String),
    FailedToCreateNote,
    SpendableNotesError(String),
}

impl From<GetUnspentWitnessErr> for GenTxError {
    fn from(err: GetUnspentWitnessErr) -> GenTxError { GenTxError::GetWitnessErr(err) }
}

impl From<NumConversError> for GenTxError {
    fn from(err: NumConversError) -> GenTxError { GenTxError::NumConversion(err) }
}

impl From<UtxoRpcError> for GenTxError {
    fn from(err: UtxoRpcError) -> GenTxError { GenTxError::Rpc(err) }
}

impl From<ZTxBuilderError> for GenTxError {
    fn from(err: ZTxBuilderError) -> GenTxError { GenTxError::TxBuilderError(err) }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteClientError> for GenTxError {
    fn from(err: SqliteClientError) -> Self { GenTxError::LightClientErr(err.to_string()) }
}

impl From<GenTxError> for WithdrawError {
    fn from(gen_tx: GenTxError) -> WithdrawError {
        match gen_tx {
            GenTxError::InsufficientBalance {
                coin,
                available,
                required,
            } => WithdrawError::NotSufficientBalance {
                coin,
                available,
                required,
            },
            GenTxError::Rpc(e) => WithdrawError::Transport(e.to_string()),
            GenTxError::DecryptedOutputNotFound
            | GenTxError::FailedToGetMerklePath
            | GenTxError::PrevTxNotConfirmed
            | GenTxError::GetWitnessErr(_)
            | GenTxError::NumConversion(_)
            | GenTxError::TxBuilderError(_)
            | GenTxError::TxReadError { .. }
            | GenTxError::BlockchainScanStopped
            | GenTxError::LightClientErr(_)
            | GenTxError::SpendableNotesError(_)
            | GenTxError::FailedToCreateNote => WithdrawError::InternalError(gen_tx.to_string()),
        }
    }
}

#[derive(Debug, Display)]
#[display(fmt = "Blockchain scan process stopped")]
pub struct BlockchainScanStopped {}

impl From<BlockchainScanStopped> for GenTxError {
    #[inline]
    fn from(_: BlockchainScanStopped) -> Self { GenTxError::BlockchainScanStopped }
}

#[derive(Debug, Display)]
#[allow(clippy::large_enum_variant)]
pub enum SendOutputsErr {
    GenTxError(GenTxError),
    NumConversion(NumConversError),
    Rpc(UtxoRpcError),
    TxNotMined(String),
    PrivKeyPolicyNotAllowed(PrivKeyPolicyNotAllowed),
}

impl From<PrivKeyPolicyNotAllowed> for SendOutputsErr {
    fn from(err: PrivKeyPolicyNotAllowed) -> Self { SendOutputsErr::PrivKeyPolicyNotAllowed(err) }
}

impl From<GenTxError> for SendOutputsErr {
    fn from(err: GenTxError) -> SendOutputsErr { SendOutputsErr::GenTxError(err) }
}

impl From<NumConversError> for SendOutputsErr {
    fn from(err: NumConversError) -> SendOutputsErr { SendOutputsErr::NumConversion(err) }
}

impl From<UtxoRpcError> for SendOutputsErr {
    fn from(err: UtxoRpcError) -> SendOutputsErr { SendOutputsErr::Rpc(err) }
}

#[derive(Debug, Display)]
pub enum GetUnspentWitnessErr {
    EmptyDbResult,
    TreeOrWitnessAppendFailed,
    OutputCmuNotFoundInCache,
    ZcashDBError(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteError> for GetUnspentWitnessErr {
    fn from(err: SqliteError) -> GetUnspentWitnessErr { GetUnspentWitnessErr::ZcashDBError(err.to_string()) }
}

#[derive(Debug, Display)]
pub enum ZCoinBuildError {
    UtxoBuilderError(UtxoCoinBuildError),
    GetAddressError,
    ZcashDBError(String),
    Rpc(UtxoRpcError),
    #[display(fmt = "Sapling cache DB does not exist at {}. Please download it.", path)]
    SaplingCacheDbDoesNotExist {
        path: String,
    },
    Io(std::io::Error),
    RpcClientInitErr(ZcoinClientInitError),
    ZCashParamsNotFound,
    ZDerivationPathNotSet,
    SaplingParamsInvalidChecksum,
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteError> for ZCoinBuildError {
    fn from(err: SqliteError) -> ZCoinBuildError { ZCoinBuildError::ZcashDBError(err.to_string()) }
}

impl From<UtxoRpcError> for ZCoinBuildError {
    fn from(err: UtxoRpcError) -> ZCoinBuildError { ZCoinBuildError::Rpc(err) }
}

impl From<std::io::Error> for ZCoinBuildError {
    fn from(err: std::io::Error) -> ZCoinBuildError { ZCoinBuildError::Io(err) }
}

impl From<UtxoCoinBuildError> for ZCoinBuildError {
    fn from(err: UtxoCoinBuildError) -> Self { ZCoinBuildError::UtxoBuilderError(err) }
}

impl From<ZcoinClientInitError> for ZCoinBuildError {
    fn from(err: ZcoinClientInitError) -> Self { ZCoinBuildError::RpcClientInitErr(err) }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) enum SqlTxHistoryError {
    Sql(SqliteError),
    FromIdDoesNotExist(i64),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteError> for SqlTxHistoryError {
    fn from(err: SqliteError) -> Self { SqlTxHistoryError::Sql(err) }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqlTxHistoryError> for MyTxHistoryErrorV2 {
    fn from(err: SqlTxHistoryError) -> Self {
        match err {
            SqlTxHistoryError::Sql(sql) => MyTxHistoryErrorV2::StorageError(sql.to_string()),
            SqlTxHistoryError::FromIdDoesNotExist(id) => {
                MyTxHistoryErrorV2::StorageError(format!("from_id {} does not exist", id))
            },
        }
    }
}

pub(super) struct NoInfoAboutTx(pub(super) H256Json);

impl From<NoInfoAboutTx> for MyTxHistoryErrorV2 {
    fn from(err: NoInfoAboutTx) -> Self {
        MyTxHistoryErrorV2::RpcError(format!("No info about transaction {:02x}", err.0))
    }
}

#[derive(Debug, Display)]
pub enum SpendableNotesError {
    DBClientError(String),
}

#[derive(Debug, Display)]
pub enum ZCoinBalanceError {}

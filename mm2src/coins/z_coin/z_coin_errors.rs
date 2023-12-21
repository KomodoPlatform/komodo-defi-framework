use crate::my_tx_history_v2::MyTxHistoryErrorV2;
use crate::utxo::rpc_clients::UtxoRpcError;
use crate::utxo::utxo_builder::UtxoCoinBuildError;
use crate::NumConversError;
use crate::PrivKeyPolicyNotAllowed;
use crate::WithdrawError;

use common::jsonrpc_client::JsonRpcError;
#[cfg(not(target_arch = "wasm32"))]
use db_common::sqlite::rusqlite::Error as SqliteError;
use derive_more::Display;
use http::uri::InvalidUri;
#[cfg(target_arch = "wasm32")]
use mm2_db::indexed_db::cursor_prelude::*;
#[cfg(target_arch = "wasm32")]
use mm2_db::indexed_db::{DbTransactionError, InitDbError};
use mm2_err_handle::mm_error::MmError;
use mm2_number::BigDecimal;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use zcash_client_backend::data_api::error::ChainInvalid;
#[cfg(not(target_arch = "wasm32"))]
use zcash_client_sqlite::error::SqliteClientError;
#[cfg(target_arch = "wasm32")] use zcash_extras::NoteId;
use zcash_primitives::consensus::BlockHeight;
use zcash_primitives::transaction::builder::Error as ZTxBuilderError;

/// Represents possible errors that might occur while interacting with Zcoin rpc.
#[derive(Debug, Display)]
#[non_exhaustive]
pub enum UpdateBlocksCacheErr {
    GrpcError(tonic::Status),
    UtxoRpcError(UtxoRpcError),
    InternalError(String),
    JsonRpcError(JsonRpcError),
    GetLiveLightClientError(String),
    ZcashDBError(String),
    DecodeError(String),
}

impl From<ZcoinStorageError> for UpdateBlocksCacheErr {
    fn from(err: ZcoinStorageError) -> Self { UpdateBlocksCacheErr::ZcashDBError(err.to_string()) }
}

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
    ZcoinStorageError(String),
    EmptyLightwalletdUris,
    #[display(fmt = "Fail to init clients while iterating lightwalletd urls {:?}", _0)]
    UrlIterFailure(Vec<UrlIterError>),
    UpdateBlocksCacheErr(UpdateBlocksCacheErr),
    UtxoCoinBuildError(UtxoCoinBuildError),
}

impl From<ZcoinStorageError> for ZcoinClientInitError {
    fn from(err: ZcoinStorageError) -> Self { ZcoinClientInitError::ZcoinStorageError(err.to_string()) }
}

impl From<UpdateBlocksCacheErr> for ZcoinClientInitError {
    fn from(err: UpdateBlocksCacheErr) -> Self { ZcoinClientInitError::UpdateBlocksCacheErr(err) }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteClientError> for ZcoinClientInitError {
    fn from(err: SqliteClientError) -> Self { ZcoinClientInitError::ZcoinStorageError(err.to_string()) }
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
    #[cfg(target_arch = "wasm32")]
    Internal(String),
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
            #[cfg(target_arch = "wasm32")]
            GenTxError::Internal(_) => WithdrawError::InternalError(gen_tx.to_string()),
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
    ZCashParamsError(String),
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
pub enum ZCoinBalanceError {
    BalanceError(String),
}

impl From<ZcoinStorageError> for ZCoinBalanceError {
    fn from(value: ZcoinStorageError) -> Self { ZCoinBalanceError::BalanceError(value.to_string()) }
}
/// The `ValidateBlocksError` enum encapsulates different types of errors that may occur
/// during the validation and scanning process of zcoin blocks.
#[derive(Debug, Display)]
pub enum ValidateBlocksError {
    #[display(fmt = "Chain Invalid occurred at height: {height:?} — with error {err:?}")]
    ChainInvalid {
        height: BlockHeight,
        err: ChainInvalid,
    },
    GetFromStorageError(String),
    IoError(String),
    DbError(String),
    DecodingError(String),
    TableNotEmpty(String),
    InvalidNote(String),
    InvalidNoteId,
    IncorrectHrpExtFvk(String),
    CorruptedData(String),
    InvalidMemo(String),
    BackendError(String),
    ZcoinStorageError(String),
}

impl From<ValidateBlocksError> for ZcoinStorageError {
    fn from(value: ValidateBlocksError) -> Self { Self::ValidateBlocksError(value) }
}
impl From<MmError<ZcoinStorageError>> for ValidateBlocksError {
    fn from(value: MmError<ZcoinStorageError>) -> Self { Self::ZcoinStorageError(value.to_string()) }
}

impl ValidateBlocksError {
    pub fn prev_hash_mismatch(height: BlockHeight) -> ValidateBlocksError {
        ValidateBlocksError::ChainInvalid {
            height,
            err: ChainInvalid::PrevHashMismatch,
        }
    }

    pub fn block_height_discontinuity(height: BlockHeight, found: BlockHeight) -> ValidateBlocksError {
        ValidateBlocksError::ChainInvalid {
            height,
            err: ChainInvalid::BlockHeightDiscontinuity(found),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteClientError> for ValidateBlocksError {
    fn from(value: SqliteClientError) -> Self {
        match value {
            SqliteClientError::CorruptedData(err) => Self::CorruptedData(err),
            SqliteClientError::IncorrectHrpExtFvk => Self::IncorrectHrpExtFvk(value.to_string()),
            SqliteClientError::InvalidNote => Self::InvalidNote(value.to_string()),
            SqliteClientError::InvalidNoteId => Self::InvalidNoteId,
            SqliteClientError::TableNotEmpty => Self::TableNotEmpty(value.to_string()),
            SqliteClientError::Bech32(_) | SqliteClientError::Base58(_) => Self::DecodingError(value.to_string()),
            SqliteClientError::DbError(err) => Self::DbError(err.to_string()),
            SqliteClientError::Io(err) => Self::IoError(err.to_string()),
            SqliteClientError::InvalidMemo(err) => Self::InvalidMemo(err.to_string()),
            SqliteClientError::BackendError(err) => Self::BackendError(err.to_string()),
        }
    }
}

/// The `ZcoinStorageError` enum encapsulates different types of errors that may occur
/// when interacting with storage operations specific to the Zcoin blockchain.
#[derive(Debug, Display)]
pub enum ZcoinStorageError {
    #[cfg(not(target_arch = "wasm32"))]
    SqliteError(SqliteClientError),
    ValidateBlocksError(ValidateBlocksError),
    #[display(fmt = "Chain Invalid occurred at height: {height:?} — with error {err:?}")]
    ChainInvalid {
        height: BlockHeight,
        err: ChainInvalid,
    },
    IoError(String),
    DbError(String),
    DecodingError(String),
    TableNotEmpty(String),
    InvalidNote(String),
    InvalidNoteId,
    #[display(fmt = "Incorrect Hrp extended full viewing key")]
    IncorrectHrpExtFvk,
    CorruptedData(String),
    InvalidMemo(String),
    BackendError(String),
    #[display(fmt = "Add to storage err: {}", _0)]
    AddToStorageErr(String),
    #[display(fmt = "Remove from storage err: {}", _0)]
    RemoveFromStorageErr(String),
    #[display(fmt = "Get from storage err: {}", _0)]
    GetFromStorageError(String),
    #[display(fmt = "Error getting {ticker} block height from storage: {err}")]
    BlockHeightNotFound {
        ticker: String,
        err: String,
    },
    #[display(fmt = "Storage Initialization err: {err} - ticker: {ticker}")]
    InitDbError {
        ticker: String,
        err: String,
    },
    ChainError(String),
    InternalError(String),
    NotSupported(String),
    #[cfg(target_arch = "wasm32")]
    ZcashParamsError(String),
}

impl From<UpdateBlocksCacheErr> for ZcoinStorageError {
    fn from(err: UpdateBlocksCacheErr) -> Self { ZcoinStorageError::DbError(err.to_string()) }
}

#[cfg(target_arch = "wasm32")]
impl From<zcash_client_backend::data_api::error::Error<NoteId>> for ZcoinStorageError {
    fn from(value: zcash_client_backend::data_api::error::Error<NoteId>) -> Self {
        Self::BackendError(value.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
impl From<InitDbError> for ZcoinStorageError {
    fn from(e: InitDbError) -> Self {
        match &e {
            InitDbError::NotSupported(_) => ZcoinStorageError::NotSupported(e.to_string()),
            InitDbError::EmptyTableList
            | InitDbError::DbIsOpenAlready { .. }
            | InitDbError::InvalidVersion(_)
            | InitDbError::OpeningError(_)
            | InitDbError::TypeMismatch { .. }
            | InitDbError::UnexpectedState(_)
            | InitDbError::UpgradingError { .. } => ZcoinStorageError::InternalError(e.to_string()),
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl From<DbTransactionError> for ZcoinStorageError {
    fn from(e: DbTransactionError) -> Self {
        match e {
            DbTransactionError::ErrorSerializingItem(_) | DbTransactionError::ErrorDeserializingItem(_) => {
                ZcoinStorageError::DecodingError(e.to_string())
            },
            DbTransactionError::ErrorUploadingItem(_) => ZcoinStorageError::AddToStorageErr(e.to_string()),
            DbTransactionError::ErrorGettingItems(_) | DbTransactionError::ErrorCountingItems(_) => {
                ZcoinStorageError::GetFromStorageError(e.to_string())
            },
            DbTransactionError::ErrorDeletingItems(_) => ZcoinStorageError::RemoveFromStorageErr(e.to_string()),
            DbTransactionError::NoSuchTable { .. }
            | DbTransactionError::ErrorCreatingTransaction(_)
            | DbTransactionError::ErrorOpeningTable { .. }
            | DbTransactionError::ErrorSerializingIndex { .. }
            | DbTransactionError::UnexpectedState(_)
            | DbTransactionError::TransactionAborted
            | DbTransactionError::MultipleItemsByUniqueIndex { .. }
            | DbTransactionError::NoSuchIndex { .. }
            | DbTransactionError::InvalidIndex { .. } => ZcoinStorageError::InternalError(e.to_string()),
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl From<CursorError> for ZcoinStorageError {
    fn from(value: CursorError) -> Self {
        match value {
            CursorError::ErrorSerializingIndexFieldValue { .. }
            | CursorError::ErrorDeserializingIndexValue { .. }
            | CursorError::ErrorDeserializingItem(_) => Self::DecodingError(value.to_string()),
            CursorError::ErrorOpeningCursor { .. }
            | CursorError::AdvanceError { .. }
            | CursorError::InvalidKeyRange { .. }
            | CursorError::IncorrectNumberOfKeysPerIndex { .. }
            | CursorError::UnexpectedState(_)
            | CursorError::IncorrectUsage { .. }
            | CursorError::TypeMismatch { .. } => Self::DbError(value.to_string()),
        }
    }
}

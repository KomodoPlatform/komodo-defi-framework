use crate::utxo::rpc_clients::UtxoRpcError;
use crate::utxo::utxo_builder::UtxoCoinBuildError;
use crate::WithdrawError;
use crate::{NumConversError, PrivKeyNotAllowed};
use db_common::sqlite::rusqlite::Error as SqliteError;
use derive_more::Display;
use http::uri::InvalidUri;
use mm2_number::BigDecimal;
use rpc::v1::types::Bytes as BytesJson;
use zcash_client_sqlite::error::SqliteClientError;
use zcash_client_sqlite::error::SqliteClientError as ZcashClientError;
use zcash_primitives::transaction::builder::Error as ZTxBuilderError;

#[derive(Debug, Display)]
#[non_exhaustive]
pub enum UpdateBlocksCacheErr {
    GrpcError(tonic::Status),
    DbError(SqliteError),
}

impl From<tonic::Status> for UpdateBlocksCacheErr {
    fn from(err: tonic::Status) -> Self { UpdateBlocksCacheErr::GrpcError(err) }
}

impl From<SqliteError> for UpdateBlocksCacheErr {
    fn from(err: SqliteError) -> Self { UpdateBlocksCacheErr::DbError(err) }
}

#[derive(Debug, Display)]
#[non_exhaustive]
pub enum ZcoinLightClientInitError {
    TlsConfigFailure(tonic::transport::Error),
    ConnectionFailure(tonic::transport::Error),
    BlocksDbInitFailure(SqliteError),
    WalletDbInitFailure(SqliteError),
    ZcashSqliteError(ZcashClientError),
}

impl From<ZcashClientError> for ZcoinLightClientInitError {
    fn from(err: ZcashClientError) -> Self { ZcoinLightClientInitError::ZcashSqliteError(err) }
}

#[derive(Debug, Display)]
#[display(fmt = "Blockchain scan process stopped")]
pub struct BlockchainScanStopped {}

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
    LightClientErr(SqliteClientError),
    FailedToCreateNote,
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

impl From<SqliteClientError> for GenTxError {
    fn from(err: SqliteClientError) -> Self { GenTxError::LightClientErr(err) }
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
            | GenTxError::FailedToCreateNote => WithdrawError::InternalError(gen_tx.to_string()),
        }
    }
}

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
    PrivKeyNotAllowed(PrivKeyNotAllowed),
}

impl From<PrivKeyNotAllowed> for SendOutputsErr {
    fn from(err: PrivKeyNotAllowed) -> Self { SendOutputsErr::PrivKeyNotAllowed(err) }
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
    Sql(SqliteError),
}

impl From<SqliteError> for GetUnspentWitnessErr {
    fn from(err: SqliteError) -> GetUnspentWitnessErr { GetUnspentWitnessErr::Sql(err) }
}

#[derive(Debug, Display)]
pub enum ZCoinBuildError {
    UtxoBuilderError(UtxoCoinBuildError),
    GetAddressError,
    SqliteError(SqliteError),
    Rpc(UtxoRpcError),
    #[display(fmt = "Sapling cache DB does not exist at {}. Please download it.", path)]
    SaplingCacheDbDoesNotExist {
        path: String,
    },
    Io(std::io::Error),
    EmptyLightwalletdUris,
    NativeModeIsNotSupportedYet,
    InvalidLightwalletdUri(InvalidUri),
    LightClientInitErr(ZcoinLightClientInitError),
    ZCashParamsNotFound,
}

impl From<SqliteError> for ZCoinBuildError {
    fn from(err: SqliteError) -> ZCoinBuildError { ZCoinBuildError::SqliteError(err) }
}

impl From<UtxoRpcError> for ZCoinBuildError {
    fn from(err: UtxoRpcError) -> ZCoinBuildError { ZCoinBuildError::Rpc(err) }
}

impl From<UtxoCoinBuildError> for ZCoinBuildError {
    fn from(err: UtxoCoinBuildError) -> Self { ZCoinBuildError::UtxoBuilderError(err) }
}

impl From<std::io::Error> for ZCoinBuildError {
    fn from(err: std::io::Error) -> ZCoinBuildError { ZCoinBuildError::Io(err) }
}

impl From<InvalidUri> for ZCoinBuildError {
    fn from(err: InvalidUri) -> Self { ZCoinBuildError::InvalidLightwalletdUri(err) }
}

impl From<ZcoinLightClientInitError> for ZCoinBuildError {
    fn from(err: ZcoinLightClientInitError) -> Self { ZCoinBuildError::LightClientInitErr(err) }
}

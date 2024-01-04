use crate::eth::GetEthAddressError;
#[cfg(target_arch = "wasm32")]
use crate::nft::storage::wasm::WasmNftCacheError;
use crate::nft::storage::NftStorageError;
use crate::{CoinFindError, GetMyAddressError, WithdrawError};
use common::{HttpStatusCode, ParseRfc3339Err};
#[cfg(not(target_arch = "wasm32"))]
use db_common::sqlite::rusqlite::Error as SqlError;
use derive_more::Display;
use enum_from::EnumFromStringify;
use http::StatusCode;
use mm2_net::transport::{GetInfoFromUriError, SlurpError};
use serde::{Deserialize, Serialize};
use web3::Error;

/// Enumerates potential errors that can arise when fetching NFT information.
#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetNftInfoError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[from_stringify("serde_json::Error")]
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
    GetEthAddressError(GetEthAddressError),
    #[display(
        fmt = "Token: token_address {}, token_id {} was not found in wallet",
        token_address,
        token_id
    )]
    TokenNotFoundInWallet {
        token_address: String,
        token_id: String,
    },
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
    ParseRfc3339Err(ParseRfc3339Err),
    #[display(fmt = "The contract type is required and should not be null.")]
    ContractTypeIsNull,
    ProtectFromSpamError(ProtectFromSpamError),
    TransferConfirmationsError(TransferConfirmationsError),
}

impl From<GetNftInfoError> for WithdrawError {
    fn from(e: GetNftInfoError) -> Self { WithdrawError::GetNftInfoError(e) }
}

impl From<SlurpError> for GetNftInfoError {
    fn from(e: SlurpError) -> Self {
        let error_str = e.to_string();
        match e {
            SlurpError::ErrorDeserializing { .. } => GetNftInfoError::InvalidResponse(error_str),
            SlurpError::Transport { .. } | SlurpError::Timeout { .. } => GetNftInfoError::Transport(error_str),
            SlurpError::InvalidRequest(_) => GetNftInfoError::InvalidRequest(error_str),
            SlurpError::Internal(_) => GetNftInfoError::Internal(error_str),
        }
    }
}

impl From<web3::Error> for GetNftInfoError {
    fn from(e: Error) -> Self {
        let error_str = e.to_string();
        match e {
            web3::Error::InvalidResponse(_) | web3::Error::Decoder(_) | web3::Error::Rpc(_) => {
                GetNftInfoError::InvalidResponse(error_str)
            },
            web3::Error::Transport(_) | web3::Error::Io(_) => GetNftInfoError::Transport(error_str),
            _ => GetNftInfoError::Internal(error_str),
        }
    }
}

impl From<GetEthAddressError> for GetNftInfoError {
    fn from(e: GetEthAddressError) -> Self { GetNftInfoError::GetEthAddressError(e) }
}

impl<T: NftStorageError> From<T> for GetNftInfoError {
    fn from(err: T) -> Self { GetNftInfoError::DbError(format!("{:?}", err)) }
}

impl From<GetInfoFromUriError> for GetNftInfoError {
    fn from(e: GetInfoFromUriError) -> Self {
        match e {
            GetInfoFromUriError::InvalidRequest(e) => GetNftInfoError::InvalidRequest(e),
            GetInfoFromUriError::Transport(e) => GetNftInfoError::Transport(e),
            GetInfoFromUriError::InvalidResponse(e) => GetNftInfoError::InvalidResponse(e),
            GetInfoFromUriError::Internal(e) => GetNftInfoError::Internal(e),
        }
    }
}

impl From<ParseRfc3339Err> for GetNftInfoError {
    fn from(e: ParseRfc3339Err) -> Self { GetNftInfoError::ParseRfc3339Err(e) }
}

impl From<ProtectFromSpamError> for GetNftInfoError {
    fn from(e: ProtectFromSpamError) -> Self { GetNftInfoError::ProtectFromSpamError(e) }
}

impl From<LockDBError> for GetNftInfoError {
    fn from(e: LockDBError) -> Self { GetNftInfoError::DbError(e.to_string()) }
}

impl From<TransferConfirmationsError> for GetNftInfoError {
    fn from(e: TransferConfirmationsError) -> Self { GetNftInfoError::TransferConfirmationsError(e) }
}

impl HttpStatusCode for GetNftInfoError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetNftInfoError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            GetNftInfoError::InvalidResponse(_) | GetNftInfoError::ParseRfc3339Err(_) => StatusCode::FAILED_DEPENDENCY,
            GetNftInfoError::ContractTypeIsNull => StatusCode::NOT_FOUND,
            GetNftInfoError::Transport(_)
            | GetNftInfoError::Internal(_)
            | GetNftInfoError::GetEthAddressError(_)
            | GetNftInfoError::TokenNotFoundInWallet { .. }
            | GetNftInfoError::DbError(_)
            | GetNftInfoError::ProtectFromSpamError(_)
            | GetNftInfoError::TransferConfirmationsError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Enumerates possible errors that can occur while updating NFT details in the database.
///
/// The errors capture various issues that can arise during:
/// - Metadata refresh
/// - NFT transfer history updating
/// - NFT list updating
///
/// The issues addressed include database errors, invalid hex strings,
/// inconsistencies in block numbers, and problems related to fetching or interpreting
/// fetched metadata.
#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum UpdateNftError {
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
    GetNftInfoError(GetNftInfoError),
    GetMyAddressError(GetMyAddressError),
    #[display(
        fmt = "Token: token_address {}, token_id {} was not found in wallet",
        token_address,
        token_id
    )]
    TokenNotFoundInWallet {
        token_address: String,
        token_id: String,
    },
    #[display(
        fmt = "Insufficient amount NFT token in the cache: amount in list table before transfer {}, transferred {}",
        amount_list,
        amount_history
    )]
    InsufficientAmountInCache {
        amount_list: String,
        amount_history: String,
    },
    #[display(
        fmt = "Last scanned nft block {} should be >= last block number {} in nft table",
        last_scanned_block,
        last_nft_block
    )]
    InvalidBlockOrder {
        last_scanned_block: String,
        last_nft_block: String,
    },
    #[display(
        fmt = "Last scanned block not found, while the last NFT block exists: {}",
        last_nft_block
    )]
    LastScannedBlockNotFound {
        last_nft_block: String,
    },
    #[display(fmt = "Attempt to receive duplicate ERC721 token in transaction hash: {}", tx_hash)]
    AttemptToReceiveAlreadyOwnedErc721 {
        tx_hash: String,
    },
    #[display(fmt = "Invalid hex string: {}", _0)]
    InvalidHexString(String),
    UpdateSpamPhishingError(UpdateSpamPhishingError),
    GetInfoFromUriError(GetInfoFromUriError),
    #[from_stringify("serde_json::Error")]
    SerdeError(String),
    ProtectFromSpamError(ProtectFromSpamError),
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin {
        coin: String,
    },
    #[display(fmt = "{} coin doesn't support NFT", coin)]
    CoinDoesntSupportNft {
        coin: String,
    },
}

impl From<GetNftInfoError> for UpdateNftError {
    fn from(e: GetNftInfoError) -> Self { UpdateNftError::GetNftInfoError(e) }
}

impl From<GetMyAddressError> for UpdateNftError {
    fn from(e: GetMyAddressError) -> Self { UpdateNftError::GetMyAddressError(e) }
}

impl<T: NftStorageError> From<T> for UpdateNftError {
    fn from(err: T) -> Self { UpdateNftError::DbError(format!("{:?}", err)) }
}

impl From<UpdateSpamPhishingError> for UpdateNftError {
    fn from(e: UpdateSpamPhishingError) -> Self { UpdateNftError::UpdateSpamPhishingError(e) }
}

impl From<GetInfoFromUriError> for UpdateNftError {
    fn from(e: GetInfoFromUriError) -> Self { UpdateNftError::GetInfoFromUriError(e) }
}

impl From<ProtectFromSpamError> for UpdateNftError {
    fn from(e: ProtectFromSpamError) -> Self { UpdateNftError::ProtectFromSpamError(e) }
}

impl From<LockDBError> for UpdateNftError {
    fn from(e: LockDBError) -> Self { UpdateNftError::DbError(e.to_string()) }
}

impl From<CoinFindError> for UpdateNftError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => UpdateNftError::NoSuchCoin { coin },
        }
    }
}

impl HttpStatusCode for UpdateNftError {
    fn status_code(&self) -> StatusCode {
        match self {
            UpdateNftError::DbError(_)
            | UpdateNftError::Internal(_)
            | UpdateNftError::GetNftInfoError(_)
            | UpdateNftError::GetMyAddressError(_)
            | UpdateNftError::TokenNotFoundInWallet { .. }
            | UpdateNftError::InsufficientAmountInCache { .. }
            | UpdateNftError::InvalidBlockOrder { .. }
            | UpdateNftError::LastScannedBlockNotFound { .. }
            | UpdateNftError::AttemptToReceiveAlreadyOwnedErc721 { .. }
            | UpdateNftError::InvalidHexString(_)
            | UpdateNftError::UpdateSpamPhishingError(_)
            | UpdateNftError::GetInfoFromUriError(_)
            | UpdateNftError::SerdeError(_)
            | UpdateNftError::ProtectFromSpamError(_)
            | UpdateNftError::NoSuchCoin { .. }
            | UpdateNftError::CoinDoesntSupportNft { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Enumerates the errors that can occur during spam protection operations.
///
/// This includes issues such as regex failures during text validation and
/// serialization/deserialization problems.
#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize)]
pub enum ProtectFromSpamError {
    #[from_stringify("regex::Error")]
    RegexError(String),
    #[from_stringify("serde_json::Error")]
    SerdeError(String),
}

/// An enumeration representing the potential errors encountered
/// during the process of updating spam or phishing-related information.
///
/// This error set captures various failures, from request malformation
/// to database interaction errors, providing a comprehensive view of
/// possible issues during the spam/phishing update operations.
#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize)]
pub enum UpdateSpamPhishingError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[from_stringify("serde_json::Error")]
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
    GetMyAddressError(GetMyAddressError),
}

impl From<GetMyAddressError> for UpdateSpamPhishingError {
    fn from(e: GetMyAddressError) -> Self { UpdateSpamPhishingError::GetMyAddressError(e) }
}

impl From<GetInfoFromUriError> for UpdateSpamPhishingError {
    fn from(e: GetInfoFromUriError) -> Self {
        match e {
            GetInfoFromUriError::InvalidRequest(e) => UpdateSpamPhishingError::InvalidRequest(e),
            GetInfoFromUriError::Transport(e) => UpdateSpamPhishingError::Transport(e),
            GetInfoFromUriError::InvalidResponse(e) => UpdateSpamPhishingError::InvalidResponse(e),
            GetInfoFromUriError::Internal(e) => UpdateSpamPhishingError::Internal(e),
        }
    }
}

impl<T: NftStorageError> From<T> for UpdateSpamPhishingError {
    fn from(err: T) -> Self { UpdateSpamPhishingError::DbError(format!("{:?}", err)) }
}

/// Errors encountered when parsing a `Chain` from a string.
#[derive(Debug, Display)]
pub enum ParseChainTypeError {
    /// The provided string does not correspond to any of the supported blockchain types.
    UnsupportedChainType,
}

#[derive(Debug, Display, EnumFromStringify)]
pub(crate) enum MetaFromUrlError {
    #[from_stringify("serde_json::Error")]
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    GetInfoFromUriError(GetInfoFromUriError),
}

impl From<GetInfoFromUriError> for MetaFromUrlError {
    fn from(e: GetInfoFromUriError) -> Self { MetaFromUrlError::GetInfoFromUriError(e) }
}

#[derive(Debug, Display)]
pub enum LockDBError {
    #[cfg(target_arch = "wasm32")]
    WasmNftCacheError(WasmNftCacheError),
    #[cfg(not(target_arch = "wasm32"))]
    SqlError(SqlError),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqlError> for LockDBError {
    fn from(e: SqlError) -> Self { LockDBError::SqlError(e) }
}

#[cfg(target_arch = "wasm32")]
impl From<WasmNftCacheError> for LockDBError {
    fn from(e: WasmNftCacheError) -> Self { LockDBError::WasmNftCacheError(e) }
}

#[derive(Clone, Debug, Deserialize, Display, PartialEq, Serialize)]
pub enum TransferConfirmationsError {
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "{} coin doesn't support NFT", coin)]
    CoinDoesntSupportNft { coin: String },
    #[display(fmt = "Get current block error: {}", _0)]
    GetCurrentBlockErr(String),
}

impl From<CoinFindError> for TransferConfirmationsError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => TransferConfirmationsError::NoSuchCoin { coin },
        }
    }
}

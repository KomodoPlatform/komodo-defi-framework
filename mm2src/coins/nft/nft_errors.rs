use common::{HttpStatusCode, ParseRfc3339Err};
use derive_more::Display;
use enum_from::EnumFromStringify;
use http::StatusCode;
use mm2_net::transport::SlurpError;
use serde::{Deserialize, Serialize};
use web3::Error;

use crate::eth::GetEthAddressError;
use crate::nft::storage::{CreateNftStorageError, NftStorageError};
use crate::{GetMyAddressError, WithdrawError};

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

impl From<CreateNftStorageError> for GetNftInfoError {
    fn from(e: CreateNftStorageError) -> Self {
        match e {
            CreateNftStorageError::Internal(err) => GetNftInfoError::Internal(err),
        }
    }
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
            | GetNftInfoError::ProtectFromSpamError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

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
}

impl From<CreateNftStorageError> for UpdateNftError {
    fn from(e: CreateNftStorageError) -> Self {
        match e {
            CreateNftStorageError::Internal(err) => UpdateNftError::Internal(err),
        }
    }
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
            | UpdateNftError::InvalidHexString(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize)]
pub(crate) enum GetInfoFromUriError {
    /// `http::Error` can appear on an HTTP request [`http::Builder::build`] building.
    #[from_stringify("http::Error")]
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[from_stringify("serde_json::Error")]
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
}

impl From<SlurpError> for GetInfoFromUriError {
    fn from(e: SlurpError) -> Self {
        let error_str = e.to_string();
        match e {
            SlurpError::ErrorDeserializing { .. } => GetInfoFromUriError::InvalidResponse(error_str),
            SlurpError::Transport { .. } | SlurpError::Timeout { .. } => GetInfoFromUriError::Transport(error_str),
            SlurpError::InvalidRequest(_) => GetInfoFromUriError::InvalidRequest(error_str),
            SlurpError::Internal(_) => GetInfoFromUriError::Internal(error_str),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize)]
pub enum ProtectFromSpamError {
    #[from_stringify("regex::Error")]
    RegexError(String),
    #[from_stringify("serde_json::Error")]
    SerdeError(String),
}

use crate::eth_provider::EthProviderError;
use derive_more::Display;
use mm2_err_handle::prelude::*;
use serde_derive::Serialize;

pub type MetamaskResult<T> = MmResult<T, MetamaskError>;

#[derive(Debug, Display)]
pub enum MetamaskError {
    #[display(fmt = "ETH provider not found")]
    EthProviderNotFound,
    #[display(fmt = "Expected one ETH selected account")]
    ExpectedOneEthAccount,
    #[display(fmt = "Unexpected account selected")]
    UnexpectedAccountSelected,
    #[display(fmt = "Error serializing RPC arguments: {}", _0)]
    ErrorSerializingArguments(String),
    #[display(fmt = "Error deserializing RPC result: {}", _0)]
    ErrorDeserializingMethodResult(String),
    #[display(fmt = "Error invoking the method '{}': {}", method, error)]
    ErrorInvokingMethod { method: String, error: String },
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<EthProviderError> for MetamaskError {
    fn from(value: EthProviderError) -> Self {
        match value {
            EthProviderError::ErrorSerializingArguments(ser) => MetamaskError::ErrorSerializingArguments(ser),
            EthProviderError::ErrorDeserializingMethodResult(de) => MetamaskError::ErrorDeserializingMethodResult(de),
            EthProviderError::ErrorInvokingMethod { method, error } => {
                MetamaskError::ErrorInvokingMethod { method, error }
            },
            EthProviderError::Internal(internal) => MetamaskError::Internal(internal),
        }
    }
}

/// This error enumeration is involved to be used as a part of another RPC error.
/// This enum consists of error types that cli/GUI must handle correctly,
/// so please extend it if it's required **only**.
///
/// Please also note that this enum is fieldless.
#[derive(Clone, Debug, Display, Serialize, PartialEq)]
pub enum MetamaskRpcError {
    EthProviderNotFound,
    #[display(fmt = "An unexpected ETH account selected. Please select previous account or re-initialize MetaMask")]
    UnexpectedAccountSelected,
}

pub trait WithMetamaskRpcError {
    fn metamask_rpc_error(metamask_rpc_error: MetamaskRpcError) -> Self;
}

/// Unfortunately, it's not possible to implementing `From<MetamaskError>` for every type
/// that implements `WithMetamaskRpcError`, `WithTimeout` and `WithInternal`.
/// So this function should be called from the `From<MetamaskError>` implementation.
pub fn from_metamask_error<T>(metamask_error: MetamaskError) -> T
where
    T: WithMetamaskRpcError + WithInternal,
{
    match metamask_error {
        MetamaskError::EthProviderNotFound => T::metamask_rpc_error(MetamaskRpcError::EthProviderNotFound),
        MetamaskError::UnexpectedAccountSelected => T::metamask_rpc_error(MetamaskRpcError::UnexpectedAccountSelected),
        other => T::internal(other.to_string()),
    }
}

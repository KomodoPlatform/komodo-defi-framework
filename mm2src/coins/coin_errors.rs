use crate::eth::{CoinAssocTypesError, NftAssocTypesError, Web3RpcError};
use crate::nft::trading_proto_v2::errors::{Erc721FunctionError, HtlcParamsError, PaymentStatusErr, PrepareTxDataError};
use crate::{utxo::rpc_clients::UtxoRpcError, NumConversError, UnexpectedDerivationMethod};
use enum_derives::EnumFromStringify;
use futures01::Future;
use mm2_err_handle::prelude::MmError;
use spv_validation::helpers_validation::SPVError;
use std::num::TryFromIntError;

/// Helper type used as result for swap payment validation function(s)
pub type ValidatePaymentFut<T> = Box<dyn Future<Item = T, Error = MmError<ValidatePaymentError>> + Send>;
/// Helper type used as result for swap payment validation function(s)
pub type ValidatePaymentResult<T> = Result<T, MmError<ValidatePaymentError>>;

/// Enum covering possible error cases of swap payment validation
#[derive(Debug, Display, EnumFromStringify)]
pub enum ValidatePaymentError {
    /// Should be used to indicate internal MM2 state problems (e.g., DB errors, etc.).
    #[from_stringify(
        "CoinAssocTypesError",
        "Erc721FunctionError",
        "NftAssocTypesError",
        "NumConversError",
        "UnexpectedDerivationMethod",
        "keys::Error",
        "PrepareTxDataError"
    )]
    InternalError(String),
    /// Problem with deserializing the transaction, or one of the transaction parts is invalid.
    #[from_stringify("rlp::DecoderError", "serialization::Error")]
    TxDeserializationError(String),
    /// One of the input parameters is invalid.
    InvalidParameter(String),
    /// Coin's RPC returned unexpected/invalid response during payment validation.
    InvalidRpcResponse(String),
    /// Payment transaction doesn't exist on-chain.
    TxDoesNotExist(String),
    /// SPV client error.
    SPVError(SPVError),
    /// Payment transaction is in unexpected state. E.g., `Uninitialized` instead of `Sent` for ETH payment.
    UnexpectedPaymentState(String),
    /// Transport (RPC) error.
    #[from_stringify("web3::Error")]
    Transport(String),
    /// Transaction has wrong properties, for example, it has been sent to a wrong address.
    WrongPaymentTx(String),
    /// Indicates error during watcher reward calculation.
    WatcherRewardError(String),
    /// Input payment timelock overflows the type used by specific coin.
    TimelockOverflow(TryFromIntError),
    #[display(fmt = "Nft Protocol is not supported yet!")]
    NftProtocolNotSupported,
}

impl From<SPVError> for ValidatePaymentError {
    fn from(err: SPVError) -> Self { Self::SPVError(err) }
}

impl From<UtxoRpcError> for ValidatePaymentError {
    fn from(err: UtxoRpcError) -> Self {
        match err {
            UtxoRpcError::Transport(e) => Self::Transport(e.to_string()),
            UtxoRpcError::Internal(e) => Self::InternalError(e),
            _ => Self::InvalidRpcResponse(err.to_string()),
        }
    }
}

impl From<Web3RpcError> for ValidatePaymentError {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(tr) => ValidatePaymentError::Transport(tr),
            Web3RpcError::InvalidResponse(resp) => ValidatePaymentError::InvalidRpcResponse(resp),
            Web3RpcError::Internal(internal) | Web3RpcError::Timeout(internal) => {
                ValidatePaymentError::InternalError(internal)
            },
            Web3RpcError::NftProtocolNotSupported => ValidatePaymentError::NftProtocolNotSupported,
        }
    }
}

impl From<PaymentStatusErr> for ValidatePaymentError {
    fn from(err: PaymentStatusErr) -> Self {
        match err {
            PaymentStatusErr::Transport(e) => Self::Transport(e),
            PaymentStatusErr::AbiError(e)
            | PaymentStatusErr::Internal(e)
            | PaymentStatusErr::TxDeserializationError(e) => Self::InternalError(e),
        }
    }
}

impl From<HtlcParamsError> for ValidatePaymentError {
    fn from(err: HtlcParamsError) -> Self {
        match err {
            HtlcParamsError::WrongPaymentTx(e) => ValidatePaymentError::WrongPaymentTx(e),
            HtlcParamsError::TxDeserializationError(e) => ValidatePaymentError::TxDeserializationError(e),
        }
    }
}

#[derive(Debug, Display, EnumFromStringify)]
pub enum MyAddressError {
    #[from_stringify("UnexpectedDerivationMethod")]
    UnexpectedDerivationMethod(String),
    InternalError(String),
}

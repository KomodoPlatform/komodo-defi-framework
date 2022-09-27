use crate::{my_tx_history_v2::MyTxHistoryErrorV2, utxo::rpc_clients::UtxoRpcError, DelegationError, NumConversError,
            TxHistoryError, UnexpectedDerivationMethod, WithdrawError};
use futures01::Future;
use mm2_err_handle::prelude::MmError;
use spv_validation::helpers_validation::SPVError;

pub type ValidatePaymentFut<T> = Box<dyn Future<Item = T, Error = MmError<ValidatePaymentError>> + Send>;

#[derive(Debug, Display)]
pub enum ValidatePaymentError {
    InternalError(String),
    // Problem with deserializing the transaction, or one of the transaction parts is invalid.
    TxDeserializationError(String),
    InvalidInput(String),
    InvalidRpcResponse(String),
    SPVError(SPVError),
    UnexpectedPaymentState(String),
    Transport(String),
    // Transaction has wrong properties, for example, it has been sent to a wrong address
    WrongPaymentTx(String),
}

impl From<rlp::DecoderError> for ValidatePaymentError {
    fn from(err: rlp::DecoderError) -> Self { Self::TxDeserializationError(err.to_string()) }
}

impl From<web3::Error> for ValidatePaymentError {
    fn from(err: web3::Error) -> Self { Self::Transport(err.to_string()) }
}

impl From<NumConversError> for ValidatePaymentError {
    fn from(err: NumConversError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<SPVError> for ValidatePaymentError {
    fn from(err: SPVError) -> Self { Self::SPVError(err) }
}

impl From<serialization::Error> for ValidatePaymentError {
    fn from(err: serialization::Error) -> Self { Self::TxDeserializationError(err.to_string()) }
}

impl From<UnexpectedDerivationMethod> for ValidatePaymentError {
    fn from(err: UnexpectedDerivationMethod) -> Self { Self::InternalError(err.to_string()) }
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

#[derive(Debug, Display)]
pub enum MyAddressError {
    UnexpectedDerivationMethod(String),
    InternalError(String),
}

impl From<UnexpectedDerivationMethod> for MyAddressError {
    fn from(err: UnexpectedDerivationMethod) -> Self { Self::UnexpectedDerivationMethod(err.to_string()) }
}

impl From<MyAddressError> for WithdrawError {
    fn from(err: MyAddressError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<MyAddressError> for UtxoRpcError {
    fn from(err: MyAddressError) -> Self { Self::Internal(err.to_string()) }
}

impl From<MyAddressError> for DelegationError {
    fn from(err: MyAddressError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<MyAddressError> for TxHistoryError {
    fn from(err: MyAddressError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<MyAddressError> for MyTxHistoryErrorV2 {
    fn from(err: MyAddressError) -> Self { Self::Internal(err.to_string()) }
}

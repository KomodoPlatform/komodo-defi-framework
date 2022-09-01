use crate::{utxo::rpc_clients::UtxoRpcError, DelegationError, NumConversError, TxHistoryError,
            UnexpectedDerivationMethod, WithdrawError};
use futures01::Future;
use mm2_err_handle::prelude::MmError;
use spv_validation::helpers_validation::SPVError;

pub type ValidatePaymentFut<T> = Box<dyn Future<Item = T, Error = MmError<ValidatePaymentError>> + Send>;

#[derive(Debug, Display)]
pub enum ValidatePaymentError {
    InternalError(String),
    InvalidPaymentTxData(String),
    InvalidInput(String),
    InvalidRpcResponse(String),
    SPVError(SPVError),
    UnexpectedPaymentState(String),
    Transport(String),
    WrongPaymentTx(String),
    ValidateHtlcError(String),
    #[display(fmt = "Payment tx {} was sent to wrong address, expected {}", found, expected)]
    WrongReceiverAddress {
        found: String,
        expected: String,
    },
    #[display(fmt = "Payment tx {} was sent from wrong address, expected {}", found, expected)]
    WrongSenderAddress {
        found: String,
        expected: String,
    },
}

impl From<rlp::DecoderError> for ValidatePaymentError {
    fn from(err: rlp::DecoderError) -> Self { Self::InvalidPaymentTxData(err.to_string()) }
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
    fn from(err: serialization::Error) -> Self { Self::InvalidPaymentTxData(err.to_string()) }
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

impl ValidatePaymentError {
    pub fn wrong_receiver_addr<Found, Expected>(found: Found, expected: Expected) -> ValidatePaymentError
    where
        Found: std::fmt::Debug,
        Expected: std::fmt::Debug,
    {
        ValidatePaymentError::WrongReceiverAddress {
            found: format!("{:?}", found),
            expected: format!("{:?}", expected),
        }
    }

    pub fn wrong_sender_addr<Found, Expected>(found: Found, expected: Expected) -> ValidatePaymentError
    where
        Found: std::fmt::Debug,
        Expected: std::fmt::Debug,
    {
        ValidatePaymentError::WrongSenderAddress {
            found: format!("{:?}", found),
            expected: format!("{:?}", expected),
        }
    }
}

#[derive(Debug, Display)]
pub enum MyAddressError {
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    InternalError(String),
}

impl From<UnexpectedDerivationMethod> for MyAddressError {
    fn from(err: UnexpectedDerivationMethod) -> Self { Self::UnexpectedDerivationMethod(err) }
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

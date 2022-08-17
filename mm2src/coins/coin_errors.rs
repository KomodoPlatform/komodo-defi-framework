use crate::{utxo::{qtum::ScriptHashTypeNotSupported, rpc_clients::UtxoRpcError},
            DelegationError, NumConversError, UnexpectedDerivationMethod, WithdrawError};
use spv_validation::helpers_validation::SPVError;

#[derive(Debug, Display, PartialEq)]
pub enum ValidatePaymentError {
    AddressParseError(String),
    InternalError(String),
    InvalidPaymentTxData(String),
    InvalidInput(String),
    ScriptHashTypeNotSupported(String),
    SPVError(String),
    TransportError(String),
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    UnexpectedErc20PaymentData(String),
    UnexpectedPaymentState(String),
    UnexpectedPaymentTx(String),
    ValidateHtlcError(String),
    #[display(fmt = "Payment tx token_addr arg {:?} is invalid, expected {:?}", found, expected)]
    WrongTokenAddress {
        found: String,
        expected: String,
    },
    #[display(fmt = "Payment tx value arg {:?} is invalid, expected {:?}", found, expected)]
    WrongValue {
        found: String,
        expected: String,
    },
    #[display(fmt = "Payment tx time_lock arg {:?} is invalid, expected {:?}", found, expected)]
    WrongTimeLock {
        found: String,
        expected: String,
    },
    #[display(fmt = "Invalid 'swap_id' {:?}, expected {:?}", found, expected)]
    WrongSwapId {
        found: String,
        expected: String,
    },
    #[display(fmt = "Payment tx receiver arg {:?} is invalid, expected {:?}", found, expected)]
    WrongReceiver {
        found: String,
        expected: String,
    },
    #[display(fmt = "Payment tx secret_hash arg {:?} is invalid, expected {:?}", found, expected)]
    WrongSecretHash {
        found: String,
        expected: String,
    },
    #[display(fmt = "Payment tx {:?} was sent to wrong address, expected {:?}", found, expected)]
    WrongReceiverAddress {
        found: String,
        expected: String,
    },
    #[display(fmt = "Payment tx {:?} was sent from wrong address, expected {:?}", found, expected)]
    WrongSenderAddress {
        found: String,
        expected: String,
    },
}

impl From<ethabi::Error> for ValidatePaymentError {
    fn from(err: ethabi::Error) -> Self { Self::InvalidPaymentTxData(err.to_string()) }
}

impl From<rlp::DecoderError> for ValidatePaymentError {
    fn from(err: rlp::DecoderError) -> Self { Self::InvalidPaymentTxData(err.to_string()) }
}

impl From<keys::Error> for ValidatePaymentError {
    fn from(err: keys::Error) -> Self { Self::InternalError(err.to_string()) }
}

impl From<web3::Error> for ValidatePaymentError {
    fn from(err: web3::Error) -> Self { Self::TransportError(err.to_string()) }
}

impl From<NumConversError> for ValidatePaymentError {
    fn from(err: NumConversError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<SPVError> for ValidatePaymentError {
    fn from(err: SPVError) -> Self { Self::SPVError(err.to_string()) }
}

impl From<serialization::Error> for ValidatePaymentError {
    fn from(err: serialization::Error) -> Self { Self::InternalError(err.to_string()) }
}

impl From<ScriptHashTypeNotSupported> for ValidatePaymentError {
    fn from(err: ScriptHashTypeNotSupported) -> Self { Self::ScriptHashTypeNotSupported(err.to_string()) }
}

impl From<UnexpectedDerivationMethod> for ValidatePaymentError {
    fn from(err: UnexpectedDerivationMethod) -> Self { Self::UnexpectedDerivationMethod(err) }
}

impl From<ethkey::Error> for ValidatePaymentError {
    fn from(err: ethkey::Error) -> Self { ValidatePaymentError::InvalidPaymentTxData(err.to_string()) }
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

    pub fn wrong_token_addr<Found, Expected>(found: Found, expected: Expected) -> ValidatePaymentError
    where
        Found: std::fmt::Debug,
        Expected: std::fmt::Debug,
    {
        ValidatePaymentError::WrongTokenAddress {
            found: format!("{:?}", found),
            expected: format!("{:?}", expected),
        }
    }

    pub fn wrong_swap_id<Found, Expected>(found: Found, expected: Expected) -> ValidatePaymentError
    where
        Found: std::fmt::Debug,
        Expected: std::fmt::Debug,
    {
        ValidatePaymentError::WrongSwapId {
            found: format!("{:?}", found),
            expected: format!("{:?}", expected),
        }
    }

    pub fn wrong_receiver<Found, Expected>(found: Found, expected: Expected) -> ValidatePaymentError
    where
        Found: std::fmt::Debug,
        Expected: std::fmt::Debug,
    {
        ValidatePaymentError::WrongReceiver {
            found: format!("{:?}", found),
            expected: format!("{:?}", expected),
        }
    }

    pub fn wrong_secret_hash<Found, Expected>(found: Found, expected: Expected) -> ValidatePaymentError
    where
        Found: std::fmt::Debug,
        Expected: std::fmt::Debug,
    {
        ValidatePaymentError::WrongSecretHash {
            found: format!("{:?}", found),
            expected: format!("{:?}", expected),
        }
    }

    pub fn wrong_timelock<Found, Expected>(found: Found, expected: Expected) -> ValidatePaymentError
    where
        Found: std::fmt::Debug,
        Expected: std::fmt::Debug,
    {
        ValidatePaymentError::WrongTimeLock {
            found: format!("{:?}", found),
            expected: format!("{:?}", expected),
        }
    }

    pub fn wrong_value<Found, Expected>(found: Found, expected: Expected) -> ValidatePaymentError
    where
        Found: std::fmt::Debug,
        Expected: std::fmt::Debug,
    {
        ValidatePaymentError::WrongValue {
            found: format!("{:?}", found),
            expected: format!("{:?}", expected),
        }
    }
}

#[derive(Debug, Display)]
pub enum MyAddressError {
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    Deprecated(String),
    InternalError(String),
}

impl From<UnexpectedDerivationMethod> for MyAddressError {
    fn from(err: UnexpectedDerivationMethod) -> Self { Self::UnexpectedDerivationMethod(err) }
}

impl From<String> for MyAddressError {
    fn from(err: String) -> Self { Self::InternalError(err) }
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

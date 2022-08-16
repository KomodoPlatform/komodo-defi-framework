use crate::{utxo::qtum::ScriptHashTypeNotSupported, NumConversError, UnexpectedDerivationMethod};
use spv_validation::helpers_validation::SPVError;

#[derive(Debug, Display, PartialEq)]
pub enum ValidatePaymentError {
    AbiError(String),
    AddressParseError(String),
    InternalError(String),
    InvalidPaymentTxData(String),
    MissingTx(String),
    PaymentStatusError(String),
    ScriptHashTypeNotSupported(String),
    SPVError(String),
    Transport(String),
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    UnexpectedErc20PaymentData(String),
    UnexpectedPaymentState(String),
    UnexpectedPaymentTx(String),
    ValidateHtlcError(String),
    WrongSenderAddress(String),
    WrongReceiverAddress(String),
}

impl From<ethabi::Error> for ValidatePaymentError {
    fn from(err: ethabi::Error) -> Self { Self::AbiError(err.to_string()) }
}

impl From<rlp::DecoderError> for ValidatePaymentError {
    fn from(err: rlp::DecoderError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<keys::Error> for ValidatePaymentError {
    fn from(err: keys::Error) -> Self { Self::InternalError(err.to_string()) }
}

impl From<web3::Error> for ValidatePaymentError {
    fn from(err: web3::Error) -> Self { Self::Transport(err.to_string()) }
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

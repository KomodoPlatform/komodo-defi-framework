use common::jsonrpc_client::JsonRpcError;
use derive_more::Display;
use hex::FromHexError;
use spv_validation::helpers_validation::SPVError;

use crate::{eth::{TokenDecimalsError, TryToAddressError, Web3RpcError},
            qrc20::script_pubkey::ScriptExtractionError,
            utxo::{qtum::ScriptHashTypeNotSupported, rpc_clients::UtxoRpcError, utxo_builder::UtxoConfError,
                   BroadcastTxErr, UnsupportedAddr},
            NumConversError, UnexpectedDerivationMethod};

#[derive(Debug, Display)]
pub enum CheckPaymentSentError {
    AddressParseError(String),
    AddrImportFailed(String),
    Erc20PaymentDetailsError(String),
    DeserializationErr(String),
    JsonRpcError(JsonRpcError),
    PaymentStatusError(String),
    PublicKeyErr(String),
    SignTxError(String),
    TransportError(String),
    TryToAddressError(TryToAddressError),
    UtxoRpcError(UtxoRpcError),
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
}

impl From<keys::Error> for CheckPaymentSentError {
    fn from(err: keys::Error) -> Self { Self::PublicKeyErr(err.to_string()) }
}

impl From<PaymentStatusError> for CheckPaymentSentError {
    fn from(err: PaymentStatusError) -> Self { Self::PaymentStatusError(err.to_string()) }
}

impl From<JsonRpcError> for CheckPaymentSentError {
    fn from(err: JsonRpcError) -> Self { Self::JsonRpcError(err) }
}

impl From<UtxoRpcError> for CheckPaymentSentError {
    fn from(err: UtxoRpcError) -> Self { Self::UtxoRpcError(err) }
}

impl From<AddressParseError> for CheckPaymentSentError {
    fn from(err: AddressParseError) -> Self { Self::AddressParseError(err.to_string()) }
}

impl From<serialization::Error> for CheckPaymentSentError {
    fn from(err: serialization::Error) -> Self { Self::DeserializationErr(err.to_string()) }
}

impl From<UnexpectedDerivationMethod> for CheckPaymentSentError {
    fn from(err: UnexpectedDerivationMethod) -> Self { Self::UnexpectedDerivationMethod(err) }
}

#[derive(Debug, Display, PartialEq)]
pub enum ExtractSecretError {
    DeserializationErr(String),
    DecodingError(String),
    ExtractionFailed(String),
    CouldNotObtainSecret(String),
    #[display(fmt = "Invalid arguments in 'receiverSpend' call: {:?}", _0)]
    InvalidArguments(Vec<ethabi::Token>),
    #[display(fmt = "Expected secret to be fixed bytes, decoded function data is {:?}", _0)]
    ExpectedFixedBytes(Vec<ethabi::Token>),
}

impl From<ethabi::Error> for ExtractSecretError {
    fn from(err: ethabi::Error) -> Self { Self::DecodingError(err.to_string()) }
}

impl From<rlp::DecoderError> for ExtractSecretError {
    fn from(err: rlp::DecoderError) -> Self { Self::DecodingError(err.to_string()) }
}

impl From<serialization::Error> for ExtractSecretError {
    fn from(err: serialization::Error) -> Self { Self::DeserializationErr(err.to_string()) }
}

#[derive(Debug, Display, PartialEq)]
pub enum MyAddressError {
    AddressParseError(AddressParseError),
    AddrDisplayError(String),
    #[display(fmt = "DeprecatedWalletAddr: 'my_address' is deprecated for HD wallets")]
    DeprecatedWalletAddr,
    EncodingError(String),
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
    Internal(String),
    UnsupportedAddr(String),
    MethodNotSupported(String),
    ScriptHashTypeNotSupported {
        script_hash_type: String,
    },
    CashAddressErr(String),
    #[display(fmt = "Transaction Reading Error: {}", _0)]
    TxReadError(String),
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    UtxoRpcError(String),
}

impl From<AddressParseError> for MyAddressError {
    fn from(e: AddressParseError) -> Self { MyAddressError::AddressParseError(e) }
}

impl From<UnexpectedDerivationMethod> for MyAddressError {
    fn from(e: UnexpectedDerivationMethod) -> Self { MyAddressError::UnexpectedDerivationMethod(e) }
}

impl From<keys::Error> for MyAddressError {
    fn from(e: keys::Error) -> Self { MyAddressError::Internal(e.to_string()) }
}

impl From<UnsupportedAddr> for MyAddressError {
    fn from(e: UnsupportedAddr) -> Self { MyAddressError::UnsupportedAddr(e.to_string()) }
}

#[derive(Debug, Display, PartialEq)]
pub enum AddressParseError {
    AddrFormatParseError(String),
    AddrDisplayError(String),
    AddrConversionErr(String),
    CashAddressErr(String),
    EncodeError(String),
    InvalidHexError(String),
    Internal(String),
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
    #[display(fmt = "Invalid address checksum")]
    InvalidAddressCheckSum,
    InvalidAddressPrefix(String),
    MyAddressError(String),
    PlatformConfIsNull(String),
    ParsingError(String),
    UnsupportedAddr(String),
    UnsupportedProtocol(String),
    UnexpectedProtocol(String),
    ScriptHashTypeNotSupported {
        script_hash_type: String,
    },
    #[display(fmt = "Transaction Reading Error: {}", _0)]
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    #[display(fmt = "Topic {:?} is expected to be H256 encoded topic (with length of 64)", _0)]
    UnexpectedTopicLen(String),
    UtxoConfError(String),
}

impl From<keys::Error> for AddressParseError {
    fn from(e: keys::Error) -> Self { AddressParseError::Internal(e.to_string()) }
}

impl From<MyAddressError> for AddressParseError {
    fn from(e: MyAddressError) -> Self { AddressParseError::MyAddressError(e.to_string()) }
}

impl From<FromHexError> for AddressParseError {
    fn from(e: FromHexError) -> Self { AddressParseError::InvalidHexError(e.to_string()) }
}

impl From<UtxoConfError> for AddressParseError {
    fn from(e: UtxoConfError) -> Self { AddressParseError::UtxoConfError(e.to_string()) }
}

impl From<serde_json::Error> for AddressParseError {
    fn from(e: serde_json::Error) -> Self { AddressParseError::Internal(e.to_string()) }
}

impl From<secp256k1::Error> for AddressParseError {
    fn from(e: secp256k1::Error) -> Self { AddressParseError::Internal(e.to_string()) }
}

#[derive(Display, Debug)]
pub enum SignedEthTxError {
    DecoderError(rlp::DecoderError),
    VerificationError(ethkey::Error),
}

impl From<rlp::DecoderError> for SignedEthTxError {
    fn from(err: rlp::DecoderError) -> Self { Self::DecoderError(err) }
}

impl From<ethkey::Error> for SignedEthTxError {
    fn from(err: ethkey::Error) -> Self { Self::VerificationError(err) }
}

#[derive(Debug, Display)]
pub enum SendRawTxError {
    BroadcastTxErr(String),
    BlockchainScanStopped(String),
    ClientError(String),
    DeserializationErr(String),
    InvalidHex(String),
    Internal(String),
    MethodNotSupported(String),
    TransportError(String),
    TxReadError(String),
    UtxoRpcError(String),
}

impl From<BroadcastTxErr> for SendRawTxError {
    fn from(err: BroadcastTxErr) -> Self { Self::BroadcastTxErr(err.to_string()) }
}

impl From<hex::FromHexError> for SendRawTxError {
    fn from(err: hex::FromHexError) -> Self { Self::InvalidHex(err.to_string()) }
}

impl From<serialization::Error> for SendRawTxError {
    fn from(err: serialization::Error) -> Self { Self::DeserializationErr(err.to_string()) }
}

impl From<UtxoRpcError> for SendRawTxError {
    fn from(err: UtxoRpcError) -> Self { Self::UtxoRpcError(err.to_string()) }
}

#[derive(Debug, Display, PartialEq)]
pub enum ValidatePaymentError {
    AddressError(String),
    Erc20PaymentDetailsError(String),
    #[display(fmt = "Iguana private key is unavailable")]
    IguanaPrivKeyUnavailable,
    #[display(fmt = "InternalError: {}", _0)]
    InternalError(String),
    InvalidSwapId(String),
    InvalidTxTokenAddrArg(String),
    InvalidTxSecretHash(String),
    InvalidTxTimeLockArg(String),
    InvalidTxReceiveArg(String),
    InvalidTxValue(String),
    MissingTx(String),
    PaymentStatusError(String),
    TryToAddressError(TryToAddressError),
    UtxoRpcError(String),
    UnexpectedPaymentOutput(String),
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    ValidateHtlcError(String),
    WrongSenderAddress(String),
    WrongReceiverAddress(String),
}

impl From<rlp::DecoderError> for ValidatePaymentError {
    fn from(err: rlp::DecoderError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<ethabi::Error> for ValidatePaymentError {
    fn from(err: ethabi::Error) -> Self { Self::InternalError(err.to_string()) }
}

impl From<keys::Error> for ValidatePaymentError {
    fn from(err: keys::Error) -> Self { Self::InternalError(err.to_string()) }
}

impl From<web3::Error> for ValidatePaymentError {
    fn from(err: web3::Error) -> Self { Self::InternalError(err.to_string()) }
}

impl From<NumConversError> for ValidatePaymentError {
    fn from(err: NumConversError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<SPVError> for ValidatePaymentError {
    fn from(err: SPVError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<serialization::Error> for ValidatePaymentError {
    fn from(err: serialization::Error) -> Self { Self::InternalError(err.to_string()) }
}

impl From<ScriptHashTypeNotSupported> for ValidatePaymentError {
    fn from(err: ScriptHashTypeNotSupported) -> Self { Self::InternalError(err.to_string()) }
}

impl From<UnexpectedDerivationMethod> for ValidatePaymentError {
    fn from(err: UnexpectedDerivationMethod) -> Self { Self::UnexpectedDerivationMethod(err) }
}

impl From<AddressParseError> for ValidatePaymentError {
    fn from(err: AddressParseError) -> Self { Self::AddressError(err.to_string()) }
}

impl From<MyAddressError> for ValidatePaymentError {
    fn from(err: MyAddressError) -> Self { Self::AddressError(err.to_string()) }
}

#[derive(Debug, Display)]
pub enum PaymentStatusError {
    EthAbiError(ethabi::Error),
    ExpectedUintForPaymentStatus(String),
    InvalidTx(String),
    PaymentStatusError(ValidatePaymentError),
    Transport(String),
    #[display(fmt = "Expected at least 3 tokens in \"payments\" call, found {}", _0)]
    UnexpectedTokenNumbers(usize),
    UtxoRpcError(String),
}

impl From<ethabi::Error> for PaymentStatusError {
    fn from(err: ethabi::Error) -> Self { Self::EthAbiError(err) }
}

impl From<PaymentStatusError> for ValidatePaymentError {
    fn from(err: PaymentStatusError) -> Self { Self::PaymentStatusError(err.to_string()) }
}

impl From<UtxoRpcError> for ValidatePaymentError {
    fn from(err: UtxoRpcError) -> Self { Self::UtxoRpcError(err.to_string()) }
}

#[derive(Debug, Display)]
pub enum EthCoinParseError {
    AddressParseError(AddressParseError),
    JsonError(String),
    Internal(String),
    InvalidUrl(String),
    KeyPairError(String),
    #[display(fmt = "swap_contract_address can't be zero address")]
    NoSwapContractAddr,
    #[display(fmt = "fallback_swap_contract can't be zero address")]
    NoFallBackSwapContractAddr,
    TokenDecimalsError(TokenDecimalsError),
    #[display(fmt = "Failed to get client version for all urls")]
    UnableToGetUrlsClientVersion,
    #[display(fmt = "Enable request for ETH coin must have at least 1 node URL")]
    UnexpectedNumOfNodeUrl,
    #[display(fmt = "Expect ETH or ERC20 protocol")]
    UnexpectedProtocol,
}

impl From<serde_json::Error> for EthCoinParseError {
    fn from(err: serde_json::Error) -> Self { Self::JsonError(err.to_string()) }
}

impl From<ethkey::Error> for EthCoinParseError {
    fn from(err: ethkey::Error) -> Self { Self::KeyPairError(err.to_string()) }
}

impl From<AddressParseError> for EthCoinParseError {
    fn from(err: AddressParseError) -> Self { Self::AddressParseError(err) }
}

#[derive(Debug, Display)]
pub enum GetTradeFeeError {
    Web3RpcError(String),
    NumConversError(NumConversError),
    UtxoRpcError(String),
}

impl From<Web3RpcError> for GetTradeFeeError {
    fn from(err: Web3RpcError) -> Self { Self::Web3RpcError(err.to_string()) }
}

impl From<NumConversError> for GetTradeFeeError {
    fn from(err: NumConversError) -> Self { Self::NumConversError(err) }
}

impl From<UtxoRpcError> for GetTradeFeeError {
    fn from(err: UtxoRpcError) -> Self { Self::UtxoRpcError(err.to_string()) }
}

#[derive(Debug, Display)]
pub enum WaitForConfirmationsErr {
    CheckContractCallError(String),
    DecoderError(String),
    DeserializationErr(String),
    Internal(String),
    JsonRpcError(String),
    #[display(fmt = "OutputIndexOutOfBounds: TxReceipt::output_index out of bounds")]
    OutputIndexOutOfBounds,
    ScriptExtractionError(String),
    SignedEthTx(String),
    Transport(String),
    TxExpired(String),
    TxStatusFailed(String),
    TxWaitTimeDue(String),
}

impl From<rlp::DecoderError> for WaitForConfirmationsErr {
    fn from(err: rlp::DecoderError) -> Self { Self::DecoderError(err.to_string()) }
}

impl From<serialization::Error> for WaitForConfirmationsErr {
    fn from(err: serialization::Error) -> Self { Self::DeserializationErr(err.to_string()) }
}

impl From<JsonRpcError> for WaitForConfirmationsErr {
    fn from(err: JsonRpcError) -> Self { Self::JsonRpcError(err.to_string()) }
}

impl From<ScriptExtractionError> for WaitForConfirmationsErr {
    fn from(err: ScriptExtractionError) -> Self { Self::ScriptExtractionError(err.to_string()) }
}

impl From<ethkey::Error> for WaitForConfirmationsErr {
    fn from(err: ethkey::Error) -> Self { Self::SignedEthTx(err.to_string()) }
}

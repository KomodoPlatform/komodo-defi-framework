use crate::siacoin::{Address, Currency, Event, EventDataWrapper, ParseHashError, PreimageError, PrivateKeyError,
                     PublicKeyError, SiaApiClientError, SiaClientHelperError, TransactionId, V2TransactionBuilderError};
use crate::{DexFee, TransactionEnum};
use common::executor::AbortedError;
use mm2_number::BigDecimal;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum SiacoinToHastingsError {
    #[error("Sia Failed to convert BigDecimal:{0} to BigInt")]
    BigDecimalToBigInt(BigDecimal),
    #[error("Sia Failed to convert BigDecimal:{0} to u128")]
    BigIntToU128(BigDecimal),
}

#[derive(Debug, Error)]
pub enum SendTakerFeeError {
    #[error("SiaCoin::new_send_taker_fee: failed to parse uuid from bytes {0}")]
    ParseUuid(#[from] uuid::Error),
    #[error("SiaCoin::new_send_taker_fee: Unexpected Uuid version {0}")]
    UuidVersion(usize),
    #[error("SiaCoin::new_send_taker_fee: failed to convert trade_fee_amount to Currency {0}")]
    SiacoinToHastings(#[from] SiacoinToHastingsError),
    #[error("SiaCoin::new_send_taker_fee: unexpected DexFee variant: {0:?}")]
    DexFeeVariant(DexFee),
    #[error("SiaCoin::new_send_taker_fee: failed to fetch my_keypair {0}")]
    MyKeypair(#[from] SiaCoinError),
    #[error("SiaCoin::new_send_taker_fee: failed to fund transaction {0}")]
    FundTx(SiaClientHelperError),
    #[error("SiaCoin::new_send_taker_fee: failed to broadcast taker_fee transaction {0}")]
    BroadcastTx(SiaClientHelperError),
}

#[derive(Debug, Error)]
pub enum SendMakerPaymentError {
    #[error("SiaCoin::new_send_maker_payment: invalid taker pubkey {0}")]
    InvalidTakerPublicKey(#[from] PublicKeyError),
    #[error("SiaCoin::new_send_maker_payment: failed to fetch my_keypair {0}")]
    MyKeypair(#[from] SiaCoinError),
    #[error("SiaCoin::new_send_maker_payment: failed to convert trade amount to Currency {0}")]
    SiacoinToHastings(#[from] SiacoinToHastingsError),
    #[error("SiaCoin::new_send_maker_payment: failed to fund transaction {0}")]
    FundTx(SiaClientHelperError),
    #[error("SiaCoin::new_send_maker_payment: failed to parse secret_hash {0}")]
    ParseSecretHash(#[from] ParseHashError),
    #[error("SiaCoin::new_send_maker_payment: failed to broadcast maker_payment transaction {0}")]
    BroadcastTx(SiaClientHelperError),
}

#[derive(Debug, Error)]
pub enum SendTakerPaymentError {
    #[error("SiaCoin::new_send_taker_payment: invalid taker pubkey {0}")]
    InvalidMakerPublicKey(#[from] PublicKeyError),
    #[error("SiaCoin::new_send_taker_payment: failed to fetch my_keypair {0}")]
    MyKeypair(#[from] SiaCoinError),
    #[error("SiaCoin::new_send_taker_payment: failed to convert trade amount to Currency {0}")]
    SiacoinToHastings(#[from] SiacoinToHastingsError),
    #[error("SiaCoin::new_send_taker_payment: failed to fund transaction {0}")]
    FundTx(SiaClientHelperError),
    #[error("SiaCoin::new_send_taker_payment: invalid secret_hash length {0}")]
    SecretHashLength(#[from] ParseHashError),
    #[error("SiaCoin::new_send_taker_payment: failed to broadcast taker_payment transaction {0}")]
    BroadcastTx(SiaClientHelperError),
}

/// Wrapper around SendRefundHltcError to allow indicating Maker or Taker context within the error
#[derive(Debug, Error)]
pub enum SendRefundHltcMakerOrTakerError {
    #[error("SiaCoin::send_refund_hltc: maker: {0}")]
    Maker(SendRefundHltcError),
    #[error("SiaCoin::send_refund_hltc: taker: {0}")]
    Taker(SendRefundHltcError),
}

#[derive(Debug, Error)]
pub enum SendRefundHltcError {
    #[error("SiaCoin::send_refund_hltc: failed to fetch my_keypair: {0}")]
    MyKeypair(#[from] SiaCoinError),
    #[error("SiaCoin::send_refund_hltc: failed to parse RefundPaymentArgs: {0}")]
    ParseArgs(#[from] SiaRefundPaymentArgsError),
    #[error("SiaCoin::send_refund_hltc: failed to fetch SiacoinElement from txid {0}")]
    UtxoFromTxid(SiaClientHelperError),
    #[error("SiaCoin::send_refund_hltc: failed to satisfy HTLC SpendPolicy {0}")]
    SatisfyHtlc(#[from] V2TransactionBuilderError),
    #[error("SiaCoin::send_refund_hltc: failed to broadcast transaction {0}")]
    BroadcastTx(SiaClientHelperError),
}

#[derive(Debug, Error)]
pub enum ValidateFeeError {
    #[error("SiaCoin::new_validate_fee: failed to parse ValidateFeeArgs {0}")]
    ParseArgs(#[from] SiaValidateFeeArgsError),
    #[error("SiaCoin::new_validate_fee: failed to fetch fee_tx event {0}")]
    FetchEvent(#[from] SiaClientHelperError),
    #[error("SiaCoin::new_validate_fee: tx confirmed before min_block_number:{min_block_number} event:{event:?}")]
    MininumHeight { event: Event, min_block_number: u64 },
    #[error("SiaCoin::new_validate_fee: all inputs do not originate from taker address txid:{0}")]
    InputsOrigin(TransactionId),
    #[error("SiaCoin::new_validate_fee: fee_tx:{txid} has {outputs_length} outputs, expected 1")]
    VoutLength { txid: TransactionId, outputs_length: usize },
    #[error("SiaCoin::new_validate_fee: fee_tx:{txid} pays wrong address:{address}")]
    InvalidFeeAddress { txid: TransactionId, address: Address },
    #[error("SiaCoin::new_validate_fee: fee_tx:{txid} pays wrong amount. expected:{expected} actual:{actual}")]
    InvalidFeeAmount {
        txid: TransactionId,
        expected: Currency,
        actual: Currency,
    },
    #[error("SiaCoin::new_validate_fee: failed to parse uuid from arbitrary_bytes {0}")]
    ParseUuid(#[from] uuid::Error),
    #[error("SiaCoin::new_validate_fee: fee_tx:{txid} wrong uuid. expected:{expected} actual:{actual}")]
    InvalidUuid {
        txid: TransactionId,
        expected: Uuid,
        actual: Uuid,
    },
}

// TODO Alright - nearly identical to MakerSpendsTakerPaymentError
// refactor similar to SendRefundHltcMakerOrTakerError
#[derive(Debug, Error)]
pub enum TakerSpendsMakerPaymentError {
    #[error("SiaCoin::new_send_taker_spends_maker_payment: failed to fetch my_keypair {0}")]
    MyKeypair(#[from] SiaCoinError),
    #[error("SiaCoin::new_send_taker_spends_maker_payment: invalid maker pubkey {0}")]
    InvalidMakerPublicKey(#[from] PublicKeyError),
    #[error("SiaCoin::new_send_taker_spends_maker_paymentt: failed to parse taker_payment_tx {0}")]
    ParseTx(#[from] SiaTransactionError),
    #[error("SiaCoin::new_send_taker_spends_maker_payment: failed to parse secret {0}")]
    ParseSecret(#[from] PreimageError),
    #[error("SiaCoin::new_send_taker_spends_maker_payment: failed to parse secret_hash {0}")]
    ParseSecretHash(#[from] ParseHashError),
    #[error("SiaCoin::new_send_taker_spends_maker_payment: failed to fetch SiacoinElement from txid {0}")]
    UtxoFromTxid(SiaClientHelperError),
    #[error("SiaCoin::new_send_taker_spends_maker_payment: failed to satisfy HTLC SpendPolicy {0}")]
    SatisfyHtlc(#[from] V2TransactionBuilderError),
    #[error("SiaCoin::new_send_taker_spends_maker_payment: failed to broadcast spend_maker_payment transaction {0}")]
    BroadcastTx(SiaClientHelperError),
}

#[derive(Debug, Error)]
pub enum MakerSpendsTakerPaymentError {
    #[error("SiaCoin::new_send_maker_spends_taker_payment: failed to fetch my_keypair {0}")]
    MyKeypair(#[from] SiaCoinError),
    #[error("SiaCoin::new_send_maker_spends_taker_payment: invalid taker pubkey {0}")]
    InvalidTakerPublicKey(#[from] PublicKeyError),
    #[error("SiaCoin::new_send_maker_spends_taker_payment: failed to parse taker_payment_tx {0}")]
    ParseTx(#[from] SiaTransactionError),
    #[error("SiaCoin::new_send_maker_spends_taker_payment: failed to parse secret {0}")]
    ParseSecret(#[from] PreimageError),
    #[error("SiaCoin::new_send_maker_spends_taker_payment: failed to parse secret_hash {0}")]
    ParseSecretHash(#[from] ParseHashError),
    #[error("SiaCoin::new_send_maker_spends_taker_payment: failed to fetch SiacoinElement from txid {0}")]
    UtxoFromTxid(SiaClientHelperError),
    #[error("SiaCoin::new_send_maker_spends_taker_payment: failed to satisfy HTLC SpendPolicy {0}")]
    SatisfyHtlc(#[from] V2TransactionBuilderError),
    #[error("SiaCoin::new_send_maker_spends_taker_payment: failed to broadcast spend_taker_payment transaction {0}")]
    BroadcastTx(SiaClientHelperError),
}

#[derive(Debug, Error)]
pub enum SiaRefundPaymentArgsError {
    #[error("SiaRefundPaymentArgs: failed to parse other_pubkey {0}")]
    ParseOtherPublicKey(#[from] PublicKeyError),
    #[error("SiaRefundPaymentArgs: failed to parse payment_tx {0}")]
    ParseTx(#[from] SiaTransactionError),
    #[error("SiaRefundPaymentArgs: failed to parse secret_hash {0}")]
    ParseSecretHash(#[from] ParseHashError),
    // SwapTxTypeVariant uses String Debug trait representation to avoid explicit lifetime annotations
    // otherwise this should be SwapTxTypeVariant(SwapTxTypeWithSecretHash) and displayed via {0:?}
    #[error("SiaRefundPaymentArgs: unexpected SwapTxTypeWithSecretHash variant {0}")]
    SwapTxTypeVariant(String),
}

#[derive(Debug, Error)]
pub enum SiaValidateFeeArgsError {
    #[error("SiaValidateFeeArgs::TryFrom<ValidateFeeArgs>: failed to parse uuid from bytes {0}")]
    ParseUuid(#[from] uuid::Error),
    #[error("SiaValidateFeeArgs::TryFrom<ValidateFeeArgs>: Unexpected Uuid version {0}")]
    UuidVersion(usize),
    #[error("SiaValidateFeeArgs::TryFrom<ValidateFeeArgs>: invalid taker pubkey {0}")]
    InvalidTakerPublicKey(#[from] PublicKeyError),
    #[error("SiaValidateFeeArgs::TryFrom<ValidateFeeArgs>: failed to convert trade_fee_amount to Currency {0}")]
    SiacoinToHastings(#[from] SiacoinToHastingsError),
    #[error("SiaValidateFeeArgs::TryFrom<ValidateFeeArgs>: unexpected DexFee variant {0:?}")]
    DexFeeVariant(DexFee),
    #[error("SiaValidateFeeArgs::TryFrom<ValidateFeeArgs>: unexpected TransactionEnum variant {0:?}")]
    TxEnumVariant(TransactionEnum),
}

#[derive(Debug, Error)]
pub enum SiaTransactionError {
    #[error("Vec<u8>::TryFrom<SiaTransaction>: failed to convert to Vec<u8>")]
    ToVec(serde_json::Error),
    #[error("SiaTransaction::TryFrom<Vec<u8>>: failed to convert from Vec<u8>")]
    FromVec(serde_json::Error),
}

#[derive(Debug, Error)]
pub enum SiaCoinBuilderError {
    #[error("SiaCoinBuilder::build: failed to create abortable system: {0}")]
    AbortableSystem(AbortedError),
    #[error("SiaCoinBuilder::build: failed to initialize client {0}")]
    Client(#[from] SiaApiClientError),
}

// This is required because AbortedError doesn't impl Error
impl From<AbortedError> for SiaCoinBuilderError {
    fn from(e: AbortedError) -> Self { SiaCoinBuilderError::AbortableSystem(e) }
}

#[derive(Debug, Error)]
pub enum SiaCoinError {
    #[error("SiaCoin::from_conf_and_request: failed to parse SiaCoinConf from JSON: {0}")]
    InvalidConf(#[from] serde_json::Error),
    #[error("SiaCoin::from_conf_and_request: invalid private key: {0}")]
    InvalidPrivateKey(#[from] PrivateKeyError),
    #[error("SiaCoin::from_conf_and_request: invalid private key policy, must use iguana seed")]
    UnsupportedPrivKeyPolicy,
    #[error("SiaCoin::from_conf_and_request: failed to build SiaCoin: {0}")]
    Builder(#[from] SiaCoinBuilderError),
    #[error("SiaCoin::my_keypair: invalid private key policy, must use iguana seed")]
    MyKeypairPrivKeyPolicy,
}

#[derive(Debug, Error)]
pub enum SiaCheckIfMyPaymentSentArgsError {
    #[error("SiaCheckIfMyPaymentSentArgs::TryFrom<CheckIfMyPaymentSentArgs>: failed to parse other_pub {0}")]
    ParseOtherPublicKey(#[from] PublicKeyError),
    #[error("SiaCheckIfMyPaymentSentArgs::TryFrom<CheckIfMyPaymentSentArgs>: failed to parse secret_hash {0}")]
    ParseSecretHash(#[from] ParseHashError),
    #[error(
        "SiaCheckIfMyPaymentSentArgs::TryFrom<CheckIfMyPaymentSentArgs>: failed to convert amount to Currency {0}"
    )]
    SiacoinToHastings(#[from] SiacoinToHastingsError),
}

#[derive(Debug, Error)]
pub enum SiaCheckIfMyPaymentSentError {
    #[error("SiaCoin::new_check_if_my_payment_sent: failed to parse CheckIfMyPaymentSentArgs: {0}")]
    ParseArgs(#[from] SiaCheckIfMyPaymentSentArgsError),
    #[error("SiaCoin::new_check_if_my_payment_sent: invalid private key policy, must use iguana seed")]
    MyKeypair(#[from] SiaCoinError),
    #[error("SiaCoin::new_check_if_my_payment_sent: failed to fetch address events: {0}")]
    FetchEvents(#[from] SiaClientHelperError),
    #[error("SiaCoin::new_check_if_my_payment_sent: expected to find single event found: {0:?}")]
    MultipleEvents(Vec<Event>),
    #[error("SiaCoin::new_check_if_my_payment_sent: unexpected event variant: {0:?}")]
    EventVariant(EventDataWrapper),
}

use lightning_invoice::Invoice;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json, H264 as H264Json};
use serde::{Deserialize, Serialize};
use skip_serializing_none::skip_serializing_none;
use std::collections::HashMap;
use uuid::Uuid;

use mm2_number::BigDecimal;
use mm2_rpc::data::legacy::{MySwapsFilter, RecoveredSwapAction};

#[derive(Serialize)]
#[serde(tag = "method", rename = "active_swaps")]
pub(crate) struct ActiveSwapsRequest {
    pub(crate) include_status: bool,
}

#[derive(Deserialize)]
pub(crate) struct ActiveSwapsResponse {
    #[allow(dead_code)]
    pub(crate) uuids: Vec<Uuid>,
    pub(crate) statuses: Option<HashMap<Uuid, SavedSwap>>,
}

#[derive(Serialize)]
#[serde(tag = "method", rename = "my_swap_status")]
pub(crate) struct MySwapStatusRequest {
    pub(crate) params: Params,
}

#[derive(Serialize)]
pub(crate) struct Params {
    pub(crate) uuid: Uuid,
}

#[derive(Serialize)]
#[serde(tag = "method", rename = "recover_funds_of_swap")]
pub(crate) struct RecoverFundsOfSwapRequest {
    pub(crate) params: Params,
}

#[derive(Deserialize)]
pub(crate) struct RecoverFundsOfSwapResponse {
    pub(crate) action: RecoveredSwapAction,
    pub(crate) coin: String,
    pub(crate) tx_hash: BytesJson,
    pub(crate) tx_hex: BytesJson,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum SavedSwap {
    Maker(MakerSavedSwap),
    Taker(TakerSavedSwap),
}

#[derive(Debug, Deserialize)]
pub(crate) struct MakerSavedSwap {
    pub(crate) uuid: Uuid,
    pub(crate) my_order_uuid: Option<Uuid>,
    pub(crate) events: Vec<MakerSavedEvent>,
    pub(crate) maker_amount: Option<BigDecimal>,
    pub(crate) maker_coin: Option<String>,
    pub(crate) maker_coin_usd_price: Option<BigDecimal>,
    pub(crate) taker_amount: Option<BigDecimal>,
    pub(crate) taker_coin: Option<String>,
    pub(crate) taker_coin_usd_price: Option<BigDecimal>,
    pub(crate) gui: Option<String>,
    pub(crate) mm_version: Option<String>,
    #[allow(dead_code)]
    pub(crate) success_events: Vec<String>,
    #[allow(dead_code)]
    pub(crate) error_events: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TakerSavedSwap {
    pub(crate) uuid: Uuid,
    pub(crate) my_order_uuid: Option<Uuid>,
    pub(crate) events: Vec<TakerSavedEvent>,
    pub(crate) maker_amount: Option<BigDecimal>,
    pub(crate) maker_coin: Option<String>,
    pub(crate) maker_coin_usd_price: Option<BigDecimal>,
    pub(crate) taker_amount: Option<BigDecimal>,
    pub(crate) taker_coin: Option<String>,
    pub(crate) taker_coin_usd_price: Option<BigDecimal>,
    pub(crate) gui: Option<String>,
    pub(crate) mm_version: Option<String>,
    #[allow(dead_code)]
    pub(crate) success_events: Vec<String>,
    #[allow(dead_code)]
    pub(crate) error_events: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct MakerSavedEvent {
    pub(crate) timestamp: u64,
    pub(crate) event: MakerSwapEvent,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TakerSavedEvent {
    pub(crate) timestamp: u64,
    pub(crate) event: TakerSwapEvent,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", content = "data")]
#[allow(clippy::large_enum_variant)]
pub(crate) enum MakerSwapEvent {
    Started(MakerSwapData),
    StartFailed(SwapError),
    Negotiated(TakerNegotiationData),
    NegotiateFailed(SwapError),
    MakerPaymentInstructionsReceived(Option<PaymentInstructions>),
    TakerFeeValidated(TransactionIdentifier),
    TakerFeeValidateFailed(SwapError),
    MakerPaymentSent(TransactionIdentifier),
    MakerPaymentTransactionFailed(SwapError),
    MakerPaymentDataSendFailed(SwapError),
    MakerPaymentWaitConfirmFailed(SwapError),
    TakerPaymentReceived(TransactionIdentifier),
    TakerPaymentWaitConfirmStarted,
    TakerPaymentValidatedAndConfirmed,
    TakerPaymentValidateFailed(SwapError),
    TakerPaymentWaitConfirmFailed(SwapError),
    TakerPaymentSpent(TransactionIdentifier),
    TakerPaymentSpendFailed(SwapError),
    TakerPaymentSpendConfirmStarted,
    TakerPaymentSpendConfirmed,
    TakerPaymentSpendConfirmFailed(SwapError),
    MakerPaymentWaitRefundStarted { wait_until: u64 },
    MakerPaymentRefundStarted,
    MakerPaymentRefunded(Option<TransactionIdentifier>),
    MakerPaymentRefundFailed(SwapError),
    MakerPaymentRefundFinished,
    Finished,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct MakerSwapData {
    pub(crate) taker_coin: String,
    pub(crate) maker_coin: String,
    pub(crate) taker: H256Json,
    pub(crate) secret: H256Json,
    pub(crate) secret_hash: Option<BytesJson>,
    pub(crate) my_persistent_pub: H264Json,
    pub(crate) lock_duration: u64,
    pub(crate) maker_amount: BigDecimal,
    pub(crate) taker_amount: BigDecimal,
    pub(crate) maker_payment_confirmations: u64,
    pub(crate) maker_payment_requires_nota: Option<bool>,
    pub(crate) taker_payment_confirmations: u64,
    pub(crate) taker_payment_requires_nota: Option<bool>,
    pub(crate) maker_payment_lock: u64,
    /// Allows to recognize one SWAP from the other in the logs. #274.
    pub(crate) uuid: Uuid,
    pub(crate) started_at: u64,
    pub(crate) maker_coin_start_block: u64,
    pub(crate) taker_coin_start_block: u64,
    /// A `MakerPayment` transaction fee.
    /// Note this value is used to calculate locked amount only.
    pub(crate) maker_payment_trade_fee: Option<SavedTradeFee>,
    /// A transaction fee that should be paid to spend a `TakerPayment`.
    /// Note this value is used to calculate locked amount only.
    pub(crate) taker_payment_spend_trade_fee: Option<SavedTradeFee>,
    pub(crate) maker_coin_swap_contract_address: Option<BytesJson>,
    pub(crate) taker_coin_swap_contract_address: Option<BytesJson>,
    /// Temporary pubkey used in HTLC redeem script when applicable for maker coin
    pub(crate) maker_coin_htlc_pubkey: Option<H264Json>,
    /// Temporary pubkey used in HTLC redeem script when applicable for taker coin
    pub(crate) taker_coin_htlc_pubkey: Option<H264Json>,
    /// Temporary privkey used to sign P2P messages when applicable
    pub(crate) p2p_privkey: Option<SerializableSecp256k1Keypair>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", content = "data")]
#[allow(clippy::large_enum_variant)]
pub(crate) enum TakerSwapEvent {
    Started(TakerSwapData),
    StartFailed(SwapError),
    Negotiated(MakerNegotiationData),
    NegotiateFailed(SwapError),
    TakerFeeSent(TransactionIdentifier),
    TakerFeeSendFailed(SwapError),
    TakerPaymentInstructionsReceived(Option<PaymentInstructions>),
    MakerPaymentReceived(TransactionIdentifier),
    MakerPaymentWaitConfirmStarted,
    MakerPaymentValidatedAndConfirmed,
    MakerPaymentValidateFailed(SwapError),
    MakerPaymentWaitConfirmFailed(SwapError),
    TakerPaymentSent(TransactionIdentifier),
    WatcherMessageSent(Option<Vec<u8>>, Option<Vec<u8>>),
    TakerPaymentTransactionFailed(SwapError),
    TakerPaymentDataSendFailed(SwapError),
    TakerPaymentWaitConfirmFailed(SwapError),
    TakerPaymentSpent(TakerPaymentSpentData),
    TakerPaymentWaitForSpendFailed(SwapError),
    MakerPaymentSpent(TransactionIdentifier),
    MakerPaymentSpendFailed(SwapError),
    TakerPaymentWaitRefundStarted { wait_until: u64 },
    TakerPaymentRefundStarted,
    TakerPaymentRefunded(Option<TransactionIdentifier>),
    TakerPaymentRefundFailed(SwapError),
    TakerPaymentRefundFinished,
    Finished,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct SwapError {
    pub(crate) error: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TakerNegotiationData {
    pub(crate) taker_payment_locktime: u64,
    pub(crate) taker_pubkey: H264Json,
    pub(crate) maker_coin_swap_contract_addr: Option<BytesJson>,
    pub(crate) taker_coin_swap_contract_addr: Option<BytesJson>,
    pub(crate) maker_coin_htlc_pubkey: Option<H264Json>,
    pub(crate) taker_coin_htlc_pubkey: Option<H264Json>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) enum PaymentInstructions {
    #[cfg(not(target_arch = "wasm32"))]
    Lightning(Invoice),
    WatcherReward(BigDecimal),
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TransactionIdentifier {
    /// Raw bytes of signed transaction in hexadecimal string, this should be sent as is to send_raw_transaction RPC to broadcast the transaction.
    /// Some payments like lightning payments don't have a tx_hex, for such payments tx_hex will be equal to tx_hash.
    pub(crate) tx_hex: BytesJson,
    /// Transaction hash in hexadecimal format
    pub(crate) tx_hash: BytesJson,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct SavedTradeFee {
    pub(crate) coin: String,
    pub(crate) amount: BigDecimal,
    #[serde(default)]
    pub(crate) paid_from_trading_vol: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct SerializableSecp256k1Keypair {
    pub(crate) inner: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct TakerSwapData {
    pub(crate) taker_coin: String,
    pub(crate) maker_coin: String,
    pub(crate) maker: H256Json,
    pub(crate) my_persistent_pub: H264Json,
    pub(crate) lock_duration: u64,
    pub(crate) maker_amount: BigDecimal,
    pub(crate) taker_amount: BigDecimal,
    pub(crate) maker_payment_confirmations: u64,
    pub(crate) maker_payment_requires_nota: Option<bool>,
    pub(crate) taker_payment_confirmations: u64,
    pub(crate) taker_payment_requires_nota: Option<bool>,
    pub(crate) taker_payment_lock: u64,
    /// Allows to recognize one SWAP from the other in the logs. #274.
    pub(crate) uuid: Uuid,
    pub(crate) started_at: u64,
    pub(crate) maker_payment_wait: u64,
    pub(crate) maker_coin_start_block: u64,
    pub(crate) taker_coin_start_block: u64,
    /// A transaction fee that should be paid to send a `TakerFee`.
    /// Note this value is used to calculate locked amount only.
    pub(crate) fee_to_send_taker_fee: Option<SavedTradeFee>,
    /// A `TakerPayment` transaction fee.
    /// Note this value is used to calculate locked amount only.
    pub(crate) taker_payment_trade_fee: Option<SavedTradeFee>,
    /// A transaction fee that should be paid to spend a `MakerPayment`.
    /// Note this value is used to calculate locked amount only.
    pub(crate) maker_payment_spend_trade_fee: Option<SavedTradeFee>,
    pub(crate) maker_coin_swap_contract_address: Option<BytesJson>,
    pub(crate) taker_coin_swap_contract_address: Option<BytesJson>,
    /// Temporary pubkey used in HTLC redeem script when applicable for maker coin
    pub(crate) maker_coin_htlc_pubkey: Option<H264Json>,
    /// Temporary pubkey used in HTLC redeem script when applicable for taker coin
    pub(crate) taker_coin_htlc_pubkey: Option<H264Json>,
    /// Temporary privkey used to sign P2P messages when applicable
    pub(crate) p2p_privkey: Option<SerializableSecp256k1Keypair>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct MakerNegotiationData {
    pub(crate) maker_payment_locktime: u64,
    pub(crate) maker_pubkey: H264Json,
    pub(crate) secret_hash: BytesJson,
    pub(crate) maker_coin_swap_contract_addr: Option<BytesJson>,
    pub(crate) taker_coin_swap_contract_addr: Option<BytesJson>,
    pub(crate) maker_coin_htlc_pubkey: Option<H264Json>,
    pub(crate) taker_coin_htlc_pubkey: Option<H264Json>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TakerPaymentSpentData {
    pub(crate) transaction: TransactionIdentifier,
    pub(crate) secret: H256Json,
}

#[derive(Deserialize)]
pub(crate) struct MySwapStatusResponse {
    #[serde(flatten)]
    pub(crate) swap: SavedSwap,
    pub(crate) my_info: Option<MySwapInfo>,
    pub(crate) recoverable: bool,
}

/// The helper structure that makes easier to parse the response for GUI devs
/// They won't have to parse the events themselves handling possible errors, index out of bounds etc.
#[derive(Deserialize)]
pub(crate) struct MySwapInfo {
    pub(crate) my_coin: String,
    pub(crate) other_coin: String,
    pub(crate) my_amount: BigDecimal,
    pub(crate) other_amount: BigDecimal,
    pub(crate) started_at: u64,
}

#[skip_serializing_none]
#[derive(Serialize)]
#[serde(tag = "method", rename = "my_recent_swaps")]
pub(crate) struct MyRecentSwapsRequest {
    #[serde(flatten)]
    pub(crate) filter: MySwapsFilter,
    pub(crate) limit: usize,
    pub(crate) from_uuid: Option<Uuid>,
    pub(crate) page_number: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct MyRecentSwapResponse {
    pub(crate) swaps: Vec<SavedSwap>,
    pub(crate) from_uuid: Option<Uuid>,
    pub(crate) skipped: usize,
    pub(crate) limit: usize,
    pub(crate) total: usize,
    pub(crate) page_number: usize,
    pub(crate) total_pages: usize,
    pub(crate) found_records: usize,
}

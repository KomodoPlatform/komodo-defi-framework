use derive_more::Display;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use skip_serializing_none::skip_serializing_none;
use std::collections::HashSet;

use crate::rpc_data::zcoin::{AnyValue, Bip32Child, Bip32PurposeValue, Bip44Tail, HardenedValue};
use common::PagingOptionsEnum;
use mm2_number::BigDecimal;

#[derive(Debug, Serialize)]
#[serde(tag = "method", rename = "send_raw_transaction")]
pub(crate) struct SendRawTransactionRequest {
    pub(crate) coin: String,
    pub(crate) tx_hex: BytesJson,
}

#[derive(Deserialize)]
pub(crate) struct SendRawTransactionResponse {
    pub(crate) tx_hash: BytesJson,
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
#[serde(tag = "method", rename = "withdraw")]
pub(crate) struct WithdrawRequest {
    pub(crate) coin: String,
    pub(crate) from: Option<WithdrawFrom>,
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) amount: BigDecimal,
    #[serde(default)]
    pub(crate) max: bool,
    pub(crate) fee: Option<WithdrawFee>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum WithdrawFrom {
    DerivationPath { derivation_path: String },
    HDWalletAddress(StandardHDCoinAddress),
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StandardHDCoinAddress {
    pub account: u32,
    pub is_change: bool,
    pub address_index: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct HDAccountAddressId {
    pub(crate) account_id: u32,
    pub(crate) chain: Bip44Chain,
    pub(crate) address_id: u32,
}

#[derive(Debug, Display, Deserialize, Serialize)]
#[repr(u32)]
pub(crate) enum Bip44Chain {
    External = 0,
    Internal = 1,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(crate) enum WithdrawFee {
    UtxoFixed { amount: BigDecimal },
    UtxoPerKbyte { amount: BigDecimal },
    EthGas { gas_price: BigDecimal, gas: u64 },
    Qrc20Gas { gas_limit: u64, gas_price: u64 },
    CosmosGas { gas_limit: u64, gas_price: f64 },
}

#[derive(Debug, Deserialize)]
pub(crate) struct WithdrawResponse {
    pub(crate) tx_hex: BytesJson,
    pub(crate) tx_hash: String,
    pub(crate) from: Vec<String>,
    pub(crate) to: Vec<String>,
    pub(crate) total_amount: BigDecimal,
    pub(crate) spent_by_me: BigDecimal,
    pub(crate) received_by_me: BigDecimal,
    pub(crate) my_balance_change: BigDecimal,
    pub(crate) block_height: u64,
    pub(crate) timestamp: u64,
    pub(crate) fee_details: Option<Json>,
    pub(crate) coin: String,
    pub(crate) internal_id: BytesJson,
    pub(crate) kmd_rewards: Option<KmdRewardsDetails>,
    pub(crate) transaction_type: Option<Json>,
    pub(crate) memo: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KmdRewardsDetails {
    pub(crate) amount: BigDecimal,
    pub(crate) claimed_by_me: bool,
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
#[serde(tag = "method", rename = "my_tx_history")]
pub(crate) struct MyTxHistoryRequest {
    pub(crate) coin: String,
    pub(crate) from_id: Option<BytesJson>,
    pub(crate) max: bool,
    pub(crate) limit: usize,
    pub(crate) page_number: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct MyTxHistoryResponse {
    pub(crate) transactions: Vec<MyTxHistoryDetails>,
    pub(crate) limit: usize,
    pub(crate) skipped: usize,
    pub(crate) from_id: Option<BytesJson>,
    pub(crate) total: usize,
    pub(crate) current_block: u64,
    pub(crate) sync_status: HistorySyncState,
    pub(crate) page_number: Option<usize>,
    pub(crate) total_pages: Option<usize>,
}

#[derive(Display, Deserialize)]
#[serde(tag = "state", content = "additional_info")]
pub(crate) enum HistorySyncState {
    NotEnabled,
    NotStarted,
    InProgress(Json),
    Error(Json),
    Finished,
}

#[derive(Deserialize)]
pub(crate) struct MyTxHistoryDetails {
    #[serde(flatten)]
    pub(crate) details: TransactionDetails,
    pub(crate) confirmations: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TransactionDetails {
    pub(crate) tx_hex: BytesJson,
    pub(crate) tx_hash: String,
    pub(crate) from: Vec<String>,
    pub(crate) to: Vec<String>,
    pub(crate) total_amount: BigDecimal,
    pub(crate) spent_by_me: BigDecimal,
    pub(crate) received_by_me: BigDecimal,
    pub(crate) my_balance_change: BigDecimal,
    pub(crate) block_height: u64,
    pub(crate) timestamp: u64,
    pub(crate) fee_details: Option<Json>,
    pub(crate) coin: String,
    pub(crate) internal_id: BytesJson,
    pub(crate) kmd_rewards: Option<KmdRewardsDetails>,
    pub(crate) transaction_type: TransactionType,
    pub(crate) memo: Option<String>,
}

#[derive(Debug, Display, Deserialize)]
pub(crate) enum TransactionType {
    StakingDelegation,
    RemoveDelegation,
    StandardTransfer,
    #[display(fmt = "TokenTransfer: {}", "hex::encode(&_0.0)")]
    TokenTransfer(BytesJson),
    FeeForTokenTx,
    #[display(fmt = "msg_type: {}, token_id: {}", "_0.msg_type", "format_bytes_json(&_0.token_id)")]
    CustomTendermintMsg(CustomTendermintMsg),
    NftTransfer,
}

fn format_bytes_json(bytes: &Option<BytesJson>) -> String {
    bytes
        .as_ref()
        .map(|v| hex::encode(&v.0))
        .unwrap_or_else(|| "none".to_string())
}

#[derive(Debug, Deserialize)]
pub(crate) struct CustomTendermintMsg {
    msg_type: CustomTendermintMsgType,
    token_id: Option<BytesJson>,
}

#[derive(Debug, Display, Deserialize)]
pub(crate) enum CustomTendermintMsgType {
    SendHtlcAmount,
    ClaimHtlcAmount,
    SignClaimHtlc,
}

#[derive(Debug, Serialize)]
pub(crate) struct MyTxHistoryRequestV2<T> {
    pub(crate) coin: String,
    pub(crate) limit: usize,
    pub(crate) paging_options: PagingOptionsEnum<T>,
}

#[derive(Deserialize)]
pub struct MyTxHistoryResponseV2<Tx, Id> {
    pub(crate) coin: String,
    pub(crate) target: MyTxHistoryTarget,
    pub(crate) current_block: u64,
    pub(crate) transactions: Vec<Tx>,
    pub(crate) sync_status: HistorySyncState,
    pub(crate) limit: usize,
    pub(crate) skipped: usize,
    pub(crate) total: usize,
    pub(crate) total_pages: usize,
    pub(crate) paging_options: PagingOptionsEnum<Id>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub(crate) enum MyTxHistoryTarget {
    Iguana,
    #[allow(dead_code)]
    AccountId {
        account_id: u32,
    },
    AddressId(HDAccountAddressId),
    AddressDerivationPath(StandardHDPath),
}

#[rustfmt::skip]
pub(crate) type StandardHDPath =
Bip32Child<Bip32PurposeValue, // `purpose`
Bip32Child<HardenedValue, // `coin_type`
Bip32Child<HardenedValue, // `account_id`
Bip32Child<Bip44ChainValue, // `chain`
Bip32Child<NonHardenedValue, // `address_id`
Bip44Tail>>>>>;

#[derive(Debug, Deserialize)]
pub(crate) struct Bip44ChainValue {
    #[allow(dead_code)]
    chain: Bip44Chain,
}

pub(crate) type NonHardenedValue = AnyValue<false>;

#[derive(Debug, Serialize)]
#[serde(tag = "method", rename = "show_priv_key")]
pub(crate) struct ShowPrivateKeyRequest {
    pub(crate) coin: String,
}

#[derive(Deserialize)]
pub(crate) struct ShowPrivateKeyResponse {
    pub(crate) coin: String,
    pub(crate) priv_key: String,
}

#[derive(Serialize)]
#[serde(tag = "method", rename = "validateaddress")]
pub(crate) struct ValidateAddressRequest {
    pub(crate) coin: String,
    pub(crate) address: String,
}

#[derive(Deserialize)]
pub(crate) struct ValidateAddressResponse {
    pub(crate) is_valid: bool,
    pub(crate) reason: Option<String>,
}

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "kmd_rewards_info")]
pub(crate) struct KmdRewardsInfoRequest {}

pub(crate) type KmdRewardsInfoResponse = Vec<KmdRewardsInfoElement>;

#[derive(Deserialize)]
pub(crate) struct KmdRewardsInfoElement {
    pub(crate) tx_hash: H256Json,
    pub(crate) height: Option<u64>,
    pub(crate) output_index: u32,
    pub(crate) amount: BigDecimal,
    pub(crate) locktime: u64,
    pub(crate) accrued_rewards: KmdRewardsAccrueInfo,
    pub(crate) accrue_start_at: Option<u64>,
    pub(crate) accrue_stop_at: Option<u64>,
}

#[derive(Display, Deserialize)]
pub(crate) enum KmdRewardsAccrueInfo {
    Accrued(BigDecimal),
    NotAccruedReason(KmdRewardsNotAccruedReason),
}

#[derive(Display, Deserialize)]
pub(crate) enum KmdRewardsNotAccruedReason {
    LocktimeNotSet,
    LocktimeLessThanThreshold,
    UtxoHeightGreaterThanEndOfEra,
    UtxoAmountLessThanTen,
    OneHourNotPassedYet,
    TransactionInMempool,
}

#[derive(Deserialize)]
pub(crate) struct ZcoinTxDetails {
    pub(crate) tx_hash: String,
    pub(crate) from: HashSet<String>,
    pub(crate) to: HashSet<String>,
    pub(crate) spent_by_me: BigDecimal,
    pub(crate) received_by_me: BigDecimal,
    pub(crate) my_balance_change: BigDecimal,
    pub(crate) block_height: i64,
    pub(crate) confirmations: i64,
    pub(crate) timestamp: i64,
    pub(crate) transaction_fee: BigDecimal,
    pub(crate) coin: String,
    pub(crate) internal_id: i64,
}

#[derive(Serialize)]
#[serde(tag = "method", rename = "convertaddress")]
pub(crate) struct ConvertAddressRequest {
    pub(crate) coin: String,
    pub(crate) from: String,
    pub(crate) to_address_format: ConvertAddressFormat,
}

#[derive(Serialize)]
#[serde(tag = "format", content = "network", rename_all = "lowercase")]
pub(crate) enum ConvertAddressFormat {
    MixedCase,
    CashAddress(CashAddressNetwork),
    Standard,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CashAddressNetwork {
    BitcoinCash,
    BchTest,
    BchReg,
}

#[derive(Deserialize)]
pub(crate) struct ConvertAddressResponse {
    pub(crate) address: String,
}

#[derive(Serialize)]
#[serde(tag = "method", rename = "convert_utxo_address")]
pub(crate) struct ConvertUtxoAddressRequest {
    pub(crate) address: String,
    pub(crate) to_coin: String,
}

pub(crate) type ConvertUtxoAddressResponse = String;

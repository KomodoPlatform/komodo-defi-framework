use derive_more::Display;
use rpc::v1::types::H256 as H256Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use mm2_rpc::data::legacy::UnbanPubkeysRequest as UnbanPubkeysRequestImpl;

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "list_banned_pubkeys")]
pub(crate) struct ListBannedPubkeysRequest {}

pub(crate) type ListBannedPubkeysResponse = HashMap<H256Json, BanReason>;

#[derive(Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub(crate) enum BanReason {
    Manual { reason: String },
    FailedSwap { caused_by_swap: Uuid },
}

#[derive(Serialize)]
#[serde(tag = "method", rename = "unban_pubkeys")]
pub(crate) struct UnbanPubkeysRequest {
    pub(crate) unban_by: UnbanPubkeysRequestImpl,
}

#[derive(Deserialize)]
pub(crate) struct UnbanPubkeysResponse {
    pub(crate) still_banned: HashMap<H256Json, BanReason>,
    pub(crate) unbanned: HashMap<H256Json, BanReason>,
    pub(crate) were_not_banned: Vec<H256Json>,
}

#[derive(Serialize)]
pub(crate) struct GetCurrentMtpRequest {
    pub(crate) coin: String,
}

#[derive(Deserialize)]
pub(crate) struct GetCurrentMtpResponse {
    pub(crate) mtp: u32,
}

#[derive(Deserialize, Display)]
#[serde(tag = "error_type", content = "error_data")]
pub(crate) enum GetCurrentMtpError {
    NoSuchCoin(String),
    #[display(fmt = "Requested coin: {}; is not supported for this action.", _0)]
    NotSupportedCoin(String),
    RpcError(String),
}

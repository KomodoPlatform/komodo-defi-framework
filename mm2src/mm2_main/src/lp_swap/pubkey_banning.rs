use super::{SwapEvent, SwapsContext};
use chain::hash::H256;
use http::Response;
use mm2_core::mm_ctx::MmArc;
use mm2_rpc::data::legacy::{BanPubkeysRequest, UnbanPubkeysRequest};
use rpc::v1::types::H256 as H256Json;
use serde_json::{self as json, Value as Json};
use std::collections::hash_map::{Entry, HashMap};
use uuid::Uuid;

#[derive(Serialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum BanReason {
    Manual {
        reason: String,
    },
    FailedSwap {
        caused_by_swap: Uuid,
        caused_by_event: SwapEvent,
    },
}

pub fn ban_pubkey_on_failed_swap(ctx: &MmArc, pubkey: H256, swap_uuid: &Uuid, event: SwapEvent) {
    let ctx = SwapsContext::from_ctx(ctx).unwrap();
    let mut banned = ctx.banned_pubkeys.lock().unwrap();
    banned.insert(pubkey.into(), BanReason::FailedSwap {
        caused_by_swap: *swap_uuid,
        caused_by_event: event,
    });
}

pub fn is_pubkey_banned(ctx: &MmArc, pubkey: &H256Json) -> bool {
    let ctx = SwapsContext::from_ctx(ctx).unwrap();
    let banned = ctx.banned_pubkeys.lock().unwrap();
    banned.contains_key(pubkey)
}

pub async fn list_banned_pubkeys_rpc(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    let ctx = try_s!(SwapsContext::from_ctx(&ctx));
    let res = try_s!(json::to_vec(&json!({
        "result": *try_s!(ctx.banned_pubkeys.lock()),
    })));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn ban_pubkey_rpc(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: BanPubkeysRequest = try_s!(json::from_value(req));
    let ctx = try_s!(SwapsContext::from_ctx(&ctx));
    let mut banned_pubs = try_s!(ctx.banned_pubkeys.lock());

    match banned_pubs.entry(req.pubkey) {
        Entry::Occupied(_) => ERR!("Pubkey is banned already"),
        Entry::Vacant(entry) => {
            entry.insert(BanReason::Manual { reason: req.reason });
            let res = try_s!(json::to_vec(&json!({
                "result": "success",
            })));
            Ok(try_s!(Response::builder().body(res)))
        },
    }
}

pub async fn unban_pubkeys_rpc(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: UnbanPubkeysRequest = try_s!(json::from_value(req["unban_by"].clone()));
    let ctx = try_s!(SwapsContext::from_ctx(&ctx));
    let mut banned_pubs = try_s!(ctx.banned_pubkeys.lock());
    let mut unbanned = HashMap::new();
    let mut were_not_banned = vec![];
    match req {
        UnbanPubkeysRequest::All => {
            unbanned = banned_pubs.drain().collect();
        },
        UnbanPubkeysRequest::Few(pubkeys) => {
            for pubkey in pubkeys {
                match banned_pubs.remove(&pubkey) {
                    Some(removed) => {
                        unbanned.insert(pubkey, removed);
                    },
                    None => were_not_banned.push(pubkey),
                }
            }
        },
    }
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "still_banned": *banned_pubs,
            "unbanned": unbanned,
            "were_not_banned": were_not_banned,
        },
    })));
    Ok(try_s!(Response::builder().body(res)))
}

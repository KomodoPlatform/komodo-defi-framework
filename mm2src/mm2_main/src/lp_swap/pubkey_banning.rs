use std::collections::HashMap;

use super::{SwapEvent, SwapsContext};
use chain::hash::H256;
use compatible_time::Duration;
use http::Response;
use mm2_core::mm_ctx::MmArc;
use rpc::v1::types::H256 as H256Json;
use serde_json::{self as json, Value as Json};
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
    // Bun them for an hour.
    const PENALTY: Duration = Duration::from_secs(60 * 60);

    let ctx = SwapsContext::from_ctx(ctx).unwrap();
    let mut banned = ctx.banned_pubkeys.lock().unwrap();
    banned.insert_expirable(
        pubkey.into(),
        BanReason::FailedSwap {
            caused_by_swap: *swap_uuid,
            caused_by_event: event,
        },
        PENALTY,
    );
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

#[derive(Deserialize)]
struct BanPubkeysReq {
    pubkey: H256Json,
    reason: String,
    duration_min: Option<u8>,
}

pub async fn ban_pubkey_rpc(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: BanPubkeysReq = try_s!(json::from_value(req));
    let ctx = try_s!(SwapsContext::from_ctx(&ctx));
    let mut banned_pubs = try_s!(ctx.banned_pubkeys.lock());

    if banned_pubs.contains_key(&req.pubkey) {
        return ERR!("Pubkey is banned already");
    }

    if let Some(duration_min) = req.duration_min {
        banned_pubs.insert_expirable(
            req.pubkey,
            BanReason::Manual { reason: req.reason },
            Duration::from_secs(duration_min as u64 * 60),
        );
    } else {
        banned_pubs.insert_constant(req.pubkey, BanReason::Manual { reason: req.reason });
    }

    let res = try_s!(json::to_vec(&json!({
        "result": "success",
    })));

    Response::builder().body(res).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
enum UnbanPubkeysReq {
    All,
    Few(Vec<H256Json>),
}

pub async fn unban_pubkeys_rpc(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: UnbanPubkeysReq = try_s!(json::from_value(req["unban_by"].clone()));
    let ctx = try_s!(SwapsContext::from_ctx(&ctx));
    let mut banned_pubs = try_s!(ctx.banned_pubkeys.lock());
    let mut were_not_banned = vec![];

    let unbanned = match req {
        UnbanPubkeysReq::All => {
            let unbanned = json!(*banned_pubs);
            banned_pubs.clear();
            unbanned
        },
        UnbanPubkeysReq::Few(pubkeys) => {
            let mut unbanned = HashMap::new();
            for pubkey in pubkeys {
                match banned_pubs.remove(&pubkey) {
                    Some(removed) => {
                        unbanned.insert(pubkey, removed);
                    },
                    None => were_not_banned.push(pubkey),
                }
            }

            json!(unbanned)
        },
    };

    let res = try_s!(json::to_vec(&json!({
        "result": {
            "still_banned": *banned_pubs,
            "unbanned": unbanned,
            "were_not_banned": were_not_banned,
        },
    })));
    Ok(try_s!(Response::builder().body(res)))
}

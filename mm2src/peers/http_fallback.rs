use base64;
use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};
use crc::crc64::checksum_ecma;
use crdts::{CvRDT, CmRDT, Map, Orswot};
use either::Either;
use futures::{future, self, Async, Future};
use gstuff::{netstring, now_float};
use hashbrown::hash_map::{Entry, HashMap, RawEntryMut};
use hyper::{Request, Body};
use hyper::body::Payload;
use hyper::rt::{Stream};
use hyper::service::Service;
use libc::c_void;
use serde_bytes::ByteBuf;
use serde_json::{self as json, Value as Json};
use std::collections::btree_map::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::mem::uninitialized;
use std::net::SocketAddr;
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::str::from_utf8_unchecked;
use tokio_core::net::TcpListener;
use zstd_sys::{ZSTD_CDict, ZSTD_createCDict_byReference, ZSTD_freeCDict, ZSTD_compress_usingCDict_advanced,
    ZSTD_frameParameters, ZSTD_createCCtx, ZSTD_freeCCtx, ZSTD_isError, ZSTD_compressBound,
    ZSTD_createDCtx, ZSTD_freeDCtx, ZSTD_DDict, ZSTD_createDDict, ZSTD_freeDDict, ZSTD_decompress_usingDDict};

use crate::common::{bits256, binprint, rpc_response, slurp_req, HyRes, CORE, HTTP};
use crate::common::mm_ctx::{from_ctx, MmArc, MmWeak};

/// Data belonging to the server side of this module and owned by the MM2 instance.  
/// NB: The client side uses the `hf_*` fields in `PeersContext`.
pub struct HttpFallbackContext {
    /// CRDT maps stored in the HTTP fallback server.  
    /// `BTreeMap` is used for reproducible ordering and prefix search.
    maps: Mutex<BTreeMap<Vec<u8>, RepStrMap>>
}

impl HttpFallbackContext {
    /// Obtains a reference to this mod context, creating it if necessary.
    pub fn from_ctx (ctx: &MmArc) -> Result<Arc<HttpFallbackContext>, String> {
        Ok (try_s! (from_ctx (&ctx.http_fallback_ctx, move || {
            Ok (HttpFallbackContext {
                maps: Mutex::new (BTreeMap::new())
            })
        })))
    }
}

/// Must be the same for the dictionary and the payloads.
/// cf. https://github.com/facebook/zstd/commit/b633377d0e6e2857e2ad2ffaa57f3015b7bc0b8f?short_path=e180ef2#diff-e180ef2189a1472b04a07b61bee5b50b
const COMPRESSION_LEVEL: i32 = 3;
/// "A dictionary can be any arbitrary data segment (also called a prefix)"
const DICT_PREFIX: &'static str = concat! (
    r#","val":{"clock":{"dots":{"1":1}},"entries":{""#,
    r#","deferred":{}"#
);

struct StaticCDict (*mut ZSTD_CDict);
unsafe impl Sync for StaticCDict {}
impl Drop for StaticCDict {
    fn drop (&mut self) {
        // cf. https://github.com/facebook/zstd/blob/a880ca239b447968493dd2fed3850e766d6305cc/contrib/linux-kernel/lib/zstd/compress.c#L2897
        unsafe {ZSTD_freeCDict (self.0)};
        self.0 = null_mut()
}   }

lazy_static! {
    // https://facebook.github.io/zstd/zstd_manual.html#Chapter26
    // cf. https://github.com/facebook/zstd/blob/a880ca239b447968493dd2fed3850e766d6305cc/lib/compress/zstd_compress.c#L3626
    //     https://github.com/facebook/zstd/blob/a880ca239b447968493dd2fed3850e766d6305cc/lib/compress/zstd_compress.c#L3586
    static ref CDICT: StaticCDict = StaticCDict (unsafe {ZSTD_createCDict_byReference (
        DICT_PREFIX.as_ptr() as *const c_void,
        DICT_PREFIX.len(),
        COMPRESSION_LEVEL
    )});
}

struct DDict (*mut ZSTD_DDict);
unsafe impl Sync for DDict {}
impl Drop for DDict {
    fn drop (&mut self) {
        // cf. https://github.com/facebook/zstd/blob/a940e78f1687edf970c75f6b9381de9e0ec493e8/lib/decompress/zstd_ddict.c#L208
        unsafe {ZSTD_freeDDict (self.0)};
        self.0 = null_mut()
}   }

lazy_static! {
    // https://facebook.github.io/zstd/zstd_manual.html#Chapter18
    static ref DDICT: DDict = DDict (unsafe {ZSTD_createDDict (
        DICT_PREFIX.as_ptr() as *const c_void,
        DICT_PREFIX.len()
    )});
}

fn fetch_map_impl (ctx: MmWeak, req: Request<Body>) -> HyRes {
    let f = req.into_body().concat2().then (move |body| -> HyRes {
        let body = try_fus! (body);
        let id = body.to_vec();

        let ctx = try_fus! (MmArc::from_weak (&ctx) .ok_or ("MM stopping"));
        let hfctx = try_fus! (HttpFallbackContext::from_ctx (&ctx));
        let maps = try_fus! (hfctx.maps.lock());
        if let Some (mapʰ) = maps.get (&id) {
            let mapʳ = try_fus! (json::to_string (&mapʰ));
            rpc_response (200, mapʳ)
        } else {
            let map = RepStrMap::new();
            let map = try_fus! (json::to_string (&map));
            rpc_response (200, map)
        }
    });
    Box::new (f)
}

fn fetch_maps_by_prefix_impl (ctx: MmWeak, req: Request<Body>) -> HyRes {
    let f = req.into_body().concat2().then (move |body| -> HyRes {
        let body = try_fus! (body);
        let body = body.to_vec();
        let mut cur = Cursor::new (&body[..]);
        let ver = try_fus! (cur.read_u8());
        if ver != 1 {return Box::new (future::err (ERRL! ("Unknown request version: {}", ver)))}
        let hf_last_poll_id = try_fus! (cur.read_u64::<BigEndian>());
        let mut prefix = Vec::with_capacity (33);
        try_fus! (cur.read_to_end (&mut prefix));
        if prefix.len() < 1 {return Box::new (future::err (ERRL! ("No prefix")))}
//pintln! ("fetch_maps_by_prefix_impl] " [=hf_last_poll_id] ", prefix: " (binprint (&prefix, b'.')));

        let ctx = try_fus! (MmArc::from_weak (&ctx) .ok_or ("MM stopping"));
        let hfctx = try_fus! (HttpFallbackContext::from_ctx (&ctx));
        let maps = try_fus! (hfctx.maps.lock());

        // Prefix search.
        // TODO: Limit the number of entries.
        // TODO: Limit the size of the returned payload.
        // TODO: See if we can make the client prove having rights to the prefix.
        let mut prefixⱼ = prefix.clone();
        let last_byte = &mut prefixⱼ[prefix.len() - 1];
        if *last_byte == u8::max_value() {
            prefixⱼ.push (0)
        } else {
            *last_byte += 1
        };

        let mut buf = Vec::new();
        for (k, map) in maps.range (prefix .. prefixⱼ) {
            try_fus! (write! (&mut buf, "{}:{},", k.len(), unsafe {from_utf8_unchecked (k)}));
            let js = try_fus! (json::to_string (map));
            try_fus! (write! (&mut buf, "{}:{},", js.len(), js));
        }
        let crc = if buf.is_empty() {0} else {checksum_ecma (&buf)};
        // HTTP fallback is not intended for large payloads; if we see some then something is likely wrong.
        if buf.len() > u16::max_value() as usize {return Box::new (future::err (ERRL! ("Payload too big")))}

        // CRDT JSON and base64 have good compression ratios.
        let mut dst: Vec<u8> = Vec::new();
        if !buf.is_empty() {
            dst.reserve (unsafe {ZSTD_compressBound (buf.len())} + 32);
            let cctx = unsafe {ZSTD_createCCtx()};  // TODO: Reuse (we already have a lock).
            assert! (!cctx.is_null());
            let len = unsafe {ZSTD_compress_usingCDict_advanced (cctx,
                dst.as_mut_ptr() as *mut c_void, dst.capacity(),
                buf.as_ptr() as *const c_void, buf.len(),
                CDICT.0,
                ZSTD_frameParameters {contentSizeFlag: 0, checksumFlag: 0, noDictIDFlag: 1}
            )};
            if unsafe {ZSTD_isError (len)} != 0 {return Box::new (future::err (ERRL! ("Can't compress")))}
            unsafe {ZSTD_freeCCtx (cctx)};  // TODO: RAII
            unsafe {dst.set_len (len)};
        }

        buf.clear();
        try_fus! (buf.write_u8 (1));  // Reply protocol version.
        try_fus! (buf.write_u64::<BigEndian> (crc));  // 8 bytes crc, aka `hf_last_poll_id`.
        buf.extend_from_slice (&dst[..]);

        if crc == hf_last_poll_id {
            // TODO: Implement HTTP long polling, returning the reply when it changes or upon a timeout.
            rpc_response (200, "not modified")
        } else {
            rpc_response (200, buf)
        }
    });
    Box::new (f)
}

fn merge_map_impl (ctx: MmWeak, req: Request<Body>) -> HyRes {
    if let Some (cl) = req.body().content_length() {
        // Guard against abuse, HTTP fallback is only intended for small payloads.
        if cl > u16::max_value() as u64 {return rpc_response (500, "Payload too big")}
    }
    let f = req.into_body().concat2().then (move |body| -> HyRes {
        let body = try_fus! (body);
        let buf = body.to_vec();
        if body.len() > u16::max_value() as usize {return rpc_response (500, "Payload too big")}
        let (id, mapˢ) = try_fus! (netstring (&buf));
        let map: RepStrMap = try_fus! (json::from_slice (mapˢ));

        let ctx = try_fus! (MmArc::from_weak (&ctx) .ok_or ("MM stopping"));
        let hfctx = try_fus! (HttpFallbackContext::from_ctx (&ctx));
        let mut maps = try_fus! (hfctx.maps.lock());
        if let Some (mapʰ) = maps.get_mut (id) {
            // NB: Diverging clocks coming from the same actor might lead to an empty map.
            // cf. https://github.com/rust-crdt/rust-crdt/blob/86c7c5601b6b4c4451e1c6840dc1481716ae1433/src/traits.rs#L14

            let actor_id = 1;
            let old_clock = mapʰ.len().add_clock.get (&actor_id);
            let new_clock = map.len().add_clock.get (&actor_id);
            if new_clock <= 2 && 2 < old_clock {
                // ^^ The clocks resets to 1 when when a client restarts.
                //    And to 2 when a client restarts and has two SWAPs with the same peer.
                // TODO: We should add a client instance ID into the merge request
                //       and only allow the rewinds when the client ID differs.
                //       The clock should protect against the out-of-order updates otherwise.
                log! ("merge_map_impl] Clock of " (binprint (id, b'.'))
                      " rewound from " (old_clock) " to " (new_clock));
                maps.insert (id.into(), map);
                rpc_response (200, mapˢ.to_vec())
            } else {
                mapʰ.merge (map);
                let mapʳ = try_fus! (json::to_string (&mapʰ));
                rpc_response (200, mapʳ)
            }
        } else {
            maps.insert (id.into(), map);
            rpc_response (200, mapˢ.to_vec())
        }
    });
    Box::new (f)
}

/// Creates a Hyper Future that would run the HTTP fallback server.
pub fn new_http_fallback (ctx: MmWeak, addr: SocketAddr) -> Result<Box<Future<Item=(), Error=()>+Send>, String> {
    let listener = try_s! (TcpListener::bind2 (&addr));

    struct RpcService {ctx: MmWeak}
    impl Service for RpcService {
        type ReqBody = Body; type ResBody = Body; type Error = String; type Future = HyRes;
        fn call (&mut self, req: Request<Body>) -> HyRes {
            let path = req.uri().path();
            if path == "/fallback/fetch_map" {
                fetch_map_impl (self.ctx.clone(), req)
            } else if path == "/fallback/fetch_maps_by_prefix" {
                fetch_maps_by_prefix_impl (self.ctx.clone(), req)
            } else if path == "/fallback/merge_map" {
                merge_map_impl (self.ctx.clone(), req)
            } else if path == "/test_ip" {  // Helps `fn test_ip` to check the IP availability.
                rpc_response (200, "k")
            } else {
                rpc_response (404, "unknown path")
            }
        }
    }
    let server = listener.incoming().for_each (move |(socket, _my_sock)| {
        let ctx = ctx.clone();
        CORE.spawn (move |_| HTTP
                .serve_connection (socket, RpcService {ctx})
                .map(|_| ())
                .map_err (|err| log! ({"{}", err})));
        Ok(())
    }) .map_err (|err| log! ({"accept error: {}", err}));

    Ok (Box::new (server))
}

/// CRDT clocks are tracked separately for each actor ID.  
/// If two different clients with the same actor ID are trying simultaneously to change a map
/// then one of the change sets might be ignored due to the clock being in the past.
pub type UniqueActorId = u64;

/// CRDT map from string to string.  
/// (The keys and values must be strings in order for the JSON seriazliation to work).
pub type RepStrMap = Map<String, Orswot<String, UniqueActorId>, UniqueActorId>;

/// As of today the replicated map doesn't provide an entries API,
/// but we can get the list of entries from the JSON representation.
pub fn rep_keys (rep_map: &RepStrMap) -> Result<Vec<String>, String> {
    let jsmap = try_s! (json::to_string (rep_map));
    let jsmap: Json = try_s! (json::from_str (&jsmap));
    if let Some (entries) = jsmap["entries"].as_object() {
        Ok (entries.keys().cloned().collect())
    } else {Ok (Vec::new())}
}

fn fallback_url (hf_addr: &SocketAddr, method: &str) -> String {
    fomat! (
        "http" if hf_addr.port() == 443 {'s'} "://"
        (hf_addr.ip())
        if hf_addr.port() != 80 && hf_addr.port() != 443 {':' (hf_addr.port())}
        "/fallback/" (method)
    )
}

/// Fetches a CRDT map stored on the HTTP fallback server.
/// 
/// * `addr` - The address of the HTTP fallback server.
///            The port should be 80 or 443 as this should help the server to function
///            even with the most restrictive internet operators.
pub fn fetch_map (addr: &SocketAddr, id: Vec<u8>) -> Box<Future<Item=RepStrMap, Error=String> + Send> {
    let hf_url = fallback_url (addr, "fetch_map");
    let request = try_fus! (Request::builder()
        .method("POST")
        .uri (hf_url)
        .body (Body::from (id)));
    let f = slurp_req (request);
    let f = f.and_then (|(status, _headers, body)| -> Result<RepStrMap, String> {
        if status.as_u16() != 200 {return ERR! ("fetch_map not 200")}
        let map: RepStrMap = try_s! (json::from_slice (&body));
        Ok (map)
    });
    Box::new (f)
}

/// Merges a CRDT map with the version stored on the HTTP fallback server.
/// 
/// * `addr` - The address of the HTTP fallback server.
///            The port should be 80 or 443 as this should help the server to function
///            even with the most restrictive internet operators.
/// 
/// Returns a fresh version of the map which is provided by the server after the merge.
pub fn merge_map (addr: &SocketAddr, id: Vec<u8>, map: &RepStrMap)
-> Box<Future<Item=RepStrMap, Error=String> + Send> {
    let url = fallback_url (addr, "merge_map");
    let mut map = try_fus! (json::to_vec (map));

    let mut buf = Vec::with_capacity (id.len() + map.len() + 9);
    try_fus! (write! (&mut buf, "{}:{},", id.len(), unsafe {from_utf8_unchecked (&id)}));
    buf.append (&mut map);

    let request = try_fus! (Request::builder()
        .method("POST")
        .uri (url)
        .body (Body::from (buf)));
    let f = slurp_req (request);
    let f = f.and_then (|(status, _headers, body)| -> Result<RepStrMap, String> {
        if status.as_u16() != 200 {return ERR! ("merge_map not 200")}
        let map: RepStrMap = try_s! (json::from_slice (&body));
        Ok (map)
    });
    Box::new (f)
}

/// Several things should be tracked whenever we are using HTTP fallback to reach a target node.
#[derive(Default)]
pub struct HttpFallbackTargetTrack {
    /// Keeping the previous versions of the HTTP fallback maps
    /// in order for the consequent map updates to maintain the total ordering of our clock.
    /// cf. https://github.com/rust-crdt/rust-crdt/blob/86c7c5601b6b4c4451e1c6840dc1481716ae1433/src/traits.rs#L16
    rep_map: RepStrMap,
    /// Time when the latest store operation started.
    pub last_store: f64
}

/// Plugged into `fn transmit` to send the chunks via HTTP fallback when necessary.
pub fn hf_transmit (pctx: &super::PeersContext, hf_addr: &Option<SocketAddr>, our_public_key: &bits256,
                    packages: &mut Vec<super::Package>) -> Result<(), String> {
    let hf_addr = match hf_addr {Some (a) => a, None => return Ok(())};

    let now = now_float();
    let mut cart = HashMap::new();  // Things we want delivered as of now.
    for package in packages.iter_mut() {
        let seed = if let Either::Left ((seed, ref send_handler)) = package.to {
            if send_handler.strong_count() == 0 {continue}
            seed
        } else {
            continue
        };

        let deliver_to_seed = cart.entry (seed) .or_insert (HashMap::new());
        for (payload, meta) in package.payloads.iter_mut() {
            let fallback = match package.fallback {Some (sec) => sec, None => continue};
            if now - package.scheduled_at < fallback.get() as f64 {continue}
            let salt = if let Some (ref salt) = payload.salt {salt.clone()} else {continue};
            if payload.chunk.is_none() {continue}
            deliver_to_seed.insert (salt, (payload, meta));
        }
    }

    for (seed, deliver_to_seed) in cart {
        let mut http_fallback_maps = try_s! (pctx.hf_maps.lock());
        let mut trackⁱ = http_fallback_maps.entry (seed);
        let track = match trackⁱ {
            Entry::Occupied (ref mut oe) => oe.get_mut(),
            Entry::Vacant (ve) => {
                if deliver_to_seed.is_empty() {
                    return Ok(())
                } else {
                    ve.insert (HttpFallbackTargetTrack::default())
                }
            }
        };

        // Synchronize the replicated map with the `deliver_to_seed`.

        let unique_actor_id = 1;
        let mut changed = false;

        let rep_keys = try_s! (rep_keys (&track.rep_map));
        for key in rep_keys {
            let salt = ByteBuf::from (try_s! (base64::decode_config (&key, base64::STANDARD_NO_PAD)));
            if !deliver_to_seed.contains_key (&salt) {
                track.rep_map.apply (track.rep_map.rm (key, track.rep_map.len().derive_rm_ctx()));
                changed = true;
            }
        }

        for (salt, (payload, _meta)) in deliver_to_seed {
            // We're only sending our own packages here.
            assert_eq! (&payload.from[..], unsafe {&our_public_key.bytes[..]});

            let saltᵊ = base64::encode_config (&salt, base64::STANDARD_NO_PAD);
            let chunk = if let Some (ref chunk) = payload.chunk {
                base64::encode_config (chunk, base64::STANDARD_NO_PAD)
            } else {continue};
            let has_chunk = match track.rep_map.get (&saltᵊ) .val.map (|v| v.read().val) {
                None => false,
                Some (set) => set.contains (&chunk)
            };

            if !has_chunk {
                track.rep_map.apply (
                    track.rep_map.update (
                        saltᵊ,
                        track.rep_map.len().derive_add_ctx (unique_actor_id),
                        |set, ctx| set.add (chunk, ctx)
                    )
                );
                changed = true
            }
        }

        // We should keep storing the map even if we have stored it before, to account for server restarts.
        let now = now_float();
        let refresh = if track.last_store > 0. {now - track.last_store > 10.} else {false};
        if !changed && !refresh {return Ok(())}

    log! ("transmit] TBD, time to use the HTTP fallback...");

        let mut hf_id = Vec::with_capacity (unsafe {seed.bytes.len() + 1 + our_public_key.bytes.len()});
        hf_id.extend_from_slice (unsafe {&seed.bytes});
        hf_id.push (b'<');
        hf_id.extend_from_slice (unsafe {&our_public_key.bytes});
        let merge_f = merge_map (&hf_addr, hf_id, &track.rep_map);
        let merge_f = merge_f.then (|r| -> Result<(), ()> {
            let _merged_rep_map = match r {
                Ok (r) => r,
                Err (err) => {log! ("manage_http_fallback] merge_map error: " (err)); return Err(())}
            };
            Ok(())
        });
        CORE.spawn (|_| merge_f);
        track.last_store = now;
    }

    Ok(())
}

/// Invoked when a delayed retrieval is detected by the peers loop.
/// 
/// * `salt` - The subject salt (checksum of the `subject` passed to `fn recv`).
pub fn hf_delayed_get (pctx: &super::PeersContext, salt: &Vec<u8>) {
    let mut delayed_salts = match pctx.hf_delayed_salts.lock() {
        Ok (set) => set,
        Err (err) => {log! ("Can't lock `delayed_salts`: " (err)); return}
    };
    if let RawEntryMut::Vacant (ve) = delayed_salts.raw_entry_mut().from_key (salt) {
        ve.insert (salt.clone(), ());
    }
}

use hyper::{HeaderMap, StatusCode};

/// Process the prefix search results obtained
/// when we query the HTTP fallback server for maps addressed to our public key.
fn process_pulled_maps (pctx: &Arc<super::PeersContext>, status: StatusCode, _headers: HeaderMap, body: Vec<u8>)
-> Result<(), String> {
    if !status.is_success() {return ERR! ("HTTP status {}", status)}
    if body == &b"not modified"[..] {return Ok(())}

    let mut cur = Cursor::new (&body);
    let ver = try_s! (cur.read_u8());
    if ver != 1 {return ERR! ("Unknown protocol version: {}", ver)}
    let crc = try_s! (cur.read_u64::<BigEndian>());

    let compressed = &body[9..];
    // NB: We know the payload will not be bigger than this.
    let mut buf: [u8; 65536] = unsafe {uninitialized()};
    let dctx = unsafe {ZSTD_createDCtx()};  // TODO: Reuse a locked one.
    let len = unsafe {ZSTD_decompress_usingDDict (
        dctx,
        buf.as_mut_ptr() as *mut c_void, buf.len(),
        compressed.as_ptr() as *const c_void, compressed.len(),
        DDICT.0
    )};
    unsafe {ZSTD_freeDCtx (dctx)};
    if unsafe {ZSTD_isError (len)} != 0 {return ERR! ("Can't decompress")}

    let our_public_key = try_s! (pctx.our_public_key.lock()) .clone();
    let mut chunks = BTreeMap::new();

    let mut tail = &buf[0..len];
    while !tail.is_empty() {
        let (hf_id, tailⁱ) = try_s! (netstring (tail));
        let (rep_map, tailⱼ) = try_s! (netstring (tailⁱ));

        if hf_id.len() != 65 || hf_id[32] != b'<' {return ERR! ("Bad hf_id: {}", binprint (hf_id, b'.'))}
        let to = &hf_id[0..32];
        if unsafe {to != &our_public_key.bytes[..]} {return ERR! ("Bad to: {}", binprint (to, b'.'))}
        let from = &hf_id[33..];  // The public key of the sender.
        let from = bits256 {bytes: *array_ref! (from, 0, 32)};
        // TODO: See if we can verify that payload is coming from `from`.

        let rep_map: RepStrMap = try_s! (json::from_slice (rep_map));
        for salt in try_s! (rep_keys (&rep_map)) {
            let set = try_s! (rep_map.get (&salt) .val.map (|v| v.read().val) .ok_or ("A key with no value"));
            if set.len() != 1 {return ERR! ("Value set of length {}", set.len())}
            let chunk = unwrap! (set.into_iter().next());

            let salt = try_s! (base64::decode_config (&salt, base64::STANDARD_NO_PAD));
            let chunk = try_s! (base64::decode_config (&chunk, base64::STANDARD_NO_PAD));
            chunks.insert (salt, (from, chunk));
        }

        tail = tailⱼ
    }

    {
        let mut hf_inbox = try_s! (pctx.hf_inbox.lock());
        *hf_inbox = chunks;
        pctx.hf_last_poll_id.store (crc, Ordering::Relaxed);
    }
    Ok(())
}

/// Manage HTTP fallback retrievals.  
/// Invoked periodically from the peers loop.
pub fn hf_poll (pctx: &Arc<super::PeersContext>, hf_addr: &Option<SocketAddr>) -> Result<(), String> {
    let hf_addr = match hf_addr {Some (ref a) => a, None => return Ok(())};

    {
        let delayed_salts = try_s! (pctx.hf_delayed_salts.lock());
        if delayed_salts.is_empty() {return Ok(())}
    }

    let now = now_float();
    if now < pctx.hf_skip_poll_till.load (Ordering::Relaxed) as f64 {return Ok(())}

    let mut hf_pollₒ = try_s! (pctx.hf_poll.lock());
    // NB: Futures can only be polled from other futures, see https://stackoverflow.com/a/41813881.
    let skip = try_s! (future::lazy (|| -> Result<bool, String> {
        if let Some (ref mut hf_poll) = *hf_pollₒ {
            match hf_poll.poll() {
                Err (err) => {
                    log! ("hf_poll error: " (err));
                    pctx.hf_skip_poll_till.store ((now + 10.) as u64, Ordering::Relaxed);
                    *hf_pollₒ = None;
                    return Ok (true)
                },
                Ok (Async::NotReady) => {
                    // Retrieval already in progress.
                    return Ok (true)
                },
                Ok (Async::Ready ((status, headers, body))) => {
                    let rc = process_pulled_maps (pctx, status, headers, body);
                    // Should reduce the pause when HTTP long polling is implemented server-side.
                    let pause = if rc.is_ok() {7.} else {10.};
                    pctx.hf_skip_poll_till.store ((now + pause) as u64, Ordering::Relaxed);
                    try_s! (rc);
                    *hf_pollₒ = None;
                    return Ok (true)
                }
            }
        }
        Ok (false)
    }) .wait());
    if skip {return Ok(())}

//pintln! ("hf_poll] polling, at id " (pctx.hf_last_poll_id.load (Ordering::Relaxed)));

    let mut hf_id_prefix = Vec::with_capacity (1 + 4 + 32 + 1);
    hf_id_prefix.push (1);  // Version of the query protocol.
    try_s! (hf_id_prefix.write_u64::<BigEndian> (pctx.hf_last_poll_id.load (Ordering::Relaxed)));
    hf_id_prefix.extend_from_slice (&unsafe {try_s! (pctx.our_public_key.lock()) .bytes} [..]);
    hf_id_prefix.push (b'<');

    let hf_url = fallback_url (hf_addr, "fetch_maps_by_prefix");
    let request = try_s! (Request::builder()
        .method("POST")
        .uri (hf_url)
        .body (Body::from (hf_id_prefix)));
    *hf_pollₒ = Some (slurp_req (request));

    Ok(())
}

/// Invoked when the client terminates a retrieval attempt.
/// 
/// * `salt` - The subject salt (checksum of the `subject` passed to `fn recv`).
pub fn hf_drop_get (pctx: &super::PeersContext, salt: &Vec<u8>) {
    let mut delayed_salts = match pctx.hf_delayed_salts.lock() {
        Ok (set) => set,
        Err (err) => {log! ("Can't lock `delayed_salts`: " (err)); return}
    };
    delayed_salts.remove (salt);
}

// Environment variables:
// LOG_UNHANDLED_ALERTS=true, runtime.
// MM2_INCOMING_PING=f, build time, ignore direct UDP communication.
// MM2_DHT_GET=f, build time, disable DHT retrieval.
// MM2_FALLBACK=$, build time, override fallback timeouts.

#![feature (non_ascii_idents, vec_resize_default, ip, weak_counts, async_await, await_macro)]
#![cfg_attr(not(feature = "native"), allow(dead_code))]
#![cfg_attr(not(feature = "native"), allow(unused_variables))]
#![cfg_attr(not(feature = "native"), allow(unused_macros))]
#![cfg_attr(not(feature = "native"), allow(unused_imports))]

#[macro_use] extern crate arrayref;
#[macro_use] extern crate common;
#[macro_use] extern crate fomat_macros;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate unwrap;

#[cfg(feature = "native")]
mod executor {}

#[cfg(not(feature = "native"))]
pub mod executor;

#[doc(hidden)]
pub mod peers_tests;

#[cfg(feature = "native")]
pub mod http_fallback;
#[cfg(feature = "native")]
use crate::http_fallback::HttpFallbackTargetTrack;
#[cfg(not(feature = "native"))]
pub struct HttpFallbackTargetTrack;
#[cfg(feature = "native")]
use crate::http_fallback::{hf_delayed_get, hf_drop_get, hf_poll, hf_transmit};

use atomic::Atomic;
use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};
use common::{bits256, SlurpFut};
#[cfg(feature = "native")]
use common::{binprint, slice_to_malloc, is_a_test_drill, RaiiRm};
use common::log::TagParam;
use common::mm_ctx::{from_ctx, MmArc};
use crc::crc32::{update, IEEE_TABLE};
use crc::crc64::checksum_ecma;
use crossbeam::channel;
use either::Either;
use futures::{future, Async, Future, Poll};
use futures::task::Task;
use gstuff::{now_float, slurp};
use hashbrown::hash_map::{DefaultHashBuilder, Entry, HashMap, OccupiedEntry, RawEntryMut};
use itertools::Itertools;
#[cfg(feature = "native")]
use libc::{c_char, c_void};
use rand::{thread_rng, Rng, RngCore};
use serde::Serialize;
use serde_bencode::ser::to_bytes as bencode;
use serde_bencode::de::from_bytes as bdecode;
use serde_bytes::ByteBuf;
use serde_json::{self as json};
use std::collections::btree_map::BTreeMap;
use std::cmp::Ordering;
use std::env::{temp_dir, var};
use std::fs;
use std::ffi::{CStr, CString};
use std::fmt::Write as FmtWrite;
use std::io::{Cursor, Write};
use std::iter::{once, FromIterator};
use std::mem::{uninitialized, zeroed};
use std::net::{IpAddr, SocketAddr};
use std::num::{NonZeroU8, NonZeroU64};
use std::path::Path;
use std::ptr::{null, null_mut, read_volatile};
use std::slice::from_raw_parts;
use std::str::from_utf8_unchecked;
use std::sync::{Arc, Mutex, Weak};
use std::sync::atomic::{Ordering as AtomicOrdering};
use std::thread;
use std::time::Duration;

lazy_static! {
    /// Any unprocessed libtorrent alers are logged if this knob is set to "true".
    static ref LOG_UNHANDLED_ALERTS: bool = match var ("LOG_UNHANDLED_ALERTS") {
        Ok (ev) => ev == "true",
        Err (_) => false
    };
}

// NB: C++ structures and functions are defined in "dht.cc".

#[cfg(feature = "native")]
#[repr(C)]
struct dugout_t {
    session: *mut c_void,  // `lt::session*` (from C++ `new`).
    err: *const c_char  // `strdup` of a C++ exception `what`.
}
#[cfg(feature = "native")]
impl dugout_t {
    fn take_err (&mut self) -> Option<String> {
        if !self.err.is_null() {unsafe {
            let what = (if let Ok (msg) = CStr::from_ptr (self.err) .to_str() {msg} else {"Non-unicode `what`"}) .to_owned();
            libc::free (self.err as *mut c_void);
            self.err = null();
            Some (what)
        }} else {
            None
        }
    }
}
#[cfg(feature = "native")]
impl Drop for dugout_t {
    fn drop (&mut self) {
        let err = unsafe {delete_dugout (self)};
        if !err.is_null() {
            let what = unwrap! (unsafe {CStr::from_ptr (err)} .to_str());
            log! ("delete_dugout error: " (what));
            unsafe {libc::free (err as *mut c_void);}
        }
    }
}

enum Alert {}

#[cfg(feature = "native")]
extern "C" {
    fn delete_dugout (dugout: *mut dugout_t) -> *const c_char;
    fn dht_init (listen_interfaces: *const c_char, read_only: bool) -> dugout_t;
    fn dht_load_state (dugout: *mut dugout_t, dht_state: *const u8, dht_state_len: i32);
    fn enable_dht (dugout: *mut dugout_t);
    fn dht_save_state (dugout: *mut dugout_t, buflen: *mut i32) -> *mut u8;
    fn dht_alerts (dugout: *mut dugout_t, cb: extern fn (dugout: *mut dugout_t, cbctx: *mut c_void, alert: *mut Alert), cbctx: *mut c_void);
    fn alert_message (alert: *const Alert) -> *const c_char;
    fn is_dht_bootstrap_alert (alert: *const Alert) -> bool;
    fn as_listen_succeeded_alert (alert: *const Alert) -> *const c_char;
    fn as_listen_failed_alert (alert: *const Alert) -> *const c_char;
    fn as_dht_mutable_item_alert (alert: *const Alert,
        pkbuf: *mut u8, pkbuflen: i32,
        saltbuf: *mut c_char, saltbuflen: i32,
        buf: *mut u8, buflen: i32,
        seq: *mut i64, auth: *mut bool) -> i32;
    fn as_external_ip_alert (alert: *const Alert, ipbuf: *mut u8, ipbuflen: *mut i32) -> i32;
    fn as_dht_pkt_alert (alert: *const Alert,
        buf: *mut u8, buflen: i32,
        direction: *mut i8,
        ipbuf: *mut u8, ipbuflen: *mut i32,
        port: *mut u16) -> i32;
    // * `key` - The 32-byte seed which is given to `ed25519_create_keypair` in order to generate the key pair.
    //           The public key of that pair is also a pointer into the DHT space: nodes closest to it will be asked to store the value.
    // * `keylen` - The length of the `key` in bytes. Must be 32 bytes, no more no less.
    // * `salt` - Essentially a second part of the key identifying the value.
    //            Using a differnt `salt` means storing the value on a different set of DHT nodes.
    // * `saltlen` - The length of the `salt` in bytes. 0 if not used.
    // * `callback` - Invoked from inside the libtorrent code, after the latter obtains the previous (existing) value from the DHT.
    // * `arg` - A pointer passed to the `callback`.
    // 
    // Callback arguments are:
    // 
    // * `arg` - A pointer passed to the `callback` via the `dht_put`.
    //           Usually points to a `Rust` struct, allowing the callback to communicate with the rest of the program.
    // * `arg2` - An integer passed to the `callback` via the `dht_put`.
    //            Might be used to verify the `arg`, for example.
    // * `have` - Bencoded value that was already present under the `key`.
    // * `havelen` - The length of `have` in bytes.
    // * `benload` - Bencoded value that should be saved under the `key`.
    //               In order to pass the value the callback must allocate the memory from `libc` (the C code will later `free` it).
    // * `benlen` - The length of `benload` in bytes.
    // * `seq` - The current version of the value (fetched together with `have`). We must increment it.
    //           DHT nodes will not accept a new value if our version is smaller than theirs.
    fn dht_put (dugout: *mut dugout_t,
                key: *const u8, keylen: i32,
                salt: *const u8, saltlen: i32,
                callback: extern fn (*mut c_void, u64, *const u8, i32, *mut *mut u8, *mut i32, *mut i64), arg: *const c_void, arg2: u64);
    // * `key` - The 32-byte seed which is given to `ed25519_create_keypair` in order to generate the key pair.
    //           The public key of that pair is also a pointer into the DHT space: nodes closest to it will be asked to store the value.
    // * `salt` - Essentially a second part of the key identifying the value.
    //            Using a differnt `salt` means storing the value on a different set of DHT nodes.
    // * `pkbuf` - The public key derived from the `key` seed.
    //             If not zero, it is reused, skipping the `ed25519_create_keypair`.
    //             If zero, receives the generated public key.
    //             This public key identifies the entries obtained via the `dht_mutable_item_alert` (because DHT storage nodes don't know our seed).
    // * `pkbuflen` - Must be 32 bytes. Passed explicitly in order for us to check it.
    fn dht_get (dugout: *mut dugout_t, key: *const u8, keylen: i32, salt: *const u8, saltlen: i32, pkbuf: *mut u8, pkbuflen: i32);
    fn lt_send_udp (dugout: *mut dugout_t, ip: *const u8, port: u16, benload: *const u8, benlen: i32);
}

// https://stackoverflow.com/a/50278316/257568
fn format_radix (mut x: u64, radix: u64) -> Vec<u8> {
    let mut result = Vec::new();

    loop {
        let m = x % radix;
        x = x / radix;

        // will panic if you use a bad radix (< 2 or > 36).
        result.push (unwrap! (std::char::from_digit (m as u32, radix as u32)) as u8);
        if x == 0 {break}
    }
    result.reverse();
    result
}

// TODO: Maybe use an array instead.
pub type Salt = Vec<u8>;

/// A command delivered to the `peers_thread` via the `PeersContext::cmd_tx`.
enum LtCommand {
    Put {
        /// The 32-byte seed which is given to `ed25519_create_keypair` in order to generate the key pair.
        /// The public key of the pair is also a pointer into the DHT space: nodes closest to it will be asked to store the value.
        seed: bits256,
        /// Identifies the value without affecting its DHT location (need to double-check this). Can be empty.  
        /// Should not be too large (BEP 44 mentions error code 207 "salt too big").  
        /// Must not contain zero bytes (we're passing it as a zero-terminated string sometimes).  
        /// NB: If the `data` is large then `peers_thread` will append chunk number to `salt` for every extra DHT chunk.
        salt: Salt,
        payload: Vec<u8>,
        send_handler: Weak<SendHandler>,
        /// Send the value to the HTTP fallback server
        /// if the delivery isn't terminated after the given number of seconds.
        fallback: NonZeroU8
    },
    /// Starts a new get operation, unless it is already in progress.
    Get {
        seed: bits256,
        salt: Salt,
        /// Identifies the `Future` responsible for this get operation.
        frid: NonZeroU64,
        /// The `Future` to wake when the payload is reassembled.
        task: Task,
        /// Start checking the HTTP fallback server for the value
        /// if it doesn't come through other channels in the given number of seconds.
        fallback: NonZeroU8
    },
    /// Stops a get operation when a corresponding `Future` handler is dropped.
    DropGet {
        salt: Salt,
        /// Identifies the `Future` responsible for this get operation.
        frid: NonZeroU64
    },
    /// Direct communication. Sends a DHT ping packet to a given endpoint.
    Ping {
        endpoint: SocketAddr
    }
}

/// A friend is a MM peer we're communicating with.  
/// We track their endpoints and try to discover them via the DHT.
#[derive(Debug, Default)]
struct Friend {
    /// The outer DHT IPs and ports of the friend peer which are known to us.
    endpoints: HashMap<SocketAddr, ()>
}

/// Internal structures used by the retransmission subsystem.
#[derive(Default)]
struct TransMeta {
    /// Tracks the endpons of the peers we're directly communicating with.
    friends: HashMap<bits256, Friend>,
    /// Groups of direct ping packets scheduled for delivery.
    packages: Vec<Package>,
    /// The monotonically incremented part of the outgoing ping packet ID.
    id_seq: u16
}
impl TransMeta {
    /// Borrow both `friends` and `packages` with separate mutable lifetimes.
    fn split_borrow<'a> (&'a mut self) -> (&'a mut HashMap<bits256, Friend>, &'a mut Vec<Package>) {
        // cf. https://github.com/rust-lang/rfcs/issues/1215 ?
        let friends: *mut HashMap<bits256, Friend> = &mut self.friends;
        let packages: *mut Vec<Package> = &mut self.packages;
        unsafe {(&mut *friends, &mut *packages)}
    }
}

/// The peer-to-peer and connectivity information local to the MM2 instance.
pub struct PeersContext {
    our_public_key: Mutex<bits256>,
    peers_thread: Mutex<Option<thread::JoinHandle<()>>>,
    cmd_tx: channel::Sender<LtCommand>,
    /// Should only be used by the `peers_thread`.
    cmd_rx: channel::Receiver<LtCommand>,
    /// Salt -> last-modified, value.
    recently_fetched: Mutex<HashMap<Salt, (f64, Vec<u8>)>>,
    /// Of retransmission subsystem.
    trans_meta: Mutex<TransMeta>,
    /// The number of enhanced DHT pings received from MMs.
    direct_pings: Atomic<u64>,
    /// The number of data chunks delivered directly via the DHT pings.  
    /// (direct_pings - discovery pings - pongs - invalid).
    direct_chunks: Atomic<u64>,
    /// Recent attempts at reaching targets via HTTP fallback. seed -> track
    hf_maps: Mutex<HashMap<bits256, HttpFallbackTargetTrack>>,
    /// Subject salts we're trying to get for longer than their `fallback` timeout.
    hf_delayed_salts: Mutex<HashMap<Salt, ()>>,
    /// Long polling from HTTP fallback server.
    hf_poll: Mutex<Option<SlurpFut>>,
    /// The version of the HTTP fallback response that we already have.
    hf_last_poll_id: Atomic<u64>,
    /// Time (in seconds since UNIX epoch) util which we should skip polling the HTTP fallback server.
    hf_skip_poll_till: Atomic<u64>,
    /// Snapshot of chunks received through the HTTP fallback server.  
    /// Using `BTreeMap` in order for `hf_to_gets` to get the first chunk (with the number of chunks) first.
    hf_inbox: Mutex<BTreeMap<Salt, (bits256, Vec<u8>)>>
}

impl PeersContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx (ctx: &MmArc) -> Result<Arc<PeersContext>, String> {
        Ok (try_s! (from_ctx (&ctx.peers_ctx, move || {
            let (cmd_tx, cmd_rx) = channel::unbounded::<LtCommand>();
            Ok (PeersContext {
                our_public_key: Mutex::new (unsafe {zeroed()}),
                peers_thread: Mutex::new (None),
                cmd_tx,
                cmd_rx,
                recently_fetched: Mutex::new (HashMap::new()),
                trans_meta: Mutex::new (TransMeta::default()),
                direct_pings: Atomic::new (0u64),
                direct_chunks: Atomic::new (0u64),
                hf_maps: Mutex::new (HashMap::new()),
                hf_delayed_salts: Mutex::new (HashMap::new()),
                hf_poll: Mutex::new (None),
                hf_last_poll_id: Atomic::new (0u64),
                hf_skip_poll_till: Atomic::new (0u64),
                hf_inbox: Mutex::new (BTreeMap::new())
            })
        })))
    }
}

/// Data passed through the C code and into the callback during the put operation.
struct PutShuttle {
    // NB: Looks like it can invoked multiple times by libtorrent.
    put_handler: Box<dyn Fn (&[u8]) -> Result<Vec<u8>, String> + 'static + Send + Sync>
}

lazy_static! {
    /// A buffer of the `PutShuttle` structures which are shared with the `callback` executed from the libtorrent and "dht.cc" code.
    /// We don't know when libtorrent will stop using the `put_handler`.
    /// Probably after the corresponding put alert, but we aren't catching one yet.
    /// So we have to keep the shuttles around for a while.
    static ref PUT_SHUTTLES: Mutex<HashMap<usize, (u64, Arc<PutShuttle>)>> = Mutex::new (HashMap::default());
    /// seed -> lm, ops
    static ref RATELIM: Mutex<HashMap<bits256, (f64, f32)>> = Mutex::new (HashMap::default());
}

fn with_ratelim<F> (seed: &bits256, cb: F) where F: FnOnce (&mut f64, &mut f32) {
    if let Ok (mut ratelim) = RATELIM.lock() {
        let mut lim_entry = ratelim.entry (*seed) .or_default();
        if lim_entry.0 == 0. {lim_entry.0 = now_float() - 0.01}
        cb (&mut lim_entry.0, &mut lim_entry.1)
    } else {log! ("Can't lock RATELIM")}
}

/// Burn away ~10 ops per second.
fn ratelim_maintenance (seed: &bits256) -> f32 {
    let mut limops = 0f32;
    with_ratelim (seed, |lm, ops| {
        *ops = 0f32 .max (*ops - (now_float() - *lm) as f32 * 10.);
        limops = *ops;
        *lm = now_float();
    });
    limops
}

macro_rules! s2b {($s: expr) => {ByteBuf::from ($s.as_bytes())};}

// TODO: Consider directly embedding `MmPayload` without generics.

// We're sending normal http://www.bittorrent.org/beps/bep_0005.html pings, only with extra `en["a"]` arguments.
// That way if something would happen with the delivery of the MM packets via `dht_direct_request`
// then the problem will be a subset of a generic ping delivery problem.
// NB: libtorrent automatically adds a random `en["t"]`.
// 1:ad2:id20:.....DG1.v.'...y..h.2:mmd4:from32:..HO).S.h_?.:....z.x5^as..XoKZ.j4:pongi0eee1:q4:ping2:roi1e1:t2:.j1:v4:LT..1:y1:qe
#[derive (Serialize, Deserialize)]
struct Ping<P> {
    y: ByteBuf,
    q: ByteBuf,
    a: PingArgs<P>
}
// NB: libtorrent automatically adds a proper `{"a" {"id": …}}` to the ping.
#[derive (Serialize, Deserialize)]
struct PingArgs<P> {
    mm: P
}

/// Tells libtorrent to send a ping DHT packet with extra payload.
/// 
/// Should preferably be run from under the `peers_thread`
/// in order to minimize the chance of synchronization issues in the unsafe C code.
/// 
/// DHT packets larger than 1500 bytes are dropped. Experimenting with payload size shows
/// that the overhead is around 81 bytes. Thus the recommended maximum size of the `extra_payload`
/// is about 1333-1400 bytes.
/// The function will return an error if the `extra_payload` is larger than this.
/// Note that we don't immediately know the actual size of the outgoing packet
/// because the final encoding is performed by the libtorrent.
/// 
/// A `dht_direct_response` alert is usually fired when we get a confirmation of the packet delivery,
/// but there seems to be a considerable time span (~15 seconds) between the time when the packet is received
/// and the time we get the delivery alert.
/// 
/// * `endpoint` - The open (and possibly hole-punched) address of the peer.
/// * `extra_payload` - Carries the extra information in the "mm" ping argument.
#[cfg(feature = "native")]
fn ping<P> (dugout: &mut dugout_t, endpoint: &SocketAddr, extra_payload: P) -> Result<(), String>
where P: Serialize {
    let ping = Ping {
        y: s2b! ("q"),  // It's a query.
        q: s2b! ("ping"),  // It's a DHT ping query.
        a: PingArgs {
            mm: extra_payload
        }
    };

    let mut ipz = String::with_capacity (64);
    let _ = wite! (&mut ipz, (endpoint.ip()) '\0');
    let benload = try_s! (bencode (&ping));
    let extra_payload_size = if benload.len() > 26 {benload.len() - 26} else {0};
    if extra_payload_size > 1400 {return ERR! ("`extra_payload` is too large")}

    unsafe {lt_send_udp (dugout, ipz.as_ptr(), endpoint.port(), benload.as_ptr(), benload.len() as i32)};
    if let Some (err) = dugout.take_err() {return ERR! ("lt_send_udp error: {}", err)}
    Ok(())
}

#[derive (Clone, Deserialize, Serialize, Debug)]
struct MmPayload {
    /// 64-bit ID of the payload.  
    /// Pong packets have the same ID as the packets they're echoing.
    id: ByteBuf,
    /// The public key ID (`LP_mypub25519`) of the sender.
    from: ByteBuf,
    /// 1 if this payload is a confirmation we emit for a previously received ping.  
    /// NB: The pongs aren't confirmed with more pongs.
    pong: u8,
    /// Identifies a chunk of a payload going from `send` to `recv`.  
    /// The chunk salt is the subject salt + one-byte index.
    salt: Option<ByteBuf>,
    /// A piece of data going from `send` to `recv`.
    chunk: Option<ByteBuf>
}
impl MmPayload {
    fn next_id (trans: &mut TransMeta) -> ByteBuf {
        trans.id_seq = trans.id_seq.wrapping_add (1);
        let id = ((trans.id_seq as u64) << 48) | (thread_rng().next_u64() & 0xFFFFFFFFFFFF);
        let mut id_buf: [u8; 8] = unsafe {zeroed()};
        let mut cur = Cursor::new (&mut id_buf[..]);
        unwrap! (cur.write_u64::<BigEndian> (id));
        ByteBuf::from (&id_buf[..])
    }
}

/// What we on our side know about an outgoing payload.
#[derive(Default, Debug)]
struct PayloadOutMeta {
    /// The time we've asked libtorrent to save this chunk in the DHT.
    dht_put_invoked: f64,
    /// The number of times we scheduled direct UDP delivery of the chunk with libtorrent.
    pings: u8,
    /// The number of pong packets we've got confirming direct transfer.
    pongs: u8
}

/// A group of ping packets (one or more) that we want to deliver.  
/// Cancellable: transmission is stopped when the `Arc<SendHandler>` returned by `send` is dropped.  
/// The target is either a specific endpoint (when we're trying to discover a peer)
/// or a friend's public key (when we're `send`ing data to that friend).
pub struct Package {
    payloads: Vec<(MmPayload, PayloadOutMeta)>,
    to: Either<(bits256, Weak<SendHandler>), SocketAddr>,
    /// Time (in seconds since UNIX epoch) when the package was scheduled.
    scheduled_at: f64,
    /// The number of seconds after which we should start sharing the data via the HTTP fallback.
    fallback: Option<NonZeroU8>
}

#[cfg(feature = "native")]
extern fn put_callback (arg: *mut c_void, arg2: u64, have: *const u8, havelen: i32, benload: *mut *mut u8, benlen: *mut i32, seq: *mut i64) {
    assert! (!arg.is_null());
    assert! (!have.is_null());
    assert! (!benload.is_null());
    assert! (!benlen.is_null());
    assert! (!seq.is_null());
    //pintln! ("peers_send_compat] callback] " [=arg] ' ' [=have] ' ' [=havelen] ' ' [=benload] ' ' [=benlen] " seq " (unsafe {*seq}));
    let shuttles = unwrap! (PUT_SHUTTLES.lock());
    let shuttle = match shuttles.get (&(arg as usize)) {
        Some ((created, shuttle)) if *created == arg2 => shuttle,
        _ => panic! ("No such shuttle: {:?}", arg)
    };
    let have = unsafe {from_raw_parts (have, havelen as usize)};
    match (shuttle.put_handler) (have) {
        Ok (new_load) => unsafe {
            *benload = slice_to_malloc (&new_load);
            *benlen = new_load.len() as i32;
            *seq += 1
        },
        Err (err) => unsafe {
            log! ("put_handler error: " (err));
            // Keeping the previous value is probably the least invasive in that it doesn't affect the value parsers.
            *benload = slice_to_malloc (have);
            *benlen = have.len() as i32
        }
    }
}

/// Invoked periodically from the `peers_thread` in order to manage and retransmit the outgoing ping packets.
#[cfg(feature = "native")]
fn transmit (dugout: &mut dugout_t, ctx: &MmArc, hf_addr: &Option<SocketAddr>) -> Result<(), String> {
    // AG: Instead of the current random RATELIM skipping
    //     we might consider *ordering* the packets according to the endpoint freshness, etc.
    //     For `dht_put` the ordering might take into account the presense or absence of the corresponding `dht_put` callback
    //     (we want to repeat a `dht_put` more often, relative to others, if we haven't heard from it).

    let pctx = try_s! (PeersContext::from_ctx (ctx));
    let our_public_key = try_s! (pctx.our_public_key.lock()) .clone();
    let mut trans = try_s! (pctx.trans_meta.lock());
    let (friends, packages) = trans.split_borrow();
    for pix in (0 .. packages.len()) .rev() {
        let mut finished = Vec::new();
        let package = &mut packages[pix];
        match package.to {
            // Recepient is an IP and port, directly reachable or tested to be.
            Either::Right (ref endpoint) => {
                for (ix, (payload, meta)) in (0..) .zip (package.payloads.iter_mut()) {
                    if meta.pongs != 0 {continue}  // We don't retransmit the delivered discovery pings at present.
                    if meta.pings == u8::max_value() {continue}  // A natural limit on the number of retries.

                    let pong = payload.pong == 1;
                    //pintln! ("transmit] Sending " [payload.id] " " if pong {"pong"} else {"ping"} " to " [endpoint] "…");
                    if let Err (err) = ping (dugout, endpoint, payload) {
                        log! ("ping error: " (err))
                    } else {
                        meta.pings += 1
                    }
                    // Don't sent the pongs twice (because we're not getting pong replies to pongs and won't remove them that way).
                    if pong {finished.push (ix)}
                }
            },
            // Recipient is a `LP_mypub25519` key of a peer.
            Either::Left ((seed, ref send_handler)) => {
                // Skip the transmission if the `Arc<SendHandler>` was dropped.
                let _send_handler = match send_handler.upgrade() {
                    Some (arc) => arc,
                    None => {
                        package.payloads.clear();  // Marks the package for removal.
                        continue
                    }
                };

                let mut limops = ratelim_maintenance (&seed);

                // See if the seed is a friend with directly-reachable endpoints.
                if let Some (friend) = friends.get_mut (&seed) {
                    for (endpoint, _endpoint_meta) in friend.endpoints.iter_mut() {
                        for (payload, meta) in package.payloads.iter_mut() {
                            if meta.pongs != 0 {continue}  // TODO: Should retransmit ponged packets, but later and less often.
                            if meta.pings == u8::max_value() {continue}  // Reached a natural retransmission limit.
                            if limops > 33. {continue}
                            //pintln! ("transmit] Sending " [payload.id] " ping to " [endpoint] "…");
                            if let Err (err) = ping (dugout, endpoint, payload) {
                                log! ("ping error: " (err))
                            } else {
                                with_ratelim (&seed, |_lm, ops| {*ops += 1.; limops = *ops});
                                meta.pings += 1
                            }
                        }
                    }
                }

                let now = now_float();
                for (payload, meta) in package.payloads.iter_mut() {
                    // Delay the `dht_put` a bit (skipping for a few loops) if we're trying direct communication.
                    if meta.pings > 0 && now - package.scheduled_at < 1. {continue}
                    // NB: `dht_put` is usually reliable, but not 100% so.
                    //     In particular, invoking `dht_put` during DHT initialization might have no effect.
                    //     And DHT might reboot when the external IP changes, adding a chance of some `dht_put` calls being skipped.
                    //     And the DHT topology might change due to churn.
                    //     We're supposed to maintain DHT values by rewriting them while they are still needed.
                    //     So here after a little while we're repeating the `dht_put`.
                    if now - meta.dht_put_invoked < 20. {continue}
                    if limops > 10. {continue}
                    if meta.dht_put_invoked > 0. && limops > 1. {continue}

                    let salt = if let Some (ref salt) = payload.salt {salt.clone()} else {continue};
                    let chunk = if let Some (ref chunk) = payload.chunk {chunk.clone()} else {continue};
                    let saltʲ = salt.clone();
                    let shuttle = Arc::new (PutShuttle {
                        put_handler: Box::new (move |_have: &[u8]| -> Result<Vec<u8>, String> {
                            let benload = try_s! (serde_bencode::ser::to_bytes (&chunk));
                            let _subject_salt = binprint (&saltʲ[0 .. saltʲ.len() - 1], b'.');
                            let _idx = saltʲ.last().unwrap();
                            //log! ("transmit] chunk " (_subject_salt) '.' (_idx) " stored, " (_have.len()) " → " (benload.len()));
                            with_ratelim (&seed, |_lm, ops| *ops += 1.);
                            Ok (benload)
                        })
                    });
                    let mut shuttles = unwrap! (PUT_SHUTTLES.lock());
                    let shuttle_ptr = (&*shuttle) as *const PutShuttle as *const c_void;
                    shuttles.insert (shuttle_ptr as usize, (now as u64, shuttle));

                    with_ratelim (&seed, |_lm, ops| {*ops += 1.; limops = *ops});
                    unsafe {dht_put (dugout, seed.bytes.as_ptr(), seed.bytes.len() as i32, salt.as_ptr(), salt.len() as i32, put_callback, shuttle_ptr, now as u64)}
                    meta.dht_put_invoked = now
                }
            }
        }
        for ix in finished.into_iter().rev() {
            package.payloads.remove (ix);
        }
        if package.payloads.is_empty() {
            // A package was fully delivered.
            packages.remove (pix);
        }
        try_s! (hf_transmit (&pctx, hf_addr, &our_public_key, packages));
    }

    Ok(())
}

fn pingʹ (ctx: &MmArc, from: &bits256, endpoint: &SocketAddr, pong: Option<ByteBuf>) {
    let pctx = match PeersContext::from_ctx (ctx) {Ok (c) => c, Err (err) => {log! ((err)); return}};
    let mut trans = unwrap! (pctx.trans_meta.lock());
    let (pong, id) = if let Some (original_id) = pong {
        (true, original_id)
    } else {
        (false, MmPayload::next_id (&mut trans))
    };
    //pintln! ("Sending a " if pong {"pong"} else {"ping"} ' ' [id] " to " [endpoint] "…");

    let mm_payload = MmPayload {
        id,
        from: ByteBuf::from ((&from.bytes[..]) .to_vec()),
        pong: if pong {1} else {0},
        salt: None,
        chunk: None
    };

    let direct_package = Package {
        payloads: vec! [(mm_payload.clone(), PayloadOutMeta::default())],
        to: Either::Right (*endpoint),
        scheduled_at: now_float(),
        fallback: None
    };

    trans.packages.push (direct_package);
}

/// Invoked from the `peers_thread`, implementing the `LtCommand::Put` op.  
/// NB: If the `data` is large then we block to rate-limit.
fn split_and_put (ctx: &MmArc, from: &bits256, seed: bits256, mut salt: Salt, mut data: Vec<u8>,
                  send_handler: Weak<SendHandler>, fallback: NonZeroU8) {
    // chunk 1 {{number of chunks, 1 byte; piece of data} crc32}
    // chunk 2 {{piece of data} crc32}
    // chunk 3 {{piece of data} crc32}

    // Prepend the number of chunks into the `data`,
    // allowing the receiving side to get the number of chunks from the first one.
    // We can store at most 992 bytes in a chunk (BEP44 1000 bytes limit - 4 bytes bencode overhead - 4 bytes checksum).

    // We're using `(1..)` ranges and as of today they seem to overflow on `254u8`. For example, this overflows:
    // println! ("{:?}", (1u8..) .zip (0..254) .collect::<Vec<_>>());
    let max_chunks = 253;

    let number_of_chunks = (data.len() + 1) / 992 + if (data.len() + 1) % 992 != 0 {1} else {0};
    let number_of_chunks = if number_of_chunks > max_chunks {
        log! ("split_and_put] Error, payload (" (data.len()) " bytes) is too large for `peers`.");
        return
    } else {
        number_of_chunks as u8
    };
    data.insert (0, number_of_chunks);

    // Split the `data` into chunks.

    let mut chunks: Vec<Vec<u8>> = data.chunks (992) .map (|slice| slice.into()) .collect();
    assert_eq! (chunks.len(), number_of_chunks as usize);

    // Calculate the CRC for every chunk.
    // We should be able to check the chunks independently on the receiving side (that is, no CRC streaming between the chunks)
    // in order for the receiving side to swiftly retry getting the chunk if there's a CRC mismatch.

    for (idx, chunk) in (1..) .zip (chunks.iter_mut()) {
        let mut crc = update (idx, &IEEE_TABLE, &chunk);
        crc = update (crc, &IEEE_TABLE, &seed.bytes[..]);
        crc = update (crc, &IEEE_TABLE, &salt);
        unwrap! (chunk.write_u32::<BigEndian> (crc));
        assert! (chunk.len() <= 996);
    }

    // Submit the chunks into delivery queue, appending the chunk number (1-based) to salt.

    let now = now_float();
    unwrap! (PUT_SHUTTLES.lock()) .retain (|_, (created, _)| now as u64 - *created < 600 * 1000);

    let pctx = unwrap! (PeersContext::from_ctx (ctx));
    let salt_base_len = salt.len();
    let mut package = Package {
        payloads: Vec::new(),
        to: Either::Left ((seed, send_handler)),
        scheduled_at: now,
        fallback: Some (fallback)
    };

    let mut trans = unwrap! (pctx.trans_meta.lock());

    for (idx, chunk) in (1..) .zip (chunks) {
        salt.truncate (salt_base_len);
        salt.push (idx);  // Should not be zero. A zero in the salt might be lost along the way (`CStr::from_ptr`).

        package.payloads.push ((MmPayload {
            id: MmPayload::next_id (&mut trans),
            from: ByteBuf::from (&from.bytes[..]),
            pong: 0,
            salt: Some (ByteBuf::from (&salt[..])),
            chunk: Some (ByteBuf::from (&chunk[..]))
        }, PayloadOutMeta::default()));
    }

    trans.packages.push (package);
}

/// Big values are split into chunks in order to conform to the BEP 44 size limit.
/// This structure keeps information about a fetched chunk.
#[derive(Debug, Default)]
struct ChunkGetsEntry {
    /// The time when we last issued a `dht_get` for the chunk.
    restarted: f64,
    /// The time when the chunk was received via a direct DHT ping.
    direct: f64,
    /// Version, seq * 2 + auth.
    seq_auth: u64,
    /// Checksum-verified chunk data obtained from libtorrent.
    payload: Option<Vec<u8>>
}

/// For each salt being sought, tracks the get operations currently in progress.
#[derive(Debug, Default)]
struct GetsEntry {
    /// Libtorrent's public key derived from `seed`.  
    /// In some APIs we only get this derived public key and not the `seed`.
    pk: Option<[u8; 32]>,
    /// Last time (in seconds since UNIX epoch) when we had all the chunks
    /// necessary to reassemble the value which corresponds to the salt.
    /// We can have several reassembly attempts, each bumping this field.
    reassembled_at: Option<f64>,
    /// How many chunks were used by the sender to upload the value.  
    /// Retrieved from the first chunk.
    number_of_chunks: Option<NonZeroU8>,
    /// Chunks we're getting from DHT.
    chunks: Vec<ChunkGetsEntry>,
    /// Futures currently interested in this entry (waiting for it).
    futures: HashMap<NonZeroU64, Task>,
    /// The time when we first discovered that the `futures` is empty.  
    /// Reset to `None` if there are `futures`.
    idle_since: Option<f64>,
    /// The time (in seconds since UNIX epoch) when this `GetsEntry` was created.  
    /// Might be `None` if the entry was created beforehand.
    created_at: Option<f64>,
    /// The time in seconds after which we begin to suspect that the entry was unduly delayed,
    /// like when there's a connectivity problem. (If there are delayed entries then we might want
    /// to contact the HTTP fallback service for extra reliability).  
    /// Might be `None` if the value wasn't known when this entry was created.
    fallback: Option<NonZeroU8>
}
impl GetsEntry {
    /// Invoked when the future `frid` is dropped.
    fn drop_get (&mut self, frid: NonZeroU64) {
        self.futures.remove (&frid);
    }
}

/// A map from a subject salt (e.g. without chunk suffix) to GetsEntry.
type Gets = HashMap<Salt, GetsEntry>;

/// Unpack and check the incoming chunk, then add it to `Gets`.  
/// Update the `GetsEntry::number_of_chunks` if the incoming chunk is the chunk number one.
fn chunk_to_gets (i_salt: &Salt, i_chunk: &Vec<u8>, our_public_key: &bits256, gets: &mut Gets)
-> Result<u8, String> {
    if i_salt.len() < 2 {return ERR! ("short ping salt")}
    let subject_salt = &i_salt[0 .. i_salt.len() - 1];
    let chunk_idx = i_salt[i_salt.len() - 1];
    if chunk_idx == 0 {return ERR! ("bad chunk index")}

    // Reject the chunk if there is a checksum mismatch.
    let mut payload = i_chunk.to_vec();
    if payload.len() < 5 {return ERR! ("short ping chunk")}
    let incoming_checksum = try_s! ((&payload[payload.len() - 4 ..]) .read_u32::<BigEndian>());
    for _ in 0..4 {payload.pop();}  // Drain the checksum.
    let mut crc = update (chunk_idx as u32, &IEEE_TABLE, &payload);
    crc = update (crc, &IEEE_TABLE, &our_public_key.bytes[..]);
    crc = update (crc, &IEEE_TABLE, &subject_salt);
    if incoming_checksum != crc {return ERR! ("bad ping chunk")}

    let mut gets_en = gets.raw_entry_mut().from_key (subject_salt);
    let (_, gets) = match gets_en {
        RawEntryMut::Occupied (ref mut oe) => oe.get_key_value_mut(),
        RawEntryMut::Vacant (ve) => ve.insert (subject_salt.to_vec(), GetsEntry::default())
    };
    if chunk_idx == 1 {
        let number_of_chunks = try_s! (NonZeroU8::new (payload.remove (0)) .ok_or ("zero chunks"));
        gets.number_of_chunks = Some (number_of_chunks)
    }
    let number_of_chunks = match gets.number_of_chunks {
        Some (n) => n,
        None => return ERR! ("no number_of_chunks")  // Out of order delivery?
    };
    if chunk_idx > number_of_chunks.get() {return ERR! ("ping chunk out of bounds")}
    gets.chunks.resize_with (number_of_chunks.get() as usize, Default::default);
    let mut en = &mut gets.chunks[chunk_idx as usize - 1];
    en.direct = now_float();
    en.payload = Some (payload);

    Ok (chunk_idx)
}

/// Copy chunks obtained via HTTP fallback into `Gets`.
fn hf_to_gets (our_public_key: &bits256, pctx: &PeersContext, gets: &mut Gets) -> Result<(), String> {
    if pctx.hf_last_poll_id.load (AtomicOrdering::Relaxed) != 0 {
        let hf_inbox = try_s! (pctx.hf_inbox.lock());
        for (i_salt, (_from, chunk)) in hf_inbox.iter() {
            let subject_salt = &i_salt[0 .. i_salt.len() - 1];
            if !gets.contains_key (subject_salt) {continue}  // We're not currently getting that subject.
            let _chunk_idx = try_s! (chunk_to_gets (i_salt, chunk, our_public_key, gets));
        }
    }
    Ok(())
}

/// Responsible for reassembling all the DHT pieces stored for a potentially large value.
/// Invoked whenever we see continued interest for the value
/// (note that the fetching should be dropped if the interest vanishes)
/// or when after one of the fetched pieces arrives.
/// 
/// * `seed` - The public key we're getting the data for (usually equals `our_public_key`).
/// * `salt` - The subject salt of the payload we're interested in.
#[cfg(feature = "native")]
fn get_pieces_scheduler (seed: &bits256, salt: Salt, frid: NonZeroU64, task: Task, fallback: NonZeroU8,
                         dugout: &mut dugout_t, gets: &mut Gets, pctx: &PeersContext) {
    let getsᵉ = match gets.entry (salt) {
        Entry::Vacant (getsᵉ) => {
            // Fetch the first chunk.
            // Having it we'll know the number of chunks necessary to reassemble the entire value.
            let mut chunk_salt = getsᵉ.key().clone();
            chunk_salt.push (1);  // Identifies the first chunk.
            let mut pk: [u8; 32] = unsafe {zeroed()};
            if option_env! ("MM2_DHT_GET") != Some ("f") {unsafe {
                dht_get (dugout, seed.bytes.as_ptr(), seed.bytes.len() as i32,
                    chunk_salt.as_ptr(), chunk_salt.len() as i32, pk.as_mut_ptr(), pk.len() as i32)
            }}
            getsᵉ.insert (GetsEntry {
                pk: Some (pk),
                reassembled_at: None,
                number_of_chunks: None,
                chunks: vec! [ChunkGetsEntry {restarted: now_float(), direct: 0., seq_auth: 0, payload: None}],
                futures: HashMap::from_iter (once ((frid, task))),
                idle_since: None,
                created_at: Some (now_float()),
                fallback: Some (fallback)
            });
            return
        },
        Entry::Occupied (mut oe) => {
            let getsᵉ = oe.get_mut();
            getsᵉ.futures.insert (frid, task);
            // Entries might be created before they are requested
            // (like when we're getting a direct DHT ping from a C callback),
            // so we should store the correct `fallback` value afterwards.
            getsᵉ.fallback = Some (fallback);
            oe
        }
    };

    get_pieces_scheduler_en (seed, dugout, getsᵉ, pctx)
}

#[cfg(feature = "native")]
fn get_pieces_scheduler_en (seed: &bits256, dugout: &mut dugout_t,
                            mut getsᵉ: OccupiedEntry<Salt, GetsEntry, DefaultHashBuilder>,
                            pctx: &PeersContext) {
    // Skip or GC the package if there are no clients still working on it.

    let (skip, remove) = loop {
        let gets = getsᵉ.get_mut();
        if gets.futures.is_empty() {
            if let Some (idle_since) = gets.idle_since {
                let idle_for = now_float() - idle_since;
                if idle_for > 600. {break (true, true)}
            } else {
                gets.idle_since = Some (now_float())
            }
            break (true, false)
        } else {
            gets.idle_since = None;
            break (false, false)
        }
    };
    if remove {getsᵉ.remove_entry(); return}
    if skip {return}

    // See if the first chunk has arrived and the number of chunks with it.

    let now = now_float();
    if let Some (number_of_chunks) = getsᵉ.get().number_of_chunks {
        // We'll never reassemble the right value while having extra chunks.
        getsᵉ.get_mut().chunks.resize_with (number_of_chunks.get() as usize, Default::default)
    }

    // Go over the chunks and see if it's time to maybe retry fetching some of them.

    let salt = getsᵉ.key().clone();
    let mut pk: [u8; 32] = match getsᵉ.get().pk {
        Some (have) => have,
        None => Default::default()  // Tells `dht_get` to fill it.
    };
    let mut limops = ratelim_maintenance (seed);  // DHT nodes will ban us if we ask for too much too soon.
    fn ordering (restarted_a: f64, restarted_b: f64, missing_a: bool, missing_b: bool) -> Ordering {
        if missing_a != missing_b {
            if missing_a {Ordering::Less} else {Ordering::Greater}
        } else if restarted_a != restarted_b {
            if restarted_a < restarted_b {Ordering::Less} else {Ordering::Greater}
        } else {
            Ordering::Equal
        }
    }
    for (idx, chunk) in (1..) .zip (getsᵉ.get_mut().chunks.iter_mut())
    .sorted_by (|(_, ca), (_, cb)| ordering (ca.restarted, cb.restarted, ca.payload.is_none(), cb.payload.is_none())) {
        if now - chunk.restarted < 10. || now - chunk.direct < 10. || limops > 10. {continue}

        let mut chunk_salt = salt.clone();
        chunk_salt.push (idx);  // Identifies the chunk.
        //log! ("dht_get on" if chunk.payload.is_none() {" a missing"}  " chunk " (binprint (&salt, b'.')) '.' (idx)
        //      " after " {"{:.1}", now - chunk.restarted}
        //      if limops > 1. {" limops " (limops)});
        if option_env! ("MM2_DHT_GET") != Some ("f") {unsafe {
            dht_get (dugout,
                seed.bytes.as_ptr(), seed.bytes.len() as i32,
                chunk_salt.as_ptr(), chunk_salt.len() as i32,
                pk.as_mut_ptr(), pk.len() as i32)
        }}
        chunk.restarted = now;
        with_ratelim (seed, |_lm, ops| {*ops += 1.; limops = *ops})
    }
    getsᵉ.get_mut().pk = Some (pk);  // In case it was initialized by `dht_get`.

    // Reassemble the value.
    // We're doing it every time because the version of a chunk payload (and the number of chunks) might have been changed since the last time.

    let missing_chunks = getsᵉ.get().chunks.iter().any (|chunk| chunk.payload.is_none());
    if missing_chunks {return}
    let mut buf = Vec::with_capacity (getsᵉ.get().chunks.len() * 992);
    for chunk in &getsᵉ.get().chunks {for &byte in unwrap! (chunk.payload.as_ref()) {buf.push (byte)}}
    if getsᵉ.get_mut().reassembled_at.is_none() {getsᵉ.get_mut().reassembled_at = Some (now)}

    let mut fetched = match pctx.recently_fetched.lock() {
        Ok (gets) => gets,
        Err (err) => {log! ("get_pieces_scheduler] Can't lock the `PeersContext::recently_fetched`: " (err)); return}
    };
    match fetched.entry (getsᵉ.key().clone()) {
        Entry::Vacant (ve) => {ve.insert ((now_float(), buf));},
        Entry::Occupied (oe) => *oe.into_mut() = (now_float(), buf)
    }
    for task in getsᵉ.get().futures.values() {task.notify()}
}

const BOOTSTRAP_STATUS: &[&dyn TagParam] = &[&"dht-boot"];

/// I've noticed that if we create a libtorrent session (`lt::session`) and destroy it right away
/// then it will often crash. Apparently we're catching it unawares during some initalization procedures.
/// Delaying the libtorrent bootstrap allows us to skip the libtorrent code altogether
/// in some of the short-lived unit tests where the DHT isn't actually used.
struct DhtDelayed {until: f64}
#[cfg(feature = "native")]
impl DhtDelayed {
    fn init (ctx: &MmArc, seconds: f64) -> DhtDelayed {
        if seconds > 0. {
            ctx.log.status (BOOTSTRAP_STATUS, "DHT bootstrap delayed ...") .detach();
        }
        DhtDelayed {until: now_float() + seconds}
    }
    fn kick (self, ctx: &MmArc, dugout: &mut dugout_t) -> Either<DhtDelayed, Result<DhtBootstrapping, String>> {
        if now_float() > self.until {
            match DhtBootstrapping::bootstrap (ctx, dugout) {
                Ok (bootstrapping) => Either::Right (Ok (bootstrapping)),
                Err (err) => Either::Right (ERR! ("{}", err))
            }
        } else {
            Either::Left (self)
}   }   }
struct DhtBootstrapping;
#[cfg(feature = "native")]
impl DhtBootstrapping {
    fn bootstrap (ctx: &MmArc, dugout: &mut dugout_t) -> Result<DhtBootstrapping, String> {
        ctx.log.status (BOOTSTRAP_STATUS, "DHT bootstrap ...") .detach();
        unsafe {enable_dht (dugout)};
        if let Some (err) = dugout.take_err() {
            ctx.log.status (BOOTSTRAP_STATUS, &fomat! ("DHT bootstrap error: " (err)));
            return ERR! ("enable_dht error: {}", err)
        }
        Ok (DhtBootstrapping)
    }
    fn bootstrapped (self) -> DhtBootstrapped {DhtBootstrapped}
}
struct DhtBootstrapped;

enum DhtBootStatus {
    DhtDelayed (DhtDelayed),
    DhtBootstrapping (DhtBootstrapping),
    DhtBootstrapped (DhtBootstrapped)
}
ifrom! (DhtBootStatus, DhtDelayed);
ifrom! (DhtBootStatus, DhtBootstrapping);
ifrom! (DhtBootStatus, DhtBootstrapped);

struct CbCtx<'a, 'b, 'c> {
    /// Seed, salt, frid -> GetsEntry.
    gets: &'a mut Gets,
    ctx: &'b MmArc,
    our_public_key: bits256,
    bootstrapped: &'c mut f64
}

/// Process the incoming DHT pings,
/// some of which are used for direct NAT-traversing communication between peers.
fn incoming_ping (cbctx: &mut CbCtx, pkt: &[u8], ip: &[u8], port: u16) -> Result<(), String> {
    if option_env! ("MM2_INCOMING_PING") == Some ("f") {return Ok(())}
    let ping = match bdecode::<Ping<MmPayload>> (pkt) {Ok (p) => p, Err (_) => return Ok(())};

    let from = &ping.a.mm.from[..];
    if from.len() != 32 {return ERR! ("Wrong `from` length in a ping: {}", from.len())}
    let from = bits256 {bytes: *array_ref! (from, 0, 32)};

    let ip: IpAddr = try_s! (unsafe {from_utf8_unchecked (ip)} .parse());
    //log! ("incoming_ping] from " (ip) " port " (port) " key " (from) ' ' [ping.a.mm.id] if ping.a.mm.pong == 1 {" pong"}
    //      if let Some (ref salt) = ping.a.mm.salt {" salt " (binprint (salt, b'.'))});
    let pctx = try_s! (PeersContext::from_ctx (cbctx.ctx));
    pctx.direct_pings.fetch_add (1, AtomicOrdering::Relaxed);

    let endpoint = SocketAddr::new (ip, port);
    if ping.a.mm.pong == 0 {
        pingʹ (cbctx.ctx, &cbctx.our_public_key, &endpoint, Some (ping.a.mm.id.clone()))  // Pong.
    }
    let pctx = try_s! (PeersContext::from_ctx (cbctx.ctx));
    // Now that we've got a direct ping from a friend, see if we can update the endpoints we have on record.
    let mut trans = try_s! (pctx.trans_meta.lock());
    let friend = trans.friends.entry (from) .or_default();
    match friend.endpoints.entry (endpoint) {
        Entry::Occupied (_oe) => {},
        Entry::Vacant (ve) => {ve.insert (());}
    };

    // Register the pong.
    if ping.a.mm.pong == 1 {
        for package in trans.packages.iter_mut() {
            for (payload, meta) in package.payloads.iter_mut() {
                if payload.id == ping.a.mm.id {
                    meta.pongs += 1
    }   }   }   }

    // Copy the chunk into `gets` where the `get_pieces_scheduler` will see it.
    let mm: MmPayload = ping.a.mm;
    if let (Some (i_salt), Some (i_chunk)) = (mm.salt.as_ref(), mm.chunk.as_ref()) {
        let chunk_idx = try_s! (chunk_to_gets (i_salt, i_chunk, &cbctx.our_public_key, &mut cbctx.gets));
        pctx.direct_chunks.fetch_add (1, AtomicOrdering::Relaxed);
        log! ("incoming_ping] Chunk " (chunk_idx) " received directly");
    }

    Ok(())
}

/// The loop driving the peers crate.
#[cfg(feature = "native")]
fn peers_thread (ctx: MmArc, _netid: u16, our_public_key: bits256, preferred_port: u16, read_only: bool, delay_dht: f64) {
    if let Err (err) = ctx.log.register_my_thread() {log! ((err))}
    let myipaddr = ctx.conf["myipaddr"].as_str();
    let listen_interfaces = (|| {
        if let Some (myipaddr) = myipaddr {
            let ip: IpAddr = unwrap! (myipaddr.parse());
            if ip.is_loopback() || ip.is_multicast() || ip.is_global() {
                log! ("Warning, myipaddr '" (myipaddr) "' does not appear globally routable, not using it for DHT");
            } else {
                return fomat! ((myipaddr) ":" (preferred_port))
            }
        }
        fomat! ("0.0.0.0:" (preferred_port) ",[::]:" (preferred_port))
    })();
    // TODO: Log the *actual* binding IP and port (when we get it from an alert). The actual port might be bumped up if the `preferred_port` binding fails.
    //pintln! ("preferred_port: " (preferred_port) "; listen_interfaces: " (listen_interfaces));
    let listen_interfaces = unwrap! (CString::new (listen_interfaces));
    let mut dugout = unsafe {dht_init (listen_interfaces.as_ptr(), read_only)};
    if let Some (err) = dugout.take_err() {
        // TODO: User-friendly log message (`LogState::log`).
        log! ("dht_init error: " (err));
        return
    }

    let dht_stateᵖ = ctx.dbdir().join ("lt-dht");
    let dht_state = slurp (&dht_stateᵖ);
    if !dht_state.is_empty() {
        // Note: Successful state reuse is reflected with a "DHT node: bootstrapping with 371 nodes" alert.
        // Whereas without the state it's "DHT node: bootstrapping with 0 nodes".
        unsafe {dht_load_state (&mut dugout, dht_state.as_ptr(), dht_state.len() as i32)};
        // TODO: User-friendly log message (`LogState::log`).
        if let Some (err) = dugout.take_err() {log! ("dht_load_state (" [dht_stateᵖ] ") error: " (err))}
    }

    let pctx = unwrap! (PeersContext::from_ctx (&ctx));

    let mut boot_status = DhtBootStatus::from (DhtDelayed::init (&ctx, delay_dht));

    let mut bootstrapped = 0.;
    let mut last_state_save = 0.;

    let mut gets = Gets::default();

    let hf_addr = loop {
        let ip = {
            let seeds = unwrap! (ctx.seeds.lock());
            if seeds.is_empty() {break None}
            seeds[0].clone()
        };
        let port = ctx.conf["http-fallback-port"].as_u64().unwrap_or (80) as u16;
        break Some (SocketAddr::new (ip, port))
    };

    loop {
        extern fn cb (_dugout: *mut dugout_t, cbctx: *mut c_void, alert: *mut Alert) {
            //let dugout: &mut dugout_t = unsafe {&mut *dugout};
            let cbctx: &mut CbCtx = unsafe {&mut *(cbctx as *mut CbCtx)};

            // We don't want to hit the 1000 bytes limit
            // (in BEP 44 it's optional, but I guess a lot of implementations enforce it by default),
            // meaning that a limited-size buffer is enough to get the data from C.
            let mut buf: [u8; 2048] = unsafe {uninitialized()};

            let mut ipbuf: [u8; 64] = unsafe {uninitialized()};
            let mut direction: i8 = 1;  // Interested in incoming packets.
            let mut ipbuflen = ipbuf.len() as i32;
            let mut port: u16 = 0;
            let rc = unsafe {as_dht_pkt_alert (alert,
                buf.as_mut_ptr(), buf.len() as i32,
                &mut direction,
                ipbuf.as_mut_ptr(), &mut ipbuflen,
                &mut port)};
            if rc > 0 {
                let rc = incoming_ping (cbctx, &buf[0 .. rc as usize], &ipbuf[0 .. ipbuflen as usize], port);
                if let Err (err) = rc {log! ("incoming_ping error: " (err))}
            } else if rc < 0 {
                log! ("as_dht_pkt error: " (rc));
            }

            ipbuflen = ipbuf.len() as i32;
            let rc = unsafe {as_external_ip_alert (alert, ipbuf.as_mut_ptr(), &mut ipbuflen)};
            if rc == 1 {
                let eip = unsafe {from_raw_parts (ipbuf.as_ptr(), ipbuflen as usize)};
                // This alert would sometimes inform us about a *changed* external IP
                // (and can not be used to detect the initial external IP).
                log! ("external_ip_alert: " (unsafe {from_utf8_unchecked (eip)}));
            } else if rc < 0 {
                log! ("as_external_ip_alert error: " (rc));
            }

            let mut keybuf: [u8; 32] = unsafe {uninitialized()};
            let mut saltbuf: [c_char; 256] = unsafe {uninitialized()};
            let mut seq: i64 = 0;
            let mut auth: bool = false;
            let rc = unsafe {as_dht_mutable_item_alert (alert,
                keybuf.as_mut_ptr(), keybuf.len() as i32,
                saltbuf.as_mut_ptr(), saltbuf.len() as i32,
                buf.as_mut_ptr(), buf.len() as i32,
                &mut seq, &mut auth)};
            if rc > 0 {
                let bencoded = &buf[0 .. rc as usize];
                let chunk_salt = unsafe {CStr::from_ptr (saltbuf.as_ptr())} .to_bytes();
                let idx = chunk_salt[chunk_salt.len() - 1] as usize;  // 1-based.
                if idx == 0 {return}  // `idx` can't be 0, ergo the received payload is garbage.
                //pintln! ("chunk " (idx) ", dht_mutable_item_alert, " [=rc] ' ' [=seq] ' ' [=auth]);

                let payload: ByteBuf = if bencoded == b"0:" {ByteBuf::new()} else {
                    match serde_bencode::de::from_bytes (bencoded) {
                        Ok (payload) => payload,
                        Err (err) => {log! ("peers_thread] Can not decode the received payload: " (err)); return}
                    }
                };
                let mut payload = payload.to_vec();

                let salt: Salt = (&chunk_salt[0 .. chunk_salt.len() - 1]) .into();  // Without the chunk number suffix.

                let gets = match cbctx.gets.iter_mut().find (|en| &en.0[..] == &salt[..] && en.1.pk == Some (keybuf)) {
                    Some (gets_entry) => gets_entry.1,
                    None => return
                };
                if idx > gets.chunks.len() {return}

                // Reject the chunk if there is a checksum mismatch.
                if payload.len() < 5 {return}  // A chunk without a checksum and at least a single byte of payload is gibberish.
                let incoming_checksum = match (&payload[payload.len() - 4 ..]) .read_u32::<BigEndian>() {Ok (c) => c, Err (_err) => return};
                for _ in 0..4 {payload.pop();}  // Drain the checksum.
                let mut crc = update (idx as u32, &IEEE_TABLE, &payload);
                crc = update (crc, &IEEE_TABLE, &cbctx.our_public_key.bytes[..]);
                crc = update (crc, &IEEE_TABLE, &salt);
                if incoming_checksum != crc {return}

                let number_of_chunks = if idx == 1 {
                    match NonZeroU8::new (payload.remove (0)) {
                        Some (nz) => Some (nz),
                        None => return  // Invalid ping, number_of_chunks can not be zero.
                    }
                } else {None};
                let seq_auth = unsafe {read_volatile (&seq) as u64 * 2 + if read_volatile (&auth) {1} else {0}};
                let chunk = &mut gets.chunks[idx-1];
                if chunk.payload.is_none() || seq_auth > chunk.seq_auth {
                    chunk.seq_auth = seq_auth;
                    chunk.payload = Some (payload);
                    if number_of_chunks.is_some() {gets.number_of_chunks = number_of_chunks}
                }
            } else if rc < 0 {
                log! ("as_dht_mutable_item_alert error: " (rc));
            }

            if unsafe {is_dht_bootstrap_alert (alert)} {
                cbctx.ctx.log.claim_status (BOOTSTRAP_STATUS) .map (|status| status.append (" Done."));
                *cbctx.bootstrapped = now_float();
                return
            }

            // TODO: Use `buf`.
            // NB: Looks like libtorrent automatically tries the next port number when it can't bind on the `preferred_port`.
            let endpoint_cs = unsafe {as_listen_succeeded_alert (alert)};
            if !endpoint_cs.is_null() {
                let _endpoint = unwrap! (unsafe {CStr::from_ptr (endpoint_cs)} .to_str());
                // TODO: Listen on "myipaddr" if present.
                //pintln! ("Listening on " (_endpoint));
                unsafe {libc::free (endpoint_cs as *mut c_void)}
                return
            }

            // TODO: Use `buf`.
            let endpoint_cs = unsafe {as_listen_failed_alert (alert)};
            if !endpoint_cs.is_null() {
                let endpoint = unwrap! (unsafe {CStr::from_ptr (endpoint_cs)} .to_str());
                log! ("Can't listen on " (endpoint));
                return
            }

            if *LOG_UNHANDLED_ALERTS {
                // TODO: Use `buf`.
                let cs = unsafe {alert_message (alert)};
                if let Ok (alert_message) = unsafe {CStr::from_ptr (cs)} .to_str() {
                    log! ("lt: " (alert_message))
                }
                unsafe {libc::free (cs as *mut c_void)}
            }
        }

        if ctx.is_stopping() {break}

        // Invoke the `cb` on the libtorrent alerts.
        {
            let mut cbctx = CbCtx {
                gets: &mut gets,
                ctx: &ctx,
                our_public_key,
                bootstrapped: &mut bootstrapped
            };
            unsafe {dht_alerts (&mut dugout, cb, &mut cbctx as *mut CbCtx as *mut c_void)};
        }
        if let Some (err) = dugout.take_err() {
            log! ("dht_alerts error: " (err));
            // Try again and see if the error would go away.  
            // (Given that libtorrent is not our only protocol we could also ignore the unexpected lt error,
            //  but we should avoid spamming the log then, maybe use a dashboard condition instead).
            thread::sleep (Duration::from_secs (2));
            continue
        }

        if ctx.is_stopping() {break}

        boot_status = match boot_status {
            DhtBootStatus::DhtDelayed (delayed) => match delayed.kick (&ctx, &mut dugout) {
                Either::Left (delayed) => delayed.into(),
                Either::Right (Ok (bootstrapping)) => bootstrapping.into(),
                Either::Right (Err (err)) => {log! ((err)); return}
            },
            DhtBootStatus::DhtBootstrapping (bootstrapping) => {
                if bootstrapped > 0. {
                    bootstrapping.bootstrapped().into()
                } else {
                    bootstrapping.into()
            }   },
            DhtBootStatus::DhtBootstrapped (bootstrapped) => bootstrapped.into()
        };

        match pctx.cmd_rx.recv_timeout (Duration::from_millis (100)) {
            Ok (LtCommand::Put {seed, salt, payload, send_handler, fallback}) =>
                split_and_put (&ctx, &our_public_key, seed, salt, payload, send_handler, fallback),
            Ok (LtCommand::Get {seed, salt, frid, task, fallback}) => {
                assert_eq! (seed, our_public_key);
                get_pieces_scheduler (&seed, salt, frid, task, fallback, &mut dugout, &mut gets, &*pctx)},
            Ok (LtCommand::DropGet {salt, frid}) => {
                if let Some (en) = gets.get_mut (&salt) {en.drop_get (frid)}
                hf_drop_get (&pctx, &salt)},
            Ok (LtCommand::Ping {endpoint}) => pingʹ (&ctx, &our_public_key, &endpoint, None),
            Err (channel::RecvTimeoutError::Timeout) => {},
            Err (channel::RecvTimeoutError::Disconnected) => break
        };

        if ctx.is_stopping() {break}

        if let Err (err) = transmit (&mut dugout, &ctx, &hf_addr) {log! ("transmit error: " (err))}

        // Cloning the keys allows `get_pieces_scheduler_en` to remove the `gets_oe` entry.
        let get_salts: Vec<_> = gets.keys().map (|k| k.clone()) .collect();
        let now = now_float();
        for salt in get_salts {
            let gets_oe = if let Entry::Occupied (oe) = gets.entry (salt) {oe} else {panic!()};

            if let Some (created_at) = gets_oe.get().created_at {
                if let Some (fallback) = gets_oe.get().fallback {
                    if now - created_at > fallback.get() as f64 {
                        hf_delayed_get (&pctx, gets_oe.key())
            }   }   }

            get_pieces_scheduler_en (&our_public_key, &mut dugout, gets_oe, &*pctx);
        }

        if let Err (err) = hf_poll (&pctx, &hf_addr) {
            log! ("hf_poll error: " (err))
        } else if let Err (err) = hf_to_gets (&our_public_key, &pctx, &mut gets) {
            log! ("hf_to_gets error: " (err))
        }

        // Remove old entries from `hf_maps`.
        if let Ok (mut hf_maps) = pctx.hf_maps.lock() {
            hf_maps.retain (|_seed, track| now - track.last_store < 600.)
        }
        if let Ok (mut recently_fetched) = pctx.recently_fetched.lock() {
            recently_fetched.retain (|_, (lm, _)| now - *lm < 600.)
        }

        let after_boot_sec = 20.;  // In order not to loose some potentially good but not yet checked nodes from a previous state.
        if bootstrapped != 0. && now - bootstrapped > after_boot_sec && now - last_state_save > 600. {
            // TODO, should: Only save the DHT state if we see some recent DHT traffic (via the counters).
            last_state_save = now;
            let mut buflen = 0i32;
            let buf = unsafe {dht_save_state (&mut dugout, &mut buflen)};
            if let Some (err) = dugout.take_err() {
                log! ("dht_save_state error: " (err))
            } else if buf == null_mut() || buflen <= 0 {
                log! ("empty result from dht_save_state");
            } else {
                let tmp_path = temp_dir().join (fomat! ("lt-dht-" (thread_rng().gen::<u64>()) ".tmp"));
                let tmp_path = RaiiRm (Path::new (&tmp_path));
                let buf = unsafe {from_raw_parts (buf, buflen as usize)};
                match fs::File::create (&tmp_path) {
                    Err (err) => log! ("Error creating " [tmp_path] ": " (err)),
                    Ok (mut file) => match file.write_all (buf) {
                        Err (err) => log! ("Error writing to " [tmp_path] ": " (err)),
                        Ok (()) => {
                            drop (file);  // Close before renaming, just in case.
                            match fs::rename (&tmp_path, &dht_stateᵖ) {
                                Err (err) => log! ("Warning, can't rename " [tmp_path] " to " [dht_stateᵖ] ": " (err)),
                                Ok (()) => {
                                    //log! ("DHT state saved to " [dht_state_path])
        }   }   }   }   }   }  }
    }
}

/// * `netid` - We ignore the peers not matching the `netid`. Usually 0.
/// * `our_public_key` - Aka `LP_mypub25519`. This is our ID, allowing us to be different from other peers
///                      and to prove our identity (ownership of the corresponding private key) to a peer.
/// * `preferred_port` - We'll try to open an UDP endpoint on this port,
///                      which might help if the user configured this port in firewall and forwarding rules.
///                      We're not limited to this port though and might try other ports as well.
#[cfg(feature = "native")]
pub fn initialize (ctx: &MmArc, netid: u16, our_public_key: bits256, preferred_port: u16) -> Result<(), String> {
    let drill = is_a_test_drill();

    // NB: From the `fn test_trade` logs it looks like the `session_id` isn't shared with the peers currently.
    //     In "lp_ordermatch.rs" we're [temporarily] using `pair_str` as the session identifier and manually embedding it in the `subject`.
    log! ("initialize] netid " (netid) " public key " (our_public_key) " preferred port " (preferred_port) " drill " (drill));
    if !our_public_key.nonz() {return ERR! ("No public key")}

    // TODO: Set it to `true` for smaller tests and to `false` for real-life deployments.
    // Maybe take the saved DHT state into account: tests always have a fresh directory,
    // whereas the real-life MM2 deployments are often restarted in an existing directory.
    // For small non-DHT tests we don't need to register ourselves.
    let read_only = drill;  // Whether to register in the DHT network.
    // We need to avoid DHT bootstrapping in short tests
    // in order not to confuse the unsafe C code in libtorrent with simultaneous bootstrap and shutdown.
    // Delaying the DHT bootstrap might also help us to test the direct UDP communication.
    // Undocumented {"dht": "on"} option is used in some tests to assure us that the DHT is required.
    let delay_dht = if ctx.conf["dht"].as_str() == Some ("on") {0.} else if drill {33.} else {0.};

    let pctx = try_s! (PeersContext::from_ctx (&ctx));
    *try_s! (pctx.our_public_key.lock()) = our_public_key;
    *try_s! (pctx.peers_thread.lock()) =
        Some (try_s! (thread::Builder::new().name ("peers".into()) .spawn ({
            let ctx = ctx.clone();
            move || peers_thread (ctx, netid, our_public_key, preferred_port, read_only, delay_dht)
        })));
    ctx.on_stop ({
        let ctx = ctx.clone();
        let pctx = pctx.clone();
        Box::new (move || -> Result<(), String> {
            assert! (ctx.is_stopping());  // Check that the `peers_thread` can see the flag.
            if let Ok (mut peers_thread) = pctx.peers_thread.lock() {
                if let Some (peers_thread) = peers_thread.take() {
                    let join_status = ctx.log.status (&[&"dht-stop"], "Waiting for the peers_thread to stop...");
                    // We want to shutdown libtorrent and stuff gracefully,
                    // but the `join` might sometimes hang when we're stopping libtorrent, so we implement a timeout.
                    let (tx, rx) = channel::bounded (1);
                    try_s! (thread::Builder::new().name ("dht-stop".into()) .spawn (move || {
                        let _ = peers_thread.join();
                        let _ = tx.send (());
                    }));
                    match rx.recv_timeout (Duration::from_secs (3)) {
                        Ok (()) => join_status.append (" Done."),
                        Err (_timeout) => join_status.append (" Timeout!")
                    }
                }
            }
            Ok(())
        })
    });

    Ok(())
}

#[derive(Serialize, Deserialize, Debug)]
struct ToPeersInitialize {
    ctx: u32,
    netid: u16,
    our_public_key: bits256,
    preferred_port: u16
}

helper! (peers_initialize, args: ToPeersInitialize, {
    match MmArc::from_ffi_handle (args.ctx) {
        Err (err) => ERR! (concat! (stringify! ($helper_name), "] !from_ffi_handle: {}"), err),
        Ok (ctx) => initialize (&ctx, args.netid, args.our_public_key, args.preferred_port)
    }
});

#[cfg(not(feature = "native"))]
pub fn initialize (ctx: &MmArc, netid: u16, our_public_key: bits256, preferred_port: u16) -> Result<(), String> {
    let pctx = try_s! (PeersContext::from_ctx (&ctx));
    *try_s! (pctx.our_public_key.lock()) = our_public_key;

    try_s! (ctx.send_to_helpers());

    let args = ToPeersInitialize {
        ctx: try_s! (ctx.ffi_handle()),
        netid,
        our_public_key,
        preferred_port
    };
    io_buf_proxy! (peers_initialize, &args, 4096)
}

/// Try to reach a peer and establish connectivity with it while knowing no more than its port and IP.
/// 
/// * `ip` - The public IP where the peer is supposedly listens for incoming connections.
/// * `preferred_port` - The preferred port of the peer.
pub fn investigate_peer (ctx: &MmArc, ip: &str, preferred_port: u16) -> Result<(), String> {
    //pintln! ("investigate_peer] ip " (ip) " preferred port " (preferred_port));
    let pctx = try_s! (PeersContext::from_ctx (&ctx));
    let endpoint = SocketAddr::new (try_s! (ip.parse()), preferred_port);
    try_s! (pctx.cmd_tx.send (LtCommand::Ping {endpoint}));
    Ok(())
}

// AG: I had trouble with Arc::into_raw in some builds (Rust 2019-06-25, pointers are correct, no double frees)
// hence using a map to keep the arcs around.
lazy_static! {static ref SEND_HANDLERS: Mutex<HashMap<u64, Arc<SendHandler>>> = Mutex::new (HashMap::new());}

#[doc(hidden)]
pub fn send_handlers_num() -> usize {unwrap! (SEND_HANDLERS.lock()) .len()}

/// Dropping this (out of a corresponding Arc) stops the transmission effort.
#[derive(Debug)]
pub struct SendHandler;

#[cfg(feature = "native")]
#[no_mangle]
pub unsafe extern fn peers_drop_send_handler (shp1: i32, shp2: i32) {
    let send_handler_ptr = (shp1 as u32 as u64) << 32 | shp2 as u32 as u64;
    if let Ok (mut handlers) = SEND_HANDLERS.lock() {
        handlers.remove (&send_handler_ptr);
}   }

#[cfg(not(feature = "native"))]
extern "C" {pub fn peers_drop_send_handler (shp1: i32, shp2: i32);}

#[derive(Serialize, Deserialize, Debug)]
pub struct SendHandlerRef (u64);

impl From<Arc<SendHandler>> for SendHandlerRef {
    fn from (arc: Arc<SendHandler>) -> SendHandlerRef {
        let mut send_handlers = unwrap! (SEND_HANDLERS.lock());
        let ptr = arc.as_ref() as *const SendHandler;
        let ptr = ptr as u64;
        send_handlers.insert (ptr, arc);
        SendHandlerRef (ptr)
}   }

impl Drop for SendHandlerRef {
    fn drop (&mut self) {
        let shp1 = (self.0 >> 32) as u32 as i32;
        let shp2 = (self.0 & 0xFFFFFFFF) as u32 as i32;
        unsafe {peers_drop_send_handler (shp1, shp2)};
}   }

/// Start sending `data` to the peer.
/// 
/// The transfer itself might take some time,
/// given that we might be waiting for the DHT bootstrap to finish
/// and then for the data to be routed to the corresponding DHT nodes.
/// Or it might happen immediately, if we have already established a direct channel of communication with that peer.
/// 
/// * `subject` - Uniquely identifies the message for both the sending and the receiving sides.
///               Should include some kind of *session* mechanics
///               in order for the receiving side not to get the *older* and outdated versions of the message.
///               (Alternatively the receiving side should be able to recognise and *reject* the outdated versions in the `validator`).
/// * `fallback` - The positive number of seconds after which we are to employ the HTTP fallback.
/// 
/// Returns a `Stream` that represents the effort extended to send the `payload`.
/// There is currently no need to schedule the returned `Stream` on a reactor.
/// It is important to `drop` that `Stream` when the effort is no longer necessary,
/// for instance, when we have received a matching reply from the peer.
/// Specifically, we might be sending the message via different channels of communication (UDP, DHT, etc),
/// some of them slower than others. A message might have been delivered on a fast channel and have received a reply
/// before a slow channel delivery went into full swing.
/// A similar example is UDP retransmissions, as we might be retransmitting the UDP messages until the `Stream` is dropped.
/// Note that we're not trying to implement delivery notifications in the `peers` crate itself
/// because for some channels of communication it will only slow things down and complicate matters even further.
/// We might also be sending the message too early, when the receiving end isn't yet ready for it,
/// so stopping the UDP transmissions after a superficial confirmation or lack of it might be suboptimal,
/// hence the manual control of when the transmission should stop.
/// Think of it as a radio-signal set on a loop.
#[cfg(feature = "native")]
pub fn send (ctx: &MmArc, peer: bits256, subject: &[u8], fallback: u8, payload: Vec<u8>)
-> Result<SendHandlerRef, String> {
    let fallback = match option_env! ("MM2_FALLBACK") {Some (n) => try_s! (n.parse()), None => fallback};
    let fallback = try_s! (NonZeroU8::new (fallback) .ok_or ("fallback is 0"));

    let pctx = try_s! (PeersContext::from_ctx (&ctx));

    // Add the peer into the friendlist, in order to discover and track its endpoints.
    if let Ok (mut trans) = pctx.trans_meta.lock() {
        trans.friends.entry (peer) .or_default();
    }

    if !peer.nonz() {return ERR! ("peer key is empty")}

    // Predictable salt size.
    // NB: There should be no zero bytes in the salt (due to `CStr::from_ptr` and the possibility of a similar problem abroad).
    let salt = format_radix (checksum_ecma (subject), 36);

    let send_handler_arc = Arc::new (SendHandler);
    let send_handler = Arc::downgrade (&send_handler_arc);

    // Tell `peers_thread` to save the data.
    try_s! (pctx.cmd_tx.send (LtCommand::Put {seed: peer, salt, payload, send_handler, fallback}));

    Ok (send_handler_arc.into())
}

#[derive(Serialize, Deserialize, Debug)]
struct ToPeersSend {ctx: u32, peer: bits256, subject: Vec<u8>, fallback: u8, payload: Vec<u8>}

helper! (peers_send, args: ToPeersSend, {
    let ctx = try_s! (MmArc::from_ffi_handle (args.ctx));
    let mut shr = try_s! (send (&ctx, args.peer, &args.subject, args.fallback, args.payload));
    // We need to leak the `SendHandlerRef` into the portable space
    // in order not to terminate the send prematurely.
    let ptr = shr.0;
    shr.0 = 0;
    Ok (ptr)
});

/// Returns the `Arc` address of the `SendHandler`.
#[cfg(not(feature = "native"))]
pub fn send (ctx: &MmArc, peer: bits256, subject: &[u8], fallback: u8, payload: Vec<u8>)
-> Result<SendHandlerRef, String> {
    let to = ToPeersSend {
        ctx: try_s! (ctx.ffi_handle()),
        peer,
        subject: subject.into(),
        fallback,
        payload
    };
    io_buf_proxy! (peers_send, &to, 4096)
}

struct RecvFuture {
    pctx: Arc<PeersContext>,
    seed: bits256,
    salt: Salt,
    validator: Box<dyn Fn(&[u8])->bool + Send>,
    frid: Option<NonZeroU64>,
    /// Start checking the HTTP fallback server for the value
    /// if it doesn't come through other channels in the given number of seconds.
    fallback: NonZeroU8
}
impl Future for RecvFuture {
    type Item = Vec<u8>;
    type Error = String;
    fn poll (&mut self) -> Poll<Vec<u8>, String> {
        if self.frid.is_none() {
            let frid = loop {
                match NonZeroU64::new (thread_rng().gen()) {Some (nz) => break nz, None => continue}
            };

            // Ask the `peers_thread` to fetch the data.
            let get = LtCommand::Get {
                seed: self.seed,
                salt: self.salt.clone(),
                frid,
                task: futures::task::current(),
                fallback: self.fallback
            };
            if let Err (err) = self.pctx.cmd_tx.send (get) {return Err (ERRL! ("!send: {}", err))}

            self.frid = Some (frid)
        }

        {   // Check if the data has arrived.
            let fetched = try_s! (self.pctx.recently_fetched.lock());
            if let Some ((_lm, payload)) = fetched.get (&self.salt) {
                if (self.validator) (payload) {
                    return Ok (Async::Ready (payload.clone()))
                }
            }
        }

        Ok (Async::NotReady)
    }
}
impl Drop for RecvFuture {
    fn drop (&mut self) {
        if let Some (frid) = self.frid {
            if let Err (err) = self.pctx.cmd_tx.send (LtCommand::DropGet {salt: self.salt.clone(), frid}) {
                log! ("!send: " (err))
}   }   }   }

/// * `subject` - Uniquely identifies the message for both the sending and the receiving sides.
///               Should include some kind of *session* mechanics
///               in order for the receiving side not to get the *older* and outdated versions of the message.
///               (Alternatively the receiving side should be able to recognise and *reject* the outdated versions in the `validator`).
/// * `fallback` - The positive number of seconds after which we are to employ the HTTP fallback.
/// * `validator` - Receives candidate `subject`-matching transmissions.
///                 Returning `true` the `validator` gives us a green light to accept the transmission and finish.
///                 Returning `false` says transmission is invalid, corrupted or outdated and that we should keep looking.
/// 
/// Returned `Future` represents our effort to receive the transmission.
/// As of now doesn't need a reactor.
/// Should be `drop`ped soon as we no longer need the transmission.
#[cfg(feature = "native")]
pub fn recv (ctx: &MmArc, subject: &[u8], fallback: u8, validator: Box<dyn Fn(&[u8])->bool + Send>)
-> Box<dyn Future<Item=Vec<u8>, Error=String> + Send> {
    let fallback = match option_env! ("MM2_FALLBACK") {Some (n) => try_fus! (n.parse()), None => fallback};
    let fallback = try_fus! (NonZeroU8::new (fallback) .ok_or ("fallback is 0"));

    let pctx = try_fus! (PeersContext::from_ctx (&ctx));

    let seed: bits256 = {
        let our_public_key = try_fus! (pctx.our_public_key.lock());
        if !our_public_key.nonz() {return Box::new (future::err (ERRL! ("No public key")))}
        *our_public_key
    };

    // Predictable salt size.
    // NB: There should be no zero bytes in the salt (due to `CStr::from_ptr` and the possibility of a similar problem abroad).
    let salt = format_radix (checksum_ecma (subject), 36);

    Box::new (RecvFuture {pctx, seed, salt, validator, frid: None, fallback})
}

#[derive(Serialize, Deserialize, Debug)]
pub enum FixedValidator {
    AnythingGoes,
    Exact (Vec<u8>)
}

#[derive(Serialize, Deserialize, Debug)]
struct ToPeersRecv {ctx: u32, subject: Vec<u8>, fallback: u8, validator: FixedValidator}

helper! (peers_recv, args: ToPeersRecv, {
    let ctx = try_s! (MmArc::from_ffi_handle (args.ctx));
    let validator: Box<dyn Fn(&[u8])->bool + Send> = match args.validator {
        FixedValidator::AnythingGoes => Box::new (|_| true),
        FixedValidator::Exact (bytes) => Box::new (move |candidate| candidate == &bytes[..])
    };
    let rf = recv (&ctx, &args.subject, args.fallback, validator);
    // Exploring, and for the simplicity's sake, we're going to just wait here.
    // Tracking background future execution in portable code can be implemented later if necessary.
    let vec = try_s! (rf.wait());
    Ok (vec)
});

#[cfg(not(feature = "native"))]
pub fn recv (ctx: &MmArc, subject: &[u8], fallback: u8, validator: FixedValidator)
-> Box<dyn Future<Item=Vec<u8>, Error=String> + Send> {
    let args = ToPeersRecv {
        ctx: try_fus! (ctx.ffi_handle()),
        subject: subject.into(),
        fallback,
        validator
    };
    let rc = (|| -> Result<Vec<u8>, String> {io_buf_proxy! (peers_recv, &args, 65536)})();
    Box::new (future::ok (try_fus! (rc)))
}

pub fn key (ctx: &MmArc) -> Result<bits256, String> {
    let pctx = try_s! (PeersContext::from_ctx (&ctx));
    let pk = try_s! (pctx.our_public_key.lock());
    Ok (pk.clone())
}

#[cfg(not(feature = "native"))]
#[no_mangle]
pub extern fn peers_check() -> i32 {1}

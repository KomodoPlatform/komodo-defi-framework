use bytes::Bytes;
use crossbeam::{channel, Sender, Receiver};
use futures::channel::mpsc;
use gstuff::Constructible;
#[cfg(not(feature = "native"))]
use http::Response;
use keys::{DisplayLayout, KeyPair, Private};
use primitives::hash::H160;
use rand::Rng;
use serde_bencode::ser::to_bytes as bencode;
use serde_bencode::de::from_bytes as bdecode;
use serde_bytes::ByteBuf;
use serde_json::{self as json, Value as Json};
use std::any::Any;
use std::collections::HashSet;
use std::collections::hash_map::{Entry, HashMap};
use std::fmt;
use std::net::IpAddr;
#[cfg(feature = "native")]
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use crate::{bits256, small_rng, QueuedCommand};
use crate::log::{self, LogState};

/// MarketMaker state, shared between the various MarketMaker threads.
///
/// Every MarketMaker has one and only one instance of `MmCtx`.
///
/// Should fully replace `LP_globals`.
///
/// *Not* a singleton: we should be able to run multiple MarketMakers instances in a process.
///
/// Any function directly using `MmCtx` is automatically a stateful function.
/// In the future we might want to replace direct state access with traceable and replayable
/// state modifications
/// (cf. https://github.com/artemii235/SuperNET/blob/mm2-dice/mm2src/README.md#purely-functional-core).
/// 
/// `MmCtx` never moves in memory (and it isn't `Send`), it is created and then destroyed in place
/// (this invariant should make it a bit simpler thinking about aliasing and thread-safety,
/// particularly of the C structures during the gradual port).
/// Only the pointers (`MmArc`, `MmWeak`) can be moved around.
/// 
/// Threads only have the non-`mut` access to `MmCtx`, allowing us to directly share certain fields.
pub struct MmCtx {
    /// MM command-line configuration.
    pub conf: Json,
    /// Human-readable log and status dashboard.
    pub log: log::LogState,
    /// Set to true after `lp_passphrase_init`, indicating that we have a usable state.
    /// 
    /// Should be refactored away in the future. State should always be valid.
    /// If there are things that are loaded in background then they should be separately optional,
    /// without invalidating the entire state.
    pub initialized: Constructible<bool>,
    /// True if the RPC HTTP server was started.
    pub rpc_started: Constructible<bool>,
    /// True if the MarketMaker instance needs to stop.
    pub stop: Constructible<bool>,
    /// Unique context identifier, allowing us to more easily pass the context through the FFI boundaries.  
    /// 0 if the handler ID is allocated yet.
    pub ffi_handle: Constructible<u32>,
    /// Callbacks to invoke from `fn stop`.
    pub stop_listeners: Mutex<Vec<Box<dyn FnMut()->Result<(), String>>>>,
    /// The context belonging to the `portfolio` crate: `PortfolioContext`.
    pub portfolio_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// The context belonging to the `ordermatch` mod: `OrdermatchContext`.
    pub ordermatch_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// The context belonging to the `peers` crate: `PeersContext`.
    pub peers_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// The context belonging to the `http_fallback` mod: `HttpFallbackContext`.
    pub http_fallback_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// The context belonging to the `coins` crate: `CoinsContext`.
    pub coins_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// The context belonging to the `prices` mod: `PricesContext`.
    pub prices_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// Seednode P2P message bus channel.
    pub seednode_p2p_channel: (Sender<Vec<u8>>, Receiver<Vec<u8>>),
    /// Standard node P2P message bus channel.
    pub client_p2p_channel: (Sender<Vec<u8>>, Receiver<Vec<u8>>),
    /// `lp_queue_command` shares messages with `lp_command_q_loop` via this channel.  
    /// The messages are usually the JSON broadcasts from the seed nodes.
    pub command_queue: mpsc::UnboundedSender<QueuedCommand>,
    /// The end of the `command_queue` channel taken by `lp_command_q_loop`.
    pub command_queueʳ: Mutex<Option<mpsc::UnboundedReceiver<QueuedCommand>>>,
    /// Broadcast `lp_queue_command` messages saved for WASM.
    pub command_queueʰ: Mutex<Option<Vec<(u64, String)>>>,
    /// RIPEMD160(SHA256(x)) where x is secp256k1 pubkey derived from passphrase.
    /// Replacement of `lp::G.LP_myrmd160`.
    pub rmd160: Constructible<H160>,
    /// Seed node IPs, initialized in `fn lp_initpeers`.
    pub seeds: Mutex<Vec<IpAddr>>,
    /// secp256k1 key pair derived from passphrase.
    /// cf. `key_pair_from_seed`.
    /// Replacement of `lp::G.LP_privkey`.
    pub secp256k1_key_pair: Constructible<KeyPair>,
    /// Coins that should be enabled to kick start the interrupted swaps and orders.
    pub coins_needed_for_kick_start: Mutex<HashSet<String>>,
    /// The context belonging to the `lp_swap` mod: `SwapsContext`.
    pub swaps_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
}
impl MmCtx {
    pub fn with_log_state (log: LogState) -> MmCtx {
        let (command_queue, command_queueʳ) = mpsc::unbounded();
        MmCtx {
            conf: Json::Object (json::Map::new()),
            log,
            initialized: Constructible::default(),
            rpc_started: Constructible::default(),
            stop: Constructible::default(),
            ffi_handle: Constructible::default(),
            stop_listeners: Mutex::new (Vec::new()),
            portfolio_ctx: Mutex::new (None),
            ordermatch_ctx: Mutex::new (None),
            peers_ctx: Mutex::new (None),
            http_fallback_ctx: Mutex::new (None),
            coins_ctx: Mutex::new (None),
            prices_ctx: Mutex::new (None),
            seednode_p2p_channel: channel::unbounded(),
            client_p2p_channel: channel::unbounded(),
            command_queue,
            command_queueʳ: Mutex::new (Some (command_queueʳ)),
            command_queueʰ: Mutex::new (None),
            rmd160: Constructible::default(),
            seeds: Mutex::new (Vec::new()),
            secp256k1_key_pair: Constructible::default(),
            coins_needed_for_kick_start: Mutex::new (HashSet::new()),
            swaps_ctx: Mutex::new (None),
        }
    }

    pub fn rmd160 (&self) -> &H160 {
        lazy_static! {static ref DEFAULT: H160 = [0; 20].into();}
        self.rmd160.or (&|| &*DEFAULT)
    }

    #[cfg(feature = "native")]
    pub fn rpc_ip_port (&self) -> Result<SocketAddr, String> {
        let port = self.conf["rpcport"].as_u64().unwrap_or (7783);
        if port < 1000 {return ERR! ("rpcport < 1000")}
        if port > u16::max_value() as u64 {return ERR! ("rpcport > u16")}

        let rpcip = if !self.conf["rpcip"].is_null() {
            try_s! (self.conf["rpcip"].as_str().ok_or ("rpcip is not a string"))
        } else {
            "127.0.0.1"
        } .to_string();
        let ip: IpAddr = try_s! (rpcip.parse());
        Ok (SocketAddr::new (ip, port as u16))
    }

    /// MM database path.  
    /// Defaults to a relative "DB".
    /// 
    /// Can be changed via the "dbdir" configuration field, for example:
    /// 
    ///     "dbdir": "c:/Users/mm2user/.mm2-db"
    /// 
    /// No checks in this method, the paths should be checked in the `fn fix_directories` instead.
    pub fn dbdir (&self) -> PathBuf {
        let path = if let Some (dbdir) = self.conf["dbdir"].as_str() {
            let dbdir = dbdir.trim();
            if !dbdir.is_empty() {
                Path::new (dbdir)
            } else {
                Path::new ("DB")
            }
        } else {
            Path::new ("DB")
        };
        path.join (hex::encode (&**self.rmd160()))
    }

    pub fn netid (&self) -> u16 {
        let big = self.conf["netid"].as_u64().unwrap_or (0);
        if big > u16::max_value().into() {panic! ("netid {} is too big", big)}
        big as u16
    }

    pub fn stop (&self) {
        if self.stop.pin (true) .is_ok() {
            let mut stop_listeners = unwrap! (self.stop_listeners.lock(), "Can't lock stop_listeners");
            // NB: It is important that we `drain` the `stop_listeners` rather than simply iterating over them
            // because otherwise there might be reference counting instances remaining in a listener
            // that would prevent the contexts from properly `Drop`ping.
            for mut listener in stop_listeners.drain (..) {
                if let Err (err) = listener() {
                    log! ({"MmCtx::stop] Listener error: {}", err})
    }   }   }   }

    /// True if the MarketMaker instance needs to stop.
    pub fn is_stopping (&self) -> bool {
        self.stop.copy_or (false)
    }

    /// Register a callback to be invoked when the MM receives the "stop" request.  
    /// The callback is invoked immediately if the MM is stopped already.
    pub fn on_stop (&self, mut cb: Box<dyn FnMut()->Result<(), String>>) {
        let mut stop_listeners = unwrap! (self.stop_listeners.lock(), "Can't lock stop_listeners");
        if self.stop.copy_or (false) {
            if let Err (err) = cb() {
                log! ({"MmCtx::on_stop] Listener error: {}", err})
            }
        } else {
            stop_listeners.push (cb)
    }   }

    /// Sends the P2P message to a processing thread
    #[cfg(feature = "native")]
    pub fn broadcast_p2p_msg(&self, msg: &str) {
        let i_am_seed = self.conf["i_am_seed"].as_bool().unwrap_or(false);
        if i_am_seed {
            unwrap!(self.seednode_p2p_channel.0.send(msg.to_owned().into_bytes()));
        } else {
            unwrap!(self.client_p2p_channel.0.send(msg.to_owned().into_bytes()));
    }   }

    #[cfg(not(feature = "native"))]
    pub fn broadcast_p2p_msg (&self, msg: &str) {
        use crate::{helperᶜ, BroadcastP2pMessageArgs};
        use crate::executor::spawn;

        let args = BroadcastP2pMessageArgs {ctx: self.ffi_handle.copy_or (0), msg: msg.into()};
        let args = unwrap! (bencode (&args));
        spawn (async move {
            let rc = helperᶜ ("broadcast_p2p_msg", args) .await;
            if let Err (err) = rc {log! ("!broadcast_p2p_msg: " (err))}
        });
    }

    /// Get a reference to the secp256k1 key pair.
    /// Panics if the key pair is not available.
    pub fn secp256k1_key_pair (&self) -> &KeyPair {
        match self.secp256k1_key_pair.as_option() {
            Some (pair) => pair,
            None => panic! ("secp256k1_key_pair not available")
    }   }

    /// This is our public ID, allowing us to be different from other peers.
    /// This should also be our public key which we'd use for message verification.
    pub fn public_id (&self) -> Result<bits256, String> {
        for pair in &self.secp256k1_key_pair {
            let public = pair.public();  // Compressed public key is going to be 33 bytes.
            // First byte is a prefix, https://davidederosa.com/basic-blockchain-programming/elliptic-curve-keys/.
            return Ok (bits256 {bytes: *array_ref! (public, 1, 32)})
        }
        ERR! ("Public ID is not yet available")
    }

    pub fn gui (&self) -> Option<&str> {
        self.conf["gui"].as_str()
    }
}
impl Default for MmCtx {
    fn default() -> Self {
        Self::with_log_state (LogState::in_memory())
}   }

// We don't want to send `MmCtx` across threads, it will only obstruct the normal use case
// (and might result in undefined behavior if there's a C struct or value in the context that is aliased from the various MM threads).
// Only the `MmArc` is `Send`.
// Also, `MmCtx` not being `Send` allows us to easily keep various C pointers on the context,
// which will likely come useful during the gradual port.
//not-implemented-on-stable// impl !Send for MmCtx {}

pub struct MmArc (pub Arc<MmCtx>);
// NB: Explicit `Send` and `Sync` marks here should become unnecessary later,
// after we finish the initial port and replace the C values with the corresponding Rust alternatives.
unsafe impl Send for MmArc {}
unsafe impl Sync for MmArc {}
impl Clone for MmArc {fn clone (&self) -> MmArc {MmArc (self.0.clone())}}
impl Deref for MmArc {type Target = MmCtx; fn deref (&self) -> &MmCtx {&*self.0}}

#[derive(Clone)]
pub struct MmWeak (Weak<MmCtx>);
// Same as `MmArc`.
unsafe impl Send for MmWeak {}
unsafe impl Sync for MmWeak {}

impl MmWeak {
    pub fn dropped (&self) -> bool {
        self.0.strong_count() == 0
}   }

impl fmt::Debug for MmWeak {
    fn fmt (&self, ft: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let ctx = MmArc::from_weak (self);
        wite! (ft, "MmWeak("
            if let Some (ctx) = ctx {
                match ctx.ffi_handle() {
                    Ok (k) => {(k)}
                    Err (err) => {"err " (err)}
                }
            } else {'-'}
        ')')
}   }

lazy_static! {
    /// A map from a unique context ID to the corresponding MM context, facilitating context access across the FFI boundaries.  
    /// NB: The entries are not removed in order to keep the FFI handlers unique.
    pub static ref MM_CTX_FFI: Mutex<HashMap<u32, MmWeak>> = Mutex::new (HashMap::default());
}

/// Portable core sharing its context with the native helpers.
/// 
/// In the integration tests we're using this to create new native contexts.
#[derive (Serialize, Deserialize)]
struct PortableCtx {
    // Sending the `conf` as a string in order for bencode not to mess with JSON, and for wire readability.
    conf: String,
    secp256k1_key_pair: ByteBuf,
    ffi_handle: Option<u32>
}

#[derive (Serialize, Deserialize, Debug)]
struct NativeCtx {
    ffi_handle: u32
}

impl MmArc {
    /// Unique context identifier, allowing us to more easily pass the context through the FFI boundaries.
    pub fn ffi_handle (&self) -> Result<u32, String> {
        let mut mm_ctx_ffi = try_s! (MM_CTX_FFI.lock());
        for &have in &self.ffi_handle {return Ok (have)}
        let mut tries = 0;
        let mut rng = small_rng();
        loop {
            if tries > 999 {panic! ("MmArc] out of RIDs")} else {tries += 1}
            let rid: u32 = rng.gen();
            if rid == 0 {continue}
            match mm_ctx_ffi.entry (rid) {
                Entry::Occupied (_) => continue,  // Try another ID.
                Entry::Vacant (ve) => {
                    ve.insert (self.weak());
                    try_s! (self.ffi_handle.pin (rid));
                    return Ok (rid)
    }   }   }   }

    #[cfg(not(feature = "native"))]
    pub async fn send_to_helpers (&self) -> Result<(), String> {
        use crate::helperᶜ;

        let ctxʷ = PortableCtx {
            conf: try_s! (json::to_string (&self.conf)),
            secp256k1_key_pair: match self.secp256k1_key_pair.as_option() {
                Some (k) => ByteBuf::from (k.private().layout()),
                None => ByteBuf::new()
            },
            ffi_handle: self.ffi_handle.as_option().copied()
        };
        let ctxᵇ = try_s! (bencode (&ctxʷ));
        let hr = try_s! (helperᶜ ("ctx2helpers", ctxᵇ) .await);

        // Remember the context ID used by the native helpers in order to simplify consecutive syncs.
        let ctxⁿ: NativeCtx = try_s! (bdecode (&hr));
        if let Some (ffi_handle) = self.ffi_handle.as_option().copied() {
            if ffi_handle != ctxⁿ.ffi_handle {return ERR! ("ffi_handle mismatch")}
        } else {
            try_s! (self.ffi_handle.pin (ctxⁿ.ffi_handle));
        }

        Ok(())
    }

    /// Tries getting access to the MM context.
    /// Fails if an invalid MM context handler is passed (no such context or dropped context).
    pub fn from_ffi_handle (ffi_handle: u32) -> Result<MmArc, String> {
        if ffi_handle == 0 {return ERR! ("MmArc] Zeroed ffi_handle")}
        let mm_ctx_ffi = try_s! (MM_CTX_FFI.lock());
        match mm_ctx_ffi.get (&ffi_handle) {
            Some (weak) => match MmArc::from_weak (weak) {
                Some (ctx) => Ok (ctx),
                None => ERR! ("MmArc] ffi_handle {} is dead", ffi_handle)
            },
            None => ERR! ("MmArc] ffi_handle {} does not exists", ffi_handle)
    }   }

    /// Generates a weak link, to track the context without prolonging its life.
    pub fn weak (&self) -> MmWeak {
        MmWeak (Arc::downgrade (&self.0))
    }

    /// Tries to obtain the MM context from the weak link.  
    pub fn from_weak (weak: &MmWeak) -> Option<MmArc> {
        weak.0.upgrade().map (|arc| MmArc (arc))
    }
}

/// Receives a subset of a portable context in order to recreate a native copy of it.
/// Can be invoked with the same context multiple times, synchronizing some of the fields.
/// As of now we're expecting a one-to-one pairing between the portable and the native versions of MM
/// so the uniqueness of the `ffi_handle` is not a concern yet.
#[cfg(feature = "native")]
pub async fn ctx2helpers (main_ctx: MmArc, req: Bytes) -> Result<Vec<u8>, String> {
    let ctxʷ: PortableCtx = try_s! (bdecode (&req));
    let private = try_s! (Private::from_layout (&ctxʷ.secp256k1_key_pair[..]));
    let main_key = try_s! (main_ctx.secp256k1_key_pair.as_option().ok_or ("No key"));

    if *main_key.private() == private {
        // We have a match with the primary native context, the one configured on the command line.
        let res = try_s! (bencode (&NativeCtx {
            ffi_handle: try_s! (main_ctx.ffi_handle())
        }));
        return Ok (res)
    }

    if let Some (ffi_handle) = ctxʷ.ffi_handle {
        if let Ok (ctx) = MmArc::from_ffi_handle (ffi_handle) {
            let key = try_s! (ctx.secp256k1_key_pair.as_option().ok_or ("No key"));
            if *key.private() != private {return ERR! ("key mismatch")}
            let res = try_s! (bencode (&NativeCtx {
                ffi_handle: try_s! (ctx.ffi_handle())
            }));
            return Ok (res)
    }   }

    // Create a native copy of the portable context.

    let pair: Option<KeyPair> = if ctxʷ.secp256k1_key_pair.is_empty() {None} else {
        let private = try_s! (Private::from_layout (&ctxʷ.secp256k1_key_pair[..]));
        Some (try_s! (KeyPair::from_private (private)))
    };

    let ctx = MmCtx {
        conf: try_s! (json::from_str (&ctxʷ.conf)),
        secp256k1_key_pair: pair.into(),
        ffi_handle: ctxʷ.ffi_handle.into(),
        ..MmCtx::with_log_state (LogState::in_memory())
    };
    let ctx = MmArc (Arc::new (ctx));
    if let Some (ffi_handle) = ctxʷ.ffi_handle {
        let mut ctx_ffi = try_s! (MM_CTX_FFI.lock());
        if ctx_ffi.contains_key (&ffi_handle) {return ERR! ("ID race")}
        ctx_ffi.insert (ffi_handle, ctx.weak());
    }
    let res = try_s! (bencode (&NativeCtx {
        ffi_handle: try_s! (ctx.ffi_handle())
    }));
    Arc::into_raw (ctx.0);  // Leak.

    Ok (res)
}

/// Helps getting a crate context from a corresponding `MmCtx` field.
/// 
/// * `ctx_field` - A dedicated crate context field in `MmCtx`, such as the `MmCtx::portfolio_ctx`.
/// * `constructor` - Generates the initial crate context.
pub fn from_ctx<T, C> (ctx_field: &Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>, constructor: C) -> Result<Arc<T>, String>
where C: FnOnce()->Result<T, String>, T: 'static + Send + Sync {
    let mut ctx_field = try_s! (ctx_field.lock());
    if let Some (ref ctx) = *ctx_field {
        let ctx: Arc<T> = match ctx.clone().downcast() {
            Ok (p) => p,
            Err (_) => return ERR! ("Error casting the context field")
        };
        return Ok (ctx)
    }
    let arc = Arc::new (try_s! (constructor()));
    *ctx_field = Some (arc.clone());
    return Ok (arc)
}

#[derive(Default)]
pub struct MmCtxBuilder {
    conf: Option<Json>,
    key_pair: Option<KeyPair>
}

impl MmCtxBuilder {
    pub fn new() -> Self {
        MmCtxBuilder::default()
    }

    pub fn with_conf(mut self, conf: Json) -> Self {
        self.conf = Some (conf);
        self
    }

    pub fn with_secp256k1_key_pair(mut self, key_pair: KeyPair) -> Self {
        self.key_pair = Some (key_pair);
        self
    }

    pub fn into_mm_arc(self) -> MmArc {
        // NB: We avoid recreating LogState
        // in order not to interfere with the integration tests checking LogState drop on shutdown.
        let log = if let Some (ref conf) = self.conf {LogState::mm (conf)} else {LogState::in_memory()};
        let mut ctx = MmCtx::with_log_state (log);
        if let Some (conf) = self.conf {
            ctx.conf = conf
        }

        if let Some (key_pair) = self.key_pair {
            unwrap! (ctx.rmd160.pin (key_pair.public().address_hash()));
            unwrap! (ctx.secp256k1_key_pair.pin (key_pair));
        }

        MmArc (Arc::new (ctx))
    }
}

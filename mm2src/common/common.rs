//! A common dependency for subcrates.
//! 
//!                   common
//!                     ^
//!                     |
//!     subcrate A   ---+---   subcrate B
//!         ^                      ^
//!         |                      |
//!         +-----------+----------+
//!                     |
//!                   binary

#![feature(non_ascii_idents)]

#[macro_use] extern crate duct;
#[macro_use] extern crate fomat_macros;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate unwrap;

/// Fills a C character array with a zero-terminated C string,
/// returning an error if the string is too large.
#[macro_export]
#[allow(unused_unsafe)]
macro_rules! safecopy {
    ($to: expr, $format: expr, $($args: tt)+) => {{
        use ::std::io::Write;
        let to = &mut $to[..];
        let to: &mut [u8] = unsafe {::std::mem::transmute (to)};  // c_char to Rust.
        let mut wr = ::std::io::Cursor::new (to);
        write! (&mut wr, concat! ($format, "\0"), $($args)+)
    }}
}

/// Implements a `From` for `enum` with a variant name matching the name of the type stored.
/// 
/// This is helpful as a workaround for the lack of datasort refinements.  
/// And also as a simpler alternative to `enum_dispatch` and `enum_derive`.
/// 
///     enum Color {Red (Red)}
///     ifrom! (Color, Red);
#[macro_export]
macro_rules! ifrom {
    ($enum: ident, $id: ident) => {
        impl From<$id> for $enum {
            fn from (t: $id) -> $enum {
                $enum::$id (t)
}   }   }   }

#[macro_use]
pub mod jsonrpc_client;
#[macro_use]
pub mod log;

pub mod for_c;
pub mod for_tests;
pub mod custom_futures;
pub mod iguana_utils;
pub mod lp_privkey;
pub mod mm_ctx;
pub mod ser;

use crossbeam::{channel};
use futures::{future, Async, Future, Poll};
use futures::sync::oneshot::{self, Receiver};
use futures::task::Task;
use gstuff::{any_to_str, duration_to_float, now_float};
use hex::FromHex;
use hyper::{Body, Client, Request, Response, StatusCode, HeaderMap};
use hyper::client::HttpConnector;
use hyper::header::{ HeaderValue, CONTENT_TYPE };
use hyper::rt::Stream;
use hyper_rustls::HttpsConnector;
use libc::{c_char, c_void, malloc, free};
use serde_json::{self as json, Value as Json};
use std::env::args;
use std::fmt;
use std::fs;
use std::ffi::{CStr, CString};
use std::intrinsics::copy;
use std::io::{Write};
use std::mem::{forget, size_of, uninitialized, zeroed};
use std::path::{Path};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::abort;
use std::ptr::{null_mut, read_volatile};
use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::str;
use tokio_core::reactor::Remote;

// Make sure we're linking the eth-secp256k1 in for it is used in the MM1 C code.
use secp256k1::Secp256k1;
pub extern fn _we_are_using_secp256k1() -> Secp256k1 {Secp256k1::new()}

#[allow(dead_code,non_upper_case_globals,non_camel_case_types,non_snake_case)]
pub mod lp {include! (concat! (env! ("OUT_DIR"), "/c_headers/LP_include.rs"));}
pub use self::lp::{_bits256 as bits256};

#[allow(dead_code,non_upper_case_globals,non_camel_case_types,non_snake_case)]
pub mod os {include! (concat! (env! ("OUT_DIR"), "/c_headers/OS_portable.rs"));}

pub const MM_VERSION: &'static str = env! ("MM_VERSION");

pub const SATOSHIS: u64 = 100000000;

/// Converts u64 satoshis to f64
pub fn sat_to_f(sat: u64) -> f64 { sat as f64 / SATOSHIS as f64 }

/// Created by `void *bitcoin_ctx()`.
pub enum BitcoinCtx {}

extern "C" {
    pub fn bitcoin_ctx() -> *mut BitcoinCtx;
    fn bitcoin_ctx_destroy (ctx: *mut BitcoinCtx);
    pub fn bitcoin_priv2wif (symbol: *const u8, wiftaddr: u8, wifstr: *mut c_char, privkey: bits256, addrtype: u8) -> i32;
    fn bits256_str (hexstr: *mut u8, x: bits256) -> *const c_char;
}

impl fmt::Display for bits256 {
    fn fmt (&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut buf: [u8; 65] = unsafe {zeroed()};
        let cs = unsafe {bits256_str (buf.as_mut_ptr(), *self)};
        let hex = unwrap! (unsafe {CStr::from_ptr (cs)} .to_str());
        f.write_str (hex)
}   }

impl fmt::Debug for bits256 {
    fn fmt (&self, f: &mut fmt::Formatter) -> fmt::Result {
        (self as &fmt::Display) .fmt (f)
}   }

impl std::cmp::PartialEq for bits256 {
    /// Should be preferred to `bits256_cmp`.
    fn eq (&self, other: &bits256) -> bool {
        unsafe {
            self.ulongs[0] == other.ulongs[0] &&
            self.ulongs[1] == other.ulongs[1] &&
            self.ulongs[2] == other.ulongs[2] &&
            self.ulongs[3] == other.ulongs[3]
}   }   }
impl std::cmp::Eq for bits256 {}

impl std::hash::Hash for bits256 {
    fn hash<H: std::hash::Hasher> (&self, state: &mut H) {
        unsafe {
            self.ulongs[0].hash (state);
            self.ulongs[1].hash (state);
            self.ulongs[2].hash (state);
            self.ulongs[3].hash (state);
}   }   }

impl bits256 {
    /// Returns true if the hash is not zero.  
    /// Port of `#define bits256_nonz`.
    pub fn nonz (&self) -> bool {
        unsafe {self.ulongs[0] != 0 || self.ulongs[1] != 0 || self.ulongs[2] != 0 || self.ulongs[3] != 0}
    }
}

/// Decodes a HEX string into a 32-bytes array.  
/// But only if the HEX string is 64 characters long, returning a zeroed array otherwise.  
/// (Use `fn nonz` to check if the array is zeroed).  
/// A port of cJSON.c/jbits256.
pub fn jbits256 (json: &Json) -> Result<bits256, String> {
    let mut hash: bits256 = unsafe {zeroed()};
    if let Some (hex) = json.as_str() {
        if hex.len() == 64 {
            //try_s! (::common::iguana_utils::decode_hex (unsafe {&mut hash.bytes[..]}, hex.as_bytes()));
            let bytes: [u8; 32] = try_s! (FromHex::from_hex (hex));
            unsafe {hash.bytes.copy_from_slice (&bytes)}
        }
    }
    Ok (hash)
}

/// [functional]
pub fn bitcoin_address (coin: &str, addrtype: u8, rmd160: [u8; 20usize]) -> Result<String, String> {
    let coinaddr: [u8; 64] = unsafe {zeroed()};
    let coin = try_s! (CString::new (coin));
    unsafe {lp::bitcoin_address (coin.as_ptr() as *mut c_char, coinaddr.as_ptr() as *mut c_char, 0, addrtype, rmd160.as_ptr() as *mut u8, 20)};
    Ok (try_s! (try_s! (CStr::from_bytes_with_nul (&coinaddr[..])) .to_str()) .to_string())
}

/// A safer version of `HASH_ITER` over `iguana_info` coins from `for_c::COINS`.
pub fn coins_iter (cb: &mut dyn FnMut (*mut lp::iguana_info) -> Result<(), String>) -> Result<(), String> {
    let coins = try_s! (for_c::COINS.lock());
    let mut iis = Vec::with_capacity (coins.len());
    for (_ticker, ii) in coins.iter() {iis.push (ii.0)}
    drop (coins);  // Unlock before callbacks, avoiding possibility of deadlocks and poisoning.

    for ii in iis {try_s! (cb (ii))}

    Ok(())
}

pub const SATOSHIDEN: i64 = 100000000;
pub fn dstr (x: i64, decimals: u8) -> f64 {x as f64 / 10.0_f64.powf(decimals as f64)}

/// Apparently helps to workaround `double` fluctuations occuring on *certain* systems.
/// cf. https://stackoverflow.com/questions/19804472/double-randomly-adds-0-000000000000001.
/// Not sure it's needed in Rust, the floating point operations should be determenistic here,
/// but better safe than sorry.
pub const SMALLVAL: f64 = 0.000000000000001;  // 1e-15f64

/// RAII and MT wrapper for `cJSON`.
pub struct CJSON (pub *mut lp::cJSON);
impl CJSON {
    pub fn from_zero_terminated (json: *const c_char) -> Result<CJSON, String> {
        lazy_static! {static ref LOCK: Mutex<()> = Mutex::new(());}
        let _lock = try_s! (LOCK.lock());  // Probably need a lock to access the error singleton.
        let c_json = unsafe {lp::cJSON_Parse (json)};
        if c_json == null_mut() {
            let err = unsafe {lp::cJSON_GetErrorPtr()};
            let err = try_s! (unsafe {CStr::from_ptr (err)} .to_str());
            ERR! ("Can't parse JSON, error: {}", err)
        } else {
            Ok (CJSON (c_json))
        }
    }
    pub fn from_str (json: &str) -> Result<CJSON, String> {
        let cs = try_s! (CString::new (json));
        CJSON::from_zero_terminated (cs.as_ptr())
    }
    pub fn new () -> CJSON {
        unwrap! (CJSON::from_str (""))
    }
}
impl Drop for CJSON {
    fn drop (&mut self) {
        unsafe {lp::cJSON_Delete (self.0)}
        self.0 = null_mut()
    }
}
unsafe impl Send for CJSON {}

/// Helps sharing a string slice with C code by allocating a zero-terminated string with the C standard library allocator.
/// 
/// The difference from `CString` is that the memory is then *owned* by the C code instead of being temporarily borrowed,
/// that is it doesn't need to be recycled in Rust.
/// Plus we don't check the slice for zeroes, most of our code doesn't need that extra check.
pub fn str_to_malloc (s: &str) -> *mut c_char {
    slice_to_malloc (s.as_bytes()) as *mut c_char
}

/// Helps sharing a byte slice with C code by allocating a zero-terminated string with the C standard library allocator.
pub fn slice_to_malloc (bytes: &[u8]) -> *mut u8 {unsafe {
    let buf = malloc (bytes.len() + 1) as *mut u8;
    copy (bytes.as_ptr(), buf, bytes.len());
    *buf.offset (bytes.len() as isize) = 0;
    buf
}}

/// Converts *mut c_char to Rust String
/// Doesn't free the allocated memory
/// It's responsibility of the caller to free the memory when required
/// Returns error in case of null pointer input
pub fn c_char_to_string(ptr: *mut c_char) -> Result<String, String> { unsafe {
    if !ptr.is_null() {
        let res_str = try_s!(CStr::from_ptr(ptr).to_str());
        let res_str = String::from(res_str);
        Ok(res_str)
    } else {
        ERR!("Tried to convert null pointer to Rust String!")
    }
}}

/// Frees C raw pointer
/// Does nothing in case of null pointer input
pub fn free_c_ptr(ptr: *mut c_void) { unsafe {
    if !ptr.is_null() {
        free(ptr as *mut libc::c_void);
    }
}}

/// Use the value, preventing the compiler and linker from optimizing it away.
pub fn black_box<T> (v: T) -> T {
    // https://github.com/rust-lang/rfcs/issues/1484#issuecomment-240853111
    let ret = unsafe {read_volatile (&v)};
    forget (v);
    ret
}

// https://doc.rust-lang.org/nightly/std/convert/fn.identity.html  
// Waiting for https://github.com/rust-lang/rust/issues/53500.
pub const fn identity<T> (v: T) -> T {v}

/// Attempts to remove the `Path` on `drop`.
#[derive(Debug)]
pub struct RaiiRm<'a> (pub &'a Path);
impl<'a> AsRef<Path> for RaiiRm<'a> {
    fn as_ref (&self) -> &Path {
        self.0
    }
}
impl<'a> Drop for RaiiRm<'a> {
    fn drop (&mut self) {
        let _ = fs::remove_file (self);
    }
}

/// Using a static buffer in order to minimize the chance of heap and stack allocations in the signal handler.
fn trace_buf() -> MutexGuard<'static, [u8; 256]> {
    lazy_static! {static ref TRACE_BUF: Mutex<[u8; 256]> = Mutex::new (unsafe {uninitialized()});}
    unwrap! (TRACE_BUF.lock())
}

fn trace_name_buf() -> MutexGuard<'static, [u8; 128]> {
    lazy_static! {static ref TRACE_NAME_BUF: Mutex<[u8; 128]> = Mutex::new (unsafe {uninitialized()});}
    unwrap! (TRACE_NAME_BUF.lock())
}

/// Formats a stack frame.
/// Some common and less than useful frames are skipped.
pub fn stack_trace_frame (buf: &mut Write, symbol: &backtrace::Symbol) {
    let filename = match symbol.filename() {Some (path) => path, None => return};
    let filename = match filename.components().rev().next() {Some (c) => c.as_os_str().to_string_lossy(), None => return};
    let lineno = match symbol.lineno() {Some (lineno) => lineno, None => return};
    let name = match symbol.name() {Some (name) => name, None => return};
    let mut name_buf = trace_name_buf();
    let name = gstring! (name_buf, {
        let _ = write! (name_buf, "{}", name);  // NB: `fmt` is different from `SymbolName::as_str`.
    });

    // Skip common and less than informative frames.

    if name == "mm2::crash_reports::rust_seh_handler" {return}
    if name == "veh_exception_filter" {return}
    if name == "common::stack_trace" {return}
    if name == "common::log_stacktrace" {return}
    if name == "__scrt_common_main_seh" {return}  // Super-main on Windows.

    if filename == "boxed.rs" {return}
    if filename == "panic.rs" {return}

    // Alphanumerically sorted on first letter.
    if name.starts_with ("alloc::") {return}
    if name.starts_with ("backtrace::") {return}
    if name.starts_with ("common::stack_trace") {return}
    if name.starts_with ("futures::") {return}
    if name.starts_with ("hyper::") {return}
    if name.starts_with ("mm2::crash_reports::signal_handler") {return}
    if name.starts_with ("panic_unwind::") {return}
    if name.starts_with ("std::") {return}
    if name.starts_with ("scoped_tls::") {return}
    if name.starts_with ("tokio::") {return}
    if name.starts_with ("tokio_core::") {return}
    if name.starts_with ("tokio_reactor::") {return}
    if name.starts_with ("tokio_executor::") {return}
    if name.starts_with ("tokio_timer::") {return}

    let _ = writeln! (buf, "  {}:{}] {}", filename, lineno, name);
}

/// Generates a string with the current stack trace.
/// 
/// To get a simple stack trace:
/// 
///     let mut trace = String::with_capacity (4096);
///     stack_trace (&mut stack_trace_frame, &mut |l| trace.push_str (l));
/// 
/// * `format` - Generates the string representation of a frame.
/// * `output` - Function used to print the stack trace.
///              Printing immediately, without buffering, should make the tracing somewhat more reliable.
pub fn stack_trace (format: &mut dyn FnMut (&mut Write, &backtrace::Symbol), output: &mut dyn FnMut (&str)) {
    backtrace::trace (|frame| {
        backtrace::resolve (frame.ip(), |symbol| {
            let mut trace_buf = trace_buf();
            let trace = gstring! (trace_buf, {
              format (trace_buf, symbol);
            });
            output (trace);
        });
        true
    });
}

/// Tries to detect if we're running under a test, allowing us to be lazy and *delay* some costly operations.
/// 
/// Note that the code SHOULD behave uniformely regardless of where it's invoked from
/// (nondeterminism breaks POLA and we don't know how the code will be used in the future)
/// but in certain cases we have a leeway of adjusting to being run from a test
/// without breaking any invariants or expectations.
/// For instance, DHT might take unknown time to initialize, and by delaying this initialization in the tests
/// we can avoid the unnecessary overhead of DHT initializaion and destruction while maintaining the contract.
pub fn is_a_test_drill() -> bool {
    let mut trace = String::with_capacity (1024);
    stack_trace (
        &mut |mut fwr, sym| {if let Some (name) = sym.name() {let _ = witeln! (fwr, (name));}},
        &mut |tr| {trace.push_str (tr)});

    if trace.contains ("\nmm2::main\n") || trace.contains ("\nmm2::run_lp_main\n") {return false}

    if let Some (executable) = args().next() {
        if executable.ends_with (r"\mm2.exe") {return false}
        if executable.ends_with ("/mm2") {return false}
    }

    true
}

fn start_core_thread() -> Remote {
    let (tx, rx) = oneshot::channel();
    unwrap! (thread::Builder::new().name ("CORE".into()) .spawn (move || {
        if let Err (err) = catch_unwind (AssertUnwindSafe (move || {
            let mut core = unwrap! (tokio_core::reactor::Core::new(), "!core");
            unwrap! (tx.send (core.remote()), "Can't send Remote.");
            loop {core.turn (None)}
        })) {
            log! ({"CORE panic! {:?}", any_to_str (&*err)});
            abort()
        }
    }), "!spawn");
    let core: Remote = unwrap! (rx.wait(), "!wait");
    core
}

lazy_static! {
    /// Shared asynchronous reactor.
    pub static ref CORE: Remote = start_core_thread();
}

/// With a shared reactor drives the future `f` to completion.
///
/// NB: This function is only useful if you need to get the results of the execution.
/// If the results are not necessary then a future can be scheduled directly on the reactor:
///
///     CORE.spawn (|_| f);
pub fn drive<F, R, E> (f: F) -> Receiver<Result<R, E>> where
F: Future<Item=R, Error=E> + Send + 'static,
R: Send + 'static,
E: Send + 'static {
    let (sx, rx) = oneshot::channel();
    CORE.spawn (move |_handle| {
        f.then (move |fr: Result<R, E>| -> Result<(),()> {
            let _ = sx.send (fr);
            Ok(())
        })
    });
    rx
}

/// With a shared reactor drives the future `f` to completion.
///
/// Similar to `fn drive`, but returns a stringified error,
/// allowing us to collapse the `Receiver` and return the `R` directly.
pub fn drive_s<F, R, E> (f: F) -> impl Future<Item=R, Error=String> where
F: Future<Item=R, Error=E> + Send + 'static,
R: Send + 'static,
E: fmt::Display + Send + 'static {
    drive (f) .then (move |r| -> Result<R, String> {
        let r = try_s! (r);  // Peel the `Receiver`.
        let r = try_s! (r);  // `E` to `String`.
        Ok (r)
    })
}

/// Finishes with the "timeout" error if the underlying future isn't ready withing the given timeframe.
/// 
/// NB: Tokio timers (in `tokio::timer`) only seem to work under the Tokio runtime,
/// which is unfortunate as we want the different futures executed on the different reactors
/// depending on how much they're I/O-bound, CPU-bound or blocking.
/// Unlike the Tokio timers this `Timeout` implementation works with any reactor.
/// Another option to consider is https://github.com/alexcrichton/futures-timer.
/// P.S. The older `0.1` version of the `tokio::timer` might work NP, it works in other parts of our code.
///      The new version, on the other hand, requires the Tokio runtime (https://tokio.rs/blog/2018-03-timers/).
/// TODO: Use futures-timer instead.
pub struct Timeout<R> {
    fut: Box<Future<Item=R, Error=String>>,
    started: f64,
    timeout: f64,
    monitor: Option<JoinHandle<()>>
}
impl<R> Future for Timeout<R> {
    type Item = R;
    type Error = String;
    fn poll (&mut self) -> Poll<R, String> {
        match self.fut.poll() {
            Err (err) => Err (err),
            Ok (Async::Ready (r)) => Ok (Async::Ready (r)),
            Ok (Async::NotReady) => {
                let now = now_float();
                if now >= self.started + self.timeout {
                    Err (format! ("timeout ({:.1} > {:.1})", now - self.started, self.timeout))
                } else {
                    // Start waking up this future until it has a chance to timeout.
                    // For now it's just a basic separate thread. Will probably optimize later.
                    if self.monitor.is_none() {
                        let task = futures::task::current();
                        let deadline = self.started + self.timeout;
                        self.monitor = Some (unwrap! (std::thread::Builder::new().name ("timeout monitor".into()) .spawn (move || {
                            loop {
                                std::thread::sleep (Duration::from_secs (1));
                                task.notify();
                                if now_float() > deadline + 2. {break}
                            }
                        })));
                    }
                    Ok (Async::NotReady)
}   }   }   }   }
impl<R> Timeout<R> {
    pub fn new (fut: Box<Future<Item=R, Error=String>>, timeout: Duration) -> Timeout<R> {
        Timeout {
            fut: fut,
            started: now_float(),
            timeout: duration_to_float (timeout),
            monitor: None
}   }   }

unsafe impl<R> Send for Timeout<R> {}

/// Initialize the crate.
pub fn init() {
    // Pre-allocate the stack trace buffer in order to avoid allocating it from a signal handler.
    black_box (&*trace_buf());
    black_box (&*trace_name_buf());
}

lazy_static! {
    /// NB: With a shared client there is a possibility that keep-alive connections will be reused.
    pub static ref HYPER: Client<HttpsConnector<HttpConnector>> = {
        let dns_threads = 2;
        let https = HttpsConnector::new (dns_threads);
        let client = Client::builder()
            .executor (CORE.clone())
            // Hyper had a lot of Keep-Alive bugs over the years and I suspect
            // that with the shared client we might be getting errno 10054
            // due to a closed Keep-Alive connection mismanagement.
            // (To solve this problem Hyper should proactively close the Keep-Alive
            // connections after a configurable amount of time has passed since
            // their creation, thus saving us from trying to use the connections
            // closed on the other side. I wonder if we can implement this strategy
            // ourselves with a custom connector or something).
            // Performance of Keep-Alive in the Hyper client is questionable as well,
            // should measure it on a case-by-case basis when we need it.
            .keep_alive (false)
            .build (https);
        client
    };
}

type SlurpFut = Box<Future<Item=(StatusCode, HeaderMap, Vec<u8>), Error=String> + Send + 'static>;

/// Executes a Hyper request, returning the response status, headers and body.
pub fn slurp_req (request: Request<Body>) -> SlurpFut {
    let uri = fomat! ((request.uri()));
    let request_f = HYPER.request (request);
    let response_f = request_f.then (move |res| -> SlurpFut {
        // Can fail with:
        // "an IO error occurred: An existing connection was forcibly closed by the remote host. (os error 10054)" (on Windows)
        // "an error occurred trying to connect: No connection could be made because the target machine actively refused it. (os error 10061)"
        // "an error occurred trying to connect: Connection refused (os error 111)"
        let res = match res {
            Ok (r) => r,
            Err (err) => return Box::new (futures::future::err (
                ERRL! ("Error accessing '{}': {}", uri, err)))
        };
        let status = res.status();
        let headers = res.headers().clone();
        let body_f = res.into_body().concat2();
        let combined_f = body_f.then (move |body| -> Result<(StatusCode, HeaderMap, Vec<u8>), String> {
            let body = try_s! (body);
            Ok ((status, headers, body.to_vec()))
        });
        Box::new (combined_f)
    });
    Box::new (drive_s (response_f))
}

/// Executes a GET request, returning the response status, headers and body.
pub fn slurp_url (url: &str) -> SlurpFut {
    slurp_req (try_fus! (Request::builder().uri (url) .body (Body::empty())))
}

#[test]
fn test_slurp_req() {
    let (status, _headers, _body) = unwrap! (slurp_url ("https://httpbin.org/get") .wait());
    assert! (status.is_success());
}

/// Fetch URL by HTTPS and parse JSON response
pub fn fetch_json<T>(url: &str) -> Box<Future<Item=T, Error=String>>
where T: serde::de::DeserializeOwned + Send + 'static {
    Box::new(slurp_url(url).and_then(|result| {
        // try to parse as json with serde_json
        let result = try_s!(serde_json::from_slice(&result.2));

        Ok(result)
    }))
}

/// Send POST JSON HTTPS request and parse response
pub fn post_json<T>(url: &str, json: String) -> Box<Future<Item=T, Error=String>>
where T: serde::de::DeserializeOwned + Send + 'static {
    let request = try_fus!(Request::builder()
        .method("POST")
        .uri(url)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json")
        )
        .body(json.into())
    );

    Box::new(slurp_req(request).and_then(|result| {
        // try to parse as json with serde_json
        let result = try_s!(serde_json::from_slice(&result.2));

        Ok(result)
    }))
}

/// RPC response, returned by the RPC handlers.  
/// NB: By default the future is executed on the shared asynchronous reactor (`CORE`),
/// the handler is responsible for spawning the future on another reactor if it doesn't fit the `CORE` well.
pub type HyRes = Box<Future<Item=Response<Body>, Error=String> + Send>;

/// Returns a JSON error HyRes on a failure.
#[macro_export]
macro_rules! try_h {
    ($e: expr) => {
        match $e {
            Ok (ok) => ok,
            Err (err) => {return $crate::rpc_err_response (500, &ERRL! ("{}", err))}
        }
    }
}

/// Wraps a JSON string into the `HyRes` RPC response future.
pub fn rpc_response<T>(status: u16, body: T) -> HyRes where Body: From<T> {
    Box::new (
        match Response::builder()
            .status(status)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(body)) {
                Ok (r) => future::ok::<Response<Body>, String> (r),
                Err (err) => future::err::<Response<Body>, String> (ERRL! ("{}", err))
            }
    )
}

/// Converts the given `err` message into the `{error: $err}` JSON string.
pub fn err_to_rpc_json_string(err: &str) -> String {
    #[derive(Serialize)]
    struct ErrResponse {
        error: String,
    }

    let err = ErrResponse {
        error: err.to_owned(),
    };
    json::to_string(&err).unwrap()
}

/// Returns the `{error: $msg}` JSON response with the given HTTP `status`.  
/// Also logs the error (if possible).
pub fn rpc_err_response(status: u16, msg: &str) -> HyRes {
    // TODO: Like in most other places, we should check for a thread-local access to the proper log here.
    // Might be a good idea to use emoji too, like "🤒" or "🤐" or "😕".
    log! ({"RPC error response: {}", msg});

    rpc_response(status, err_to_rpc_json_string(msg))
}

/// A closure that would (re)start a `Future` to synchronize with an external resource in `RefreshedExternalResource`.
type ExternalResourceSync<R> = Box<Fn()->Box<Future<Item=R,Error=String> + Send + 'static> + Send + 'static>;

/// Memory space accessible to the `Future` tail spawned by the `RefreshedExternalResource`.
struct RerShelf<R: Send + 'static> {
    /// The time when the `Future` generated by `sync` has filled this shell.
    time: f64,
    /// Results of the `sync`-generated `Future`.
    result: Result<R, String>
}

/// Often we have an external resource that we need a fresh copy of.
/// (Or the other way around, when there is an external resource that we need to periodically update or synchronize with).
/// Particular property of such resources is that they might be unavailable,
/// might be slow due to resource overload or network congestion,
/// need to be resynchronized periodically
/// while being nice to the resource by maintaining rate limits.
///
/// Some of these resources are naturally singleton.
/// For exampe, we have only one "bittrex.com" and we need not multiple copies of its market data withing the process.
///
/// This helper here will organize the handling of such synchronization, periodically starting the synchronization `Future`,
/// restarting it on timeout, maintaining rate limits.
pub struct RefreshedExternalResource<R: Send + 'static> {
    sync: Mutex<ExternalResourceSync<R>>,
    /// Rate limit in the form of the desired number of seconds between the syncs.
    every_n_sec: f64,
    /// Start a new `Future` and drop the old one if it fails to finish after this number of seconds.
    timeout_sec: f64,
    /// The time (in f64 seconds) when we last (re)started the `sync`.
    /// We want `AtomicU64` but it isn't yet stable.
    last_start: AtomicUsize,
    shelf: Arc<Mutex<Option<RerShelf<R>>>>,
    /// The `Future`s interested in the next update.  
    /// When there is an updated the `Task::notify` gets invoked once and then the `Task` is removed from the `listeners` list.
    listeners: Arc<Mutex<Vec<Task>>>
}
impl<R: Send + 'static> RefreshedExternalResource<R> {
    /// New instance of the external resource tracker.
    ///
    /// * `every_n_sec` - Desired number of seconds between the syncs.
    /// * `timeout_sec` - Start a new `sync` and drop the old `Future` if it fails to finish after this number of seconds.
    ///                   Automatically bumped to be at least `every_n_sec` large.
    /// * `sync` - Generates the `Future` that should synchronize with the external resource in background.
    ///            Note that we'll tail the `Future`, polling the tail from the shared asynchronous reactor;
    ///            *spawn* the `Future` onto a different reactor if the shared asynchronous reactor is not the best option.
    pub fn new (every_n_sec: f64, timeout_sec: f64, sync: ExternalResourceSync<R>) -> RefreshedExternalResource<R> {
        assert_eq! (size_of::<usize>(), 8);
        RefreshedExternalResource {
            sync: Mutex::new (sync),
            every_n_sec,
            timeout_sec: timeout_sec .max (every_n_sec),
            last_start: AtomicUsize::new (0f64.to_bits() as usize),
            shelf: Arc::new (Mutex::new (None)),
            listeners: Arc::new (Mutex::new (Vec::new()))
        }
    }

    pub fn add_listeners (&self, mut tasks: Vec<Task>) -> Result<(), String> {
        let mut listeners = try_s! (self.listeners.lock());
        listeners.append (&mut tasks);
        Ok(())
    }

    /// Performs the maintenance operations necessary to periodically refresh the resource.
    pub fn tick (&self) -> Result<(), String> {
        let now = now_float();
        let last_finish = match * try_s! (self.shelf.lock()) {Some (ref rer_shelf) => rer_shelf.time, None => 0.};
        let last_start = f64::from_bits (self.last_start.load (Ordering::Relaxed) as u64);

        if now - last_start > self.timeout_sec || (last_finish > last_start && now - last_start > self.every_n_sec) {
            self.last_start.store (now.to_bits() as usize, Ordering::Relaxed);
            let sync = try_s! (self.sync.lock());
            let f = (*sync)();
            let shelf_tx = self.shelf.clone();
            let listeners = self.listeners.clone();
            let f = f.then (move |result| -> Result<(), ()> {
                let mut shelf = match shelf_tx.lock() {Ok (l) => l, Err (err) => {
                    log! ({"RefreshedExternalResource::tick] Can't lock the shelf: {}", err});
                    return Err(())
                }};
                let shelf_time = match *shelf {Some (ref r) => r.time, None => 0.};
                if now > shelf_time {  // This check prevents out-of-order shelf updates.
                    *shelf = Some (RerShelf {
                        time: now_float(),
                        result
                    });
                    drop (shelf);  // Don't hold the lock unnecessarily.
                    {
                        let mut listeners = match listeners.lock() {Ok (l) => l, Err (err) => {
                            log! ({"RefreshedExternalResource::tick] Can't lock the listeners: {}", err});
                            return Err(())
                        }};
                        for task in listeners.drain (..) {task.notify()}
                    }
                }
                Ok(())
            });
            CORE.spawn (move |_| f);  // Polls `f` in background.
        }

        Ok(())
    }

    /// The time, in seconds since UNIX epoch, when the refresh `Future` resolved.
    pub fn last_finish (&self) -> Result<f64, String> {
        Ok (match * try_s! (self.shelf.lock()) {
            Some (ref rer_shelf) => rer_shelf.time,
            None => 0.
        })
    }

    pub fn with_result<V, F: FnMut (Option<&Result<R, String>>) -> Result<V, String>> (&self, mut cb: F) -> Result<V, String> {
        let shelf = try_s! (self.shelf.lock());
        match *shelf {
            Some (ref rer_shelf) => cb (Some (&rer_shelf.result)),
            None => cb (None)
        }
    }
}

/// A Send wrapper for MutexGuard
pub struct MutexGuardWrapper(pub MutexGuard<'static, ()>);
unsafe impl Send for MutexGuardWrapper {}

/// From<io::Error> is required to be implemented by futures-timer timeout.
/// We can't implement it for String directly due to Rust restrictions.
/// So this solution looks like simplest at least for now. We have to remap errors to get proper type.
pub struct StringError(pub String);

impl From<std::io::Error> for StringError {
    fn from(e: std::io::Error) -> StringError {
        StringError(ERRL!("{}", e))
    }
}

#[derive(Debug)]
pub struct QueuedCommand {
    pub response_sock: i32,
    pub stats_json_only: i32,
    pub queue_id: u32,
    pub msg: String,
    // retstrp: *mut *mut c_char,
}

lazy_static! {
    // TODO: Move to `MmCtx`.
    pub static ref COMMAND_QUEUE: (channel::Sender<QueuedCommand>, channel::Receiver<QueuedCommand>) = channel::unbounded();
}

/// Register an RPC command that came internally or from the peer-to-peer bus.
#[no_mangle]
pub extern "C" fn lp_queue_command_for_c (retstrp: *mut *mut c_char, buf: *mut c_char, response_sock: i32,
                                          stats_json_only: i32, queue_id: u32) -> () {
    if retstrp != null_mut() {
        unsafe { *retstrp = null_mut() }
    }

    if buf == null_mut() {panic! ("!buf")}
    let msg = String::from (unwrap! (unsafe {CStr::from_ptr (buf)} .to_str()));
    let cmd = QueuedCommand {
        msg,
        queue_id,
        response_sock,
        stats_json_only
    };
    unwrap! ((*COMMAND_QUEUE).0.send (cmd))
}

pub fn lp_queue_command (msg: String) -> () {
    let cmd = QueuedCommand {
        msg,
        queue_id: 0,
        response_sock: -1,
        stats_json_only: 0,
    };
    unwrap! ((*COMMAND_QUEUE).0.send (cmd))
}

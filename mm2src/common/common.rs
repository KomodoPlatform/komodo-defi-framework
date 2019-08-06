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

#![feature(non_ascii_idents, integer_atomics, panic_info_message)]
#![feature(async_await)]
#![cfg_attr(not(feature = "native"), allow(unused_imports))]
#![cfg_attr(not(feature = "native"), allow(dead_code))]

#[macro_use] extern crate arrayref;
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

#[cfg(feature = "native")]
pub mod for_c;
pub mod custom_futures;
pub mod iguana_utils;
pub mod lp_privkey;
pub mod mm_ctx;
pub mod seri;
#[cfg(feature = "native")]
pub mod lift_body;

use bigdecimal::BigDecimal;
use crossbeam::{channel};
use futures::{future, Future};
use futures::task::Task;
use hex::FromHex;
use http::{Response, StatusCode, HeaderMap};
use http::header::{HeaderValue, CONTENT_TYPE};
#[cfg(feature = "native")]
use libc::{c_char, c_void, malloc, free};
use rand::{SeedableRng, rngs::SmallRng};
use serde::{ser, de};
use serde_json::{self as json, Value as Json};
use std::env::args;
use std::fmt::{self, Write as FmtWrite};
use std::fs;
use std::ffi::{CStr};
use std::intrinsics::copy;
use std::io::{Write};
use std::mem::{forget, size_of, uninitialized, zeroed};
use std::path::{Path};
use std::ptr::{null_mut, read_volatile};
use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::str;

#[cfg(feature = "native")]
#[allow(dead_code,non_upper_case_globals,non_camel_case_types,non_snake_case)]
pub mod lp {include! (concat! (env! ("OUT_DIR"), "/c_headers/LP_include.rs"));}

pub const MM_VERSION: &'static str = env! ("MM_VERSION");

pub const SATOSHIS: u64 = 100000000;

/// Converts u64 satoshis to f64
pub fn sat_to_f(sat: u64) -> f64 { sat as f64 / SATOSHIS as f64 }

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct bits256 {pub bytes: [u8; 32]}

impl fmt::Display for bits256 {
    fn fmt (&self, fm: &mut fmt::Formatter) -> fmt::Result {
        for &ch in self.bytes.iter() {
            fn hex_from_digit (num: u8) -> char {
                if num < 10 {(b'0' + num) as char} else {(b'a' + num - 10) as char}}
            fm.write_char (hex_from_digit (ch / 16)) ?;
            fm.write_char (hex_from_digit (ch % 16)) ?;
        }
        Ok(())
}   }

impl ser::Serialize for bits256 {
    fn serialize<S> (&self, se: S) -> Result<S::Ok, S::Error> where S: ser::Serializer {
        se.serialize_bytes (&self.bytes[..])
}   }

impl<'de> de::Deserialize<'de> for bits256 {
    fn deserialize<D> (deserializer: D) -> Result<bits256, D::Error> where D: de::Deserializer<'de> {
        struct Bits256Visitor;
        impl<'de> de::Visitor<'de> for Bits256Visitor {
            type Value = bits256;
            fn expecting (&self, fm: &mut fmt::Formatter) -> fmt::Result {fm.write_str ("a byte array")}
            fn visit_seq<S> (self, mut seq: S) -> Result<bits256, S::Error> where S: de::SeqAccess<'de> {
                let mut bytes: [u8; 32] = [0; 32];
                let mut pos = 0;
                while let Some (byte) = seq.next_element()? {
                    if pos >= bytes.len() {return Err (de::Error::custom ("bytes length > 32"))}
                    bytes[pos] = byte;
                    pos += 1;
                }
                Ok (bits256 {bytes})
            }
            fn visit_bytes<E> (self, v: &[u8]) -> Result<Self::Value, E> where E: de::Error {
                if v.len() != 32 {return Err (de::Error::custom ("bytes length <> 32"))}
                Ok (bits256 {bytes: *array_ref! [v, 0, 32]})
        }   }
        deserializer.deserialize_bytes (Bits256Visitor)
}   }

impl fmt::Debug for bits256 {
    fn fmt (&self, f: &mut fmt::Formatter) -> fmt::Result {
        (self as &dyn fmt::Display) .fmt (f)
}   }

impl From<[u8; 32]> for bits256 {
    fn from (bytes: [u8; 32]) -> Self {bits256 {bytes}}
}

#[cfg(feature = "native")]
impl From<lp::_bits256> for bits256 {
    fn from (bits: lp::_bits256) -> Self {unsafe {bits256 {bytes: bits.bytes}}}
}

#[cfg(feature = "native")]
impl From<bits256> for lp::_bits256 {
    fn from (k: bits256) -> lp::_bits256 {unsafe {
        let mut bits: lp::_bits256 = zeroed();
        bits.bytes.copy_from_slice (&k.bytes[..]);
        bits
    }}
}

impl bits256 {
    /// Returns true if the hash is not zero.  
    /// Port of `#define bits256_nonz`.
    pub fn nonz (&self) -> bool {
        self.bytes.iter().any (|ch| *ch != 0)
    }
}

pub fn nonz (k: [u8; 32]) -> bool {
    k.iter().any (|ch| *ch != 0)
}

/// Decodes a HEX string into a 32-bytes array.  
/// But only if the HEX string is 64 characters long, returning a zeroed array otherwise.  
/// (Use `fn nonz` to check if the array is zeroed).  
/// A port of cJSON.c/jbits256.
pub fn jbits256 (json: &Json) -> Result<bits256, String> {
    if let Some (hex) = json.as_str() {
        if hex.len() == 64 {
            //try_s! (::common::iguana_utils::decode_hex (unsafe {&mut hash.bytes[..]}, hex.as_bytes()));
            let bytes: [u8; 32] = try_s! (FromHex::from_hex (hex));
            return Ok (bits256::from (bytes))
    }   }
    Ok (unsafe {zeroed()})
}

pub const SATOSHIDEN: i64 = 100000000;
pub fn dstr (x: i64, decimals: u8) -> f64 {x as f64 / 10.0_f64.powf(decimals as f64)}

/// Apparently helps to workaround `double` fluctuations occuring on *certain* systems.
/// cf. https://stackoverflow.com/questions/19804472/double-randomly-adds-0-000000000000001.
/// Not sure it's needed in Rust, the floating point operations should be determenistic here,
/// but better safe than sorry.
pub const SMALLVAL: f64 = 0.000000000000001;  // 1e-15f64

/// Helps sharing a string slice with C code by allocating a zero-terminated string with the C standard library allocator.
/// 
/// The difference from `CString` is that the memory is then *owned* by the C code instead of being temporarily borrowed,
/// that is it doesn't need to be recycled in Rust.
/// Plus we don't check the slice for zeroes, most of our code doesn't need that extra check.
#[cfg(feature = "native")]
pub fn str_to_malloc (s: &str) -> *mut c_char {
    slice_to_malloc (s.as_bytes()) as *mut c_char
}

/// Helps sharing a byte slice with C code by allocating a zero-terminated string with the C standard library allocator.
#[cfg(feature = "native")]
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
#[cfg(feature = "native")]
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
#[cfg(feature = "native")]
pub fn free_c_ptr(ptr: *mut c_void) { unsafe {
    if !ptr.is_null() {
        free(ptr as *mut libc::c_void);
    }
}}

/// Use the value, preventing the compiler and linker from optimizing it away.
pub fn black_box<T> (v: T) -> T {
    // https://github.com/rust-lang/rfcs/issues/1484#issuecomment-240853111
    //std::hint::black_box (v)

    let ret = unsafe {read_volatile (&v)};
    forget (v);
    ret
}

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
pub fn stack_trace_frame (buf: &mut dyn Write, symbol: &backtrace::Symbol) {
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
    if name.starts_with ("common::set_panic_hook") {return}
    if name.starts_with ("common::stack_trace") {return}
    if name.starts_with ("core::ops::") {return}
    if name.starts_with ("futures::") {return}
    if name.starts_with ("hyper::") {return}
    if name.starts_with ("mm2::crash_reports::signal_handler") {return}
    if name.starts_with ("panic_unwind::") {return}
    if name.starts_with ("std::") {return}
    if name.starts_with ("scoped_tls::") {return}
    if name.starts_with ("test::run_test::") {return}
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
pub fn stack_trace (format: &mut dyn FnMut (&mut dyn Write, &backtrace::Symbol), output: &mut dyn FnMut (&str)) {
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

/// Sets our own panic handler using patched backtrace crate. It was discovered that standard Rust panic
/// handlers print only "unknown" in Android backtraces which is not helpful.
/// Using custom hook with patched backtrace version solves this issue.
/// NB: https://github.com/rust-lang/backtrace-rs/issues/227
#[cfg(feature = "native")]
pub fn set_panic_hook() {
    use atomic::Atomic;
    use std::panic::{set_hook, PanicInfo};

    thread_local! {static ENTERED: Atomic<bool> = Atomic::new (false);}

    set_hook (Box::new (|info: &PanicInfo| {
        // Stack tracing and logging might panic (in `println!` for example).
        // Let us detect this and do nothing on second panic.
        // We'll likely still get a crash after the hook is finished
        // (experimenting with this I'm getting the "thread panicked while panicking. aborting." on Windows)
        // but that crash will have a better stack trace compared to the one with deep hook recursion.
        if let Ok (Err (_)) = ENTERED.try_with (
            |e| e.compare_exchange (false, true, Ordering::Relaxed, Ordering::Relaxed)) {
                return}

        let mut trace = String::new();
        stack_trace (&mut stack_trace_frame, &mut |l| trace.push_str (l));
        log! ((info));
        log! ("backtrace\n" (trace));

        let _ = ENTERED.try_with (|e| e.compare_exchange (true, false, Ordering::Relaxed, Ordering::Relaxed));
    }))
}

/// Simulates the panic-in-panic crash.
pub fn double_panic_crash() {
    struct Panicker (u32);
    impl Drop for Panicker {
        fn drop (&mut self) {
            panic! ("panic in drop; {}", self.0)
    }   }
    let panicker = Panicker (1);
    panic! ("first panic; {}", panicker.0)
}

/// Helps logging binary data (particularly with text-readable parts, such as bencode, netstring)
/// by replacing all the non-printable bytes with the `blank` character.
pub fn binprint (bin: &[u8], blank: u8) -> String {
    let mut bin: Vec<u8> = bin.into();
    for ch in bin.iter_mut() {if *ch < 0x20 || *ch >= 0x7F {*ch = blank}}
    unsafe {String::from_utf8_unchecked (bin)}
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
    // Stack tracing would sometimes crash on Windows, doesn't worth the risk here.
    if cfg! (windows) {return false}

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

pub type SlurpFut = Box<dyn Future<Item=(StatusCode, HeaderMap, Vec<u8>), Error=String> + Send + 'static>;

/// RPC response, returned by the RPC handlers.  
/// NB: By default the future is executed on the shared asynchronous reactor (`CORE`),
/// the handler is responsible for spawning the future on another reactor if it doesn't fit the `CORE` well.
pub type HyRes = Box<dyn Future<Item=Response<Vec<u8>>, Error=String> + Send>;

// To improve git history and ease of exploratory refactoring
// we're splitting the code in place with conditional compilation.
// wio stands for "web I/O" or "wasm I/O",
// it contains the parts which aren't directly available with WASM.

// TODO: Move this.
// How to link them together...
// 1) Use separate folders for wasm and native builds in order not to mess the C objects and linking.
// 2) In the native Rust binary add a mode which will run the WASM core (on wasmi),
//    supplying it with the necessary helpers.

#[cfg(not(feature = "native"))]
pub mod wio {
    use futures::future::IntoFuture;
    use http::Request;
    use super::SlurpFut;

    pub fn spawn<F, R> (_f: F) where
        F: FnOnce(()) -> R + Send + 'static,
        R: IntoFuture<Item = (), Error = ()>,
        R::Future: 'static
    {
        unimplemented!()
    }

    #[allow(dead_code)]
    pub fn slurp_req (_request: Request<Vec<u8>>) -> SlurpFut {
        unimplemented!()
    }
}

#[cfg(feature = "native")]
pub mod wio {
    use crate::lift_body::LiftBody;
    use crate::SlurpFut;
    use futures::{future, Async, Future, Poll};
    use futures::sync::oneshot::{self, Receiver};
    use future::IntoFuture;
    use gstuff::{any_to_str, duration_to_float, now_float};
    use http::{Request, StatusCode, HeaderMap};
    //use http_body::Body;
    use hyper::Client;
    use hyper::client::HttpConnector;
    use hyper::header::{ HeaderValue, CONTENT_TYPE };
    use hyper::rt::Stream;
    use hyper::server::conn::Http;
    use hyper_rustls::HttpsConnector;
    use std::fmt;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::process::abort;
    use std::thread;
    use std::thread::JoinHandle;
    use std::time::Duration;
    use std::str;
    use tokio_core::reactor::Remote;
    use tokio_core::reactor::Handle;

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
        /// Shared HTTP server.
        pub static ref HTTP: Http = Http::new();
    }

    pub fn spawn<F, R> (f: F) where
        F: FnOnce(&Handle) -> R + Send + 'static,
        R: IntoFuture<Item = (), Error = ()>,
        R::Future: 'static
    {
        CORE.spawn (f);
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
    /// P.S. We could try using the `futures-timer` crate instead, but note that it is currently under-maintained,
    ///      https://github.com/rustasync/futures-timer/issues/9#issuecomment-400802515. 
    pub struct Timeout<R> {
        fut: Box<dyn Future<Item=R, Error=String>>,
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
        pub fn new (fut: Box<dyn Future<Item=R, Error=String>>, timeout: Duration) -> Timeout<R> {
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
        super::black_box (&*super::trace_buf());
        super::black_box (&*super::trace_name_buf());
    }

    lazy_static! {
        /// NB: With a shared client there is a possibility that keep-alive connections will be reused.
        pub static ref HYPER: Client<HttpsConnector<HttpConnector>, LiftBody<Vec<u8>>> = {
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

    /// Executes a Hyper request, returning the response status, headers and body.
    pub fn slurp_req (request: Request<Vec<u8>>) -> SlurpFut {
        let (head, body) = request.into_parts();
        let request = Request::from_parts (head, LiftBody::from (body));

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
            let body = res.into_body();
            let body_f = body.concat2();
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
        slurp_req (try_fus! (Request::builder().uri (url) .body (Vec::new())))
    }

    #[test]
    #[ignore]
    fn test_slurp_req() {
        let (status, headers, body) = unwrap! (slurp_url ("https://httpbin.org/get") .wait());
        assert! (status.is_success(), format!("{:?} {:?} {:?}", status, headers, body));
    }

    /// Fetch URL by HTTPS and parse JSON response
    pub fn fetch_json<T>(url: &str) -> Box<dyn Future<Item=T, Error=String>>
    where T: serde::de::DeserializeOwned + Send + 'static {
        Box::new(slurp_url(url).and_then(|result| {
            // try to parse as json with serde_json
            let result = try_s!(serde_json::from_slice(&result.2));

            Ok(result)
        }))
    }

    /// Send POST JSON HTTPS request and parse response
    pub fn post_json<T>(url: &str, json: String) -> Box<dyn Future<Item=T, Error=String>>
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
}

/// Wraps a JSON string into the `HyRes` RPC response future.
pub fn rpc_response<T> (status: u16, body: T) -> HyRes where Vec<u8>: From<T> {
    let rf = match Response::builder()
        .status (status)
        .header (CONTENT_TYPE, HeaderValue::from_static ("application/json"))
        .body (Vec::from (body)) {
            Ok (r) => future::ok::<Response<Vec<u8>>, String> (r),
            Err (err) => future::err::<Response<Vec<u8>>, String> (ERRL! ("{}", err))
        };
    Box::new (rf)
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
    // TODO: Consider turning this into a macros or merging with `try_h` in order to retain the `line!`.
    log! ({"RPC error response: {}", msg});

    rpc_response(status, err_to_rpc_json_string(msg))
}

/// A closure that would (re)start a `Future` to synchronize with an external resource in `RefreshedExternalResource`.
type ExternalResourceSync<R> = Box<dyn Fn()->Box<dyn Future<Item=R,Error=String> + Send + 'static> + Send + 'static>;

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
            wio::spawn (move |_| f);  // Polls `f` in background.
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
#[cfg(feature = "native")]
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

#[cfg(feature = "native")]
pub use gstuff::{now_ms, now_float};
#[cfg(not(feature = "native"))]
pub fn now_ms() -> u64 {
    extern "C" {pub fn date_now() -> f64;}
    unsafe {date_now() as u64}
}
#[cfg(not(feature = "native"))]
pub fn now_float() -> f64 {
    use gstuff::duration_to_float;
    use std::time::Duration;
    duration_to_float (Duration::from_millis (now_ms()))
}

#[cfg(feature = "native")]
pub fn writeln (line: &str) {
    use std::panic::catch_unwind;

    // `catch_unwind` protects the tests from error
    //
    //     thread 'CORE' panicked at 'cannot access stdout during shutdown'
    //
    // (which might be related to https://github.com/rust-lang/rust/issues/29488).
    let _ = catch_unwind (|| {
        println! ("{}", line);
    });
}
#[cfg(not(feature = "native"))]
pub fn writeln (line: &str) {
    use std::ffi::CString;
    use std::os::raw::c_char;

    extern "C" {pub fn console_log (ptr: *const c_char, len: i32);}
    let lineᶜ = unwrap! (CString::new (line));
    unsafe {console_log (lineᶜ.as_ptr(), line.len() as i32)}
}

/// Set up a panic hook that prints the panic location and the message.
/// (The default Rust handler doesn't have the means to print the message.
///  Note that we're also getting the stack trace from Node.js and rustfilt).
#[cfg(not(feature = "native"))]
#[no_mangle]
pub extern fn set_panic_hook() {
    use gstuff::filename;
    use std::panic::{set_hook, PanicInfo};

    set_hook (Box::new (|info: &PanicInfo| {
        let mut msg = String::new();
        if let Some (loc) = info.location() {
            let _ = wite! (&mut msg, (filename (loc.file())) ':' (loc.line()) "] ");
        } else {
            msg.push_str ("?] ");
        }
        if let Some (message) = info.message() {
            let _ = wite! (&mut msg, "panick: " (message));
        } else {
            msg.push_str ("panick!")
        }
        writeln (&msg)
    }))
}

pub fn small_rng() -> SmallRng {
    SmallRng::seed_from_u64 (now_ms())
}

/// Proxy invoking a helper function which takes the (ptr, len) input and fills the (rbuf, rlen) output.
#[macro_export]
macro_rules! io_buf_proxy {
    ($helper:ident, $payload:expr, $rlen:literal) => {
        unsafe {
            let payload = try_s! (json::to_vec ($payload));
            let mut rbuf: [u8; $rlen] = std::mem::uninitialized();
            let mut rlen = rbuf.len() as u32;
            $helper (
                payload.as_ptr(), payload.len() as u32,
                rbuf.as_mut_ptr(), &mut rlen
            );
            let rlen = rlen as usize;
            // Checks that `rlen` has changed
            // (`rlen` staying the same might indicate that the helper was not invoked).
            if rlen >= rbuf.len() {return ERR! ("Bad rlen: {}", rlen)}
            try_s! (json::from_slice (&rbuf[0..rlen]))
}   }   }

#[doc(hidden)]
pub fn serialize_to_rbuf<T: ser::Serialize> (line: u32, rc: Result<T, String>, rbuf: *mut u8, rlen: *mut u32) {
    use std::io::Cursor;
    use std::ptr::{read_unaligned, write_unaligned};
    use std::slice::from_raw_parts_mut;
    unsafe {
        let rbuf_capacity = read_unaligned (rlen) as usize;
        let rbufˢ: &mut [u8] = from_raw_parts_mut (rbuf, rbuf_capacity);
        let mut cur = Cursor::new (rbufˢ);
        if let Err (err) = json::to_writer (&mut cur, &rc) {
            let rbufˢ: &mut [u8] = from_raw_parts_mut (rbuf, rbuf_capacity);
            cur = Cursor::new (rbufˢ);
            let rc: Result<T, String> = Err (fomat! ((line) "] Error serializing response: " (err)));
            unwrap! (json::to_writer (&mut cur, &rc), "Error serializing an error");
        }
        let seralized_len = cur.position();
        assert! (seralized_len <= rbuf_capacity as u64);
        write_unaligned (rlen, seralized_len as u32)
}   }

#[macro_export]
macro_rules! helper {
    ($helperⁱ:ident, $encoded_argsⁱ:ident: $encoded_argsᵗ:ty, $body:block) => {
        #[cfg(not(feature = "native"))]
        extern "C" {pub fn $helperⁱ (ptr: *const u8, len: u32, rbuf: *mut u8, rlen: *mut u32);}

        #[cfg(feature = "native")]
        #[no_mangle]
        pub extern fn $helperⁱ (ptr: *const u8, len: u32, rbuf: *mut u8, rlen: *mut u32) {
            use std::slice::from_raw_parts;
            // TODO: Try using bencode instead of JSON.

            let rc: Result<_, String>;
            let encoded_argsˢ = unsafe {from_raw_parts (ptr, len as usize)};
            match json::from_slice::<$encoded_argsᵗ> (encoded_argsˢ) {
                Err (err) => rc = ERR! (concat! (stringify! ($helperⁱ), "] error deserializing: {}"), err),
                Ok ($encoded_argsⁱ) => rc = (|| $body)()
            }
            $crate::serialize_to_rbuf (line!(), rc, rbuf, rlen)
}   }   }

fn without_trailing_zeroes (decimal: &str, dot: usize) -> &str {
    let mut pos = decimal.len() - 1;
    loop {
        let ch = decimal.as_bytes()[pos];
        if ch != b'0' {break &decimal[0..=pos]}
        if pos == dot {break &decimal[0..pos]}
        pos -= 1
    }
}

/// Round half away from zero (aka commercial rounding).
pub fn round_to (bd: &BigDecimal, places: u8) -> String {
    // Normally we'd do
    // 
    //     let divisor = pow (10, places)
    //     round (self * divisor) / divisor
    // 
    // But we don't have a `round` function in `BigDecimal` at present, so we're on our own.

    let bds = format! ("{}", bd);
    let bda = bds.as_bytes();
    let dot = bda.iter().position (|&ch| ch == b'.');
    let dot = match dot {Some (dot) => dot, None => return bds};

    if bda.len() - dot <= places as usize {
        return String::from (without_trailing_zeroes (&bds, dot))
    }

    let mut pos = bda.len() - 1;
    let mut ch = bda[pos];
    let mut prev_digit = 0;
    loop {
        let digit = ch - b'0';
        let rounded = if prev_digit > 5 {digit + 1} else {digit};
        //println! ("{} at {}: prev_digit {}, digit {}, rounded {}", bds, pos, prev_digit, digit, rounded);

        if pos < dot {
            //println! ("{}, pos < dot, stopping at pos {}", bds, pos);
            let mut integer: i64 = unwrap! ((&bds[0..=pos]).parse());
            if prev_digit > 5 {
                if bda[0] == b'-' {
                    integer = unwrap! (integer.checked_sub (1))
                } else {
                    integer = unwrap! (integer.checked_add (1))
            }   }
            return format! ("{}", integer)
        }

        if pos == dot + places as usize && rounded < 10 {
            //println! ("{}, stopping at pos {}", bds, pos);
            break format! ("{}{}", &bds[0..pos], rounded)
        }

        pos -= 1;
        if pos == dot {pos -= 1}  // Skip over the dot.
        ch = bda[pos];
        prev_digit = rounded
    }
}

#[test]
fn test_round_to() {
    assert_eq! (round_to (&BigDecimal::from (0.999), 2), "1");
    assert_eq! (round_to (&BigDecimal::from (-0.999), 2), "-1");

    assert_eq! (round_to (&BigDecimal::from (10.999), 2), "11");
    assert_eq! (round_to (&BigDecimal::from (-10.999), 2), "-11");

    assert_eq! (round_to (&BigDecimal::from (99.9), 1), "99.9");
    assert_eq! (round_to (&BigDecimal::from (-99.9), 1), "-99.9");

    assert_eq! (round_to (&BigDecimal::from (99.9), 0), "100");
    assert_eq! (round_to (&BigDecimal::from (-99.9), 0), "-100");

    let ouch = BigDecimal::from (1) / BigDecimal::from (7);
    assert_eq! (round_to (&ouch, 3), "0.143");

    let ouch = BigDecimal::from (1) / BigDecimal::from (3);
    assert_eq! (round_to (&ouch, 0), "0");
    assert_eq! (round_to (&ouch, 1), "0.3");
    assert_eq! (round_to (&ouch, 2), "0.33");
    assert_eq! (round_to (&ouch, 9), "0.333333333");

    assert_eq! (round_to (&BigDecimal::from (0.123), 99), "0.123");
    assert_eq! (round_to (&BigDecimal::from (-0.123), 99), "-0.123");

    assert_eq! (round_to (&BigDecimal::from (0), 99), "0");
    assert_eq! (round_to (&BigDecimal::from (-0), 99), "0");

    assert_eq! (round_to (&BigDecimal::from (0.123), 0), "0");
    assert_eq! (round_to (&BigDecimal::from (-0.123), 0), "0");

    assert_eq! (round_to (&BigDecimal::from (0), 0), "0");
    assert_eq! (round_to (&BigDecimal::from (-0), 0), "0");
}

pub mod for_tests;

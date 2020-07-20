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
#![feature(async_closure)]
#![feature(hash_raw_entry)]
#![feature(optin_builtin_traits)]
#![feature(const_fn)]
#![allow(uncommon_codepoints)]
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
            fn from(t: $id) -> $enum { $enum::$id(t) }
        }
    };
}

#[macro_use]
pub mod jsonrpc_client;
#[macro_use]
pub mod log;
#[macro_use]
pub mod mm_metrics;

pub mod big_int_str;
pub mod custom_futures;
pub mod duplex_mutex;
pub mod file_lock;
#[cfg(feature = "native")] pub mod for_c;
pub mod header;
pub mod iguana_utils;
pub mod mm_ctx;
pub mod mm_number;
pub mod privkey;
pub mod seri;

#[cfg(feature = "native")] pub mod lift_body;
#[cfg(not(feature = "native"))]
pub mod lift_body {
    #[derive(Debug)]
    pub struct LiftBody<T> {
        inner: T,
    }
}

use atomic::Atomic;
use bigdecimal::BigDecimal;
#[cfg(all(feature = "native", not(windows)))]
use findshlibs::{IterationControl, Segment, SharedLibrary, TargetSharedLibrary};
use futures::compat::Future01CompatExt;
use futures::future::FutureExt;
use futures::task::Waker;
#[cfg(not(feature = "native"))]
use futures::task::{Context, Poll as Poll03};
use futures01::{future, task::Task, Future};
use gstuff::binprint;
use hex::FromHex;
use http::header::{HeaderValue, CONTENT_TYPE};
use http::{HeaderMap, Request, Response, StatusCode};
#[cfg(feature = "native")] use libc::{free, malloc};
use parking_lot::{Mutex as PaMutex, MutexGuard as PaMutexGuard};
use rand::{rngs::SmallRng, SeedableRng};
use serde::{de, ser};
#[cfg(not(feature = "native"))]
use serde_bencode::de::from_bytes as bdecode;
use serde_bytes::ByteBuf;
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;
use std::env::{self, args};
use std::ffi::{CStr, OsStr};
use std::fmt::{self, Write as FmtWrite};
use std::fs;
use std::fs::DirEntry;
use std::future::Future as Future03;
use std::intrinsics::copy;
use std::io::Write;
use std::mem::{forget, size_of, zeroed};
use std::ops::{Add, Deref, Div, RangeInclusive};
use std::os::raw::{c_char, c_void};
use std::path::{Path, PathBuf};
#[cfg(not(feature = "native"))] use std::pin::Pin;
use std::ptr::{null_mut, read_volatile};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;
use uuid::Uuid;
#[cfg(feature = "w-bindgen")] use wasm_bindgen::prelude::*;

pub use num_bigint::BigInt;

#[cfg(feature = "native")]
#[allow(dead_code,non_upper_case_globals,non_camel_case_types,non_snake_case)]
pub mod lp {include! (concat! (env! ("OUT_DIR"), "/c_headers/LP_include.rs"));}

pub const MM_DATETIME: &str = env!("MM_DATETIME");
pub const MM_VERSION: &str = env!("MM_VERSION");

pub const SATOSHIS: u64 = 100_000_000;

/// Converts u64 satoshis to f64
pub fn sat_to_f(sat: u64) -> f64 { sat as f64 / SATOSHIS as f64 }

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct bits256 {
    pub bytes: [u8; 32],
}

impl Default for bits256 {
    fn default() -> bits256 {
        bits256 {
            bytes: unsafe { zeroed() },
        }
    }
}

impl fmt::Display for bits256 {
    fn fmt(&self, fm: &mut fmt::Formatter) -> fmt::Result {
        for &ch in self.bytes.iter() {
            fn hex_from_digit(num: u8) -> char {
                if num < 10 {
                    (b'0' + num) as char
                } else {
                    (b'a' + num - 10) as char
                }
            }
            fm.write_char(hex_from_digit(ch / 16))?;
            fm.write_char(hex_from_digit(ch % 16))?;
        }
        Ok(())
    }
}

impl ser::Serialize for bits256 {
    fn serialize<S>(&self, se: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        se.serialize_bytes(&self.bytes[..])
    }
}

impl<'de> de::Deserialize<'de> for bits256 {
    fn deserialize<D>(deserializer: D) -> Result<bits256, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct Bits256Visitor;
        impl<'de> de::Visitor<'de> for Bits256Visitor {
            type Value = bits256;
            fn expecting(&self, fm: &mut fmt::Formatter) -> fmt::Result { fm.write_str("a byte array") }
            fn visit_seq<S>(self, mut seq: S) -> Result<bits256, S::Error>
            where
                S: de::SeqAccess<'de>,
            {
                let mut bytes: [u8; 32] = [0; 32];
                let mut pos = 0;
                while let Some(byte) = seq.next_element()? {
                    if pos >= bytes.len() {
                        return Err(de::Error::custom("bytes length > 32"));
                    }
                    bytes[pos] = byte;
                    pos += 1;
                }
                Ok(bits256 { bytes })
            }
            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if v.len() != 32 {
                    return Err(de::Error::custom("bytes length <> 32"));
                }
                Ok(bits256 {
                    bytes: *array_ref![v, 0, 32],
                })
            }
        }
        deserializer.deserialize_bytes(Bits256Visitor)
    }
}

impl fmt::Debug for bits256 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { (self as &dyn fmt::Display).fmt(f) }
}

impl From<[u8; 32]> for bits256 {
    fn from(bytes: [u8; 32]) -> Self { bits256 { bytes } }
}

impl bits256 {
    /// Returns true if the hash is not zero.  
    /// Port of `#define bits256_nonz`.
    pub fn nonz(&self) -> bool { self.bytes.iter().any(|ch| *ch != 0) }
}

pub fn nonz(k: [u8; 32]) -> bool { k.iter().any(|ch| *ch != 0) }

/// Decodes a HEX string into a 32-bytes array.  
/// But only if the HEX string is 64 characters long, returning a zeroed array otherwise.  
/// (Use `fn nonz` to check if the array is zeroed).  
/// A port of cJSON.c/jbits256.
pub fn jbits256(json: &Json) -> Result<bits256, String> {
    if let Some(hex) = json.as_str() {
        if hex.len() == 64 {
            //try_s! (::common::iguana_utils::decode_hex (unsafe {&mut hash.bytes[..]}, hex.as_bytes()));
            let bytes: [u8; 32] = try_s!(FromHex::from_hex(hex));
            return Ok(bits256::from(bytes));
        }
    }
    Ok(unsafe { zeroed() })
}

pub const SATOSHIDEN: i64 = 100_000_000;
pub fn dstr(x: i64, decimals: u8) -> f64 { x as f64 / 10.0_f64.powf(decimals as f64) }

/// Apparently helps to workaround `double` fluctuations occuring on *certain* systems.
/// cf. https://stackoverflow.com/questions/19804472/double-randomly-adds-0-000000000000001.
/// Not sure it's needed in Rust, the floating point operations should be determenistic here,
/// but better safe than sorry.
pub const SMALLVAL: f64 = 0.000_000_000_000_001; // 1e-15f64

/// Helps sharing a string slice with C code by allocating a zero-terminated string with the C standard library allocator.
///
/// The difference from `CString` is that the memory is then *owned* by the C code instead of being temporarily borrowed,
/// that is it doesn't need to be recycled in Rust.
/// Plus we don't check the slice for zeroes, most of our code doesn't need that extra check.
#[cfg(feature = "native")]
pub fn str_to_malloc(s: &str) -> *mut c_char { slice_to_malloc(s.as_bytes()) as *mut c_char }

/// Helps sharing a byte slice with C code by allocating a zero-terminated string with the C standard library allocator.
#[cfg(feature = "native")]
pub fn slice_to_malloc(bytes: &[u8]) -> *mut u8 {
    unsafe {
        let buf = malloc(bytes.len() + 1) as *mut u8;
        copy(bytes.as_ptr(), buf, bytes.len());
        *buf.add(bytes.len()) = 0;
        buf
    }
}

/// Converts *mut c_char to Rust String
/// Doesn't free the allocated memory
/// It's responsibility of the caller to free the memory when required
/// Returns error in case of null pointer input
#[cfg(feature = "native")]
#[allow(clippy::missing_safety_doc)]
pub unsafe fn c_char_to_string(ptr: *mut c_char) -> Result<String, String> {
    if !ptr.is_null() {
        let res_str = try_s!(CStr::from_ptr(ptr).to_str());
        let res_str = String::from(res_str);
        Ok(res_str)
    } else {
        ERR!("Tried to convert null pointer to Rust String!")
    }
}

/// Frees C raw pointer
/// Does nothing in case of null pointer input
#[cfg(feature = "native")]
pub fn free_c_ptr(ptr: *mut c_void) {
    unsafe {
        if !ptr.is_null() {
            free(ptr as *mut libc::c_void);
        }
    }
}

/// Use the value, preventing the compiler and linker from optimizing it away.
pub fn black_box<T>(v: T) -> T {
    // https://github.com/rust-lang/rfcs/issues/1484#issuecomment-240853111
    //std::hint::black_box (v)

    let ret = unsafe { read_volatile(&v) };
    forget(v);
    ret
}

/// Attempts to remove the `Path` on `drop`.
#[derive(Debug)]
pub struct RaiiRm<'a>(pub &'a Path);
impl<'a> AsRef<Path> for RaiiRm<'a> {
    fn as_ref(&self) -> &Path { self.0 }
}
impl<'a> Drop for RaiiRm<'a> {
    fn drop(&mut self) { let _ = fs::remove_file(self); }
}

/// Using a static buffer in order to minimize the chance of heap and stack allocations in the signal handler.
fn trace_buf() -> PaMutexGuard<'static, [u8; 256]> {
    static TRACE_BUF: PaMutex<[u8; 256]> = PaMutex::new([0; 256]);
    TRACE_BUF.lock()
}

fn trace_name_buf() -> PaMutexGuard<'static, [u8; 128]> {
    static TRACE_NAME_BUF: PaMutex<[u8; 128]> = PaMutex::new([0; 128]);
    TRACE_NAME_BUF.lock()
}

/// Formats a stack frame.
/// Some common and less than useful frames are skipped.
pub fn stack_trace_frame(instr_ptr: *mut c_void, buf: &mut dyn Write, symbol: &backtrace::Symbol) {
    let filename = match symbol.filename() {
        Some(path) => match path.components().rev().next() {
            Some(c) => c.as_os_str().to_string_lossy(),
            None => "??".into(),
        },
        None => "??".into(),
    };
    let lineno = match symbol.lineno() {
        Some(lineno) => lineno,
        None => 0,
    };
    let name = match symbol.name() {
        Some(name) => name,
        None => SymbolName::new(&[]),
    };
    let mut name_buf = trace_name_buf();
    let name = gstring!(name_buf, {
        let _ = write!(name_buf, "{}", name); // NB: `fmt` is different from `SymbolName::as_str`.
    });

    // Skip common and less than informative frames.

    match name {
        "mm2::crash_reports::rust_seh_handler"
        | "veh_exception_filter"
        | "common::stack_trace"
        | "common::log_stacktrace"
        // Super-main on Windows.
        | "__scrt_common_main_seh" => return,
        _ => (),
    }

    match filename.as_ref() {
        "boxed.rs" | "panic.rs" => return,
        _ => (),
    }

    if name.starts_with("alloc::")
        || name.starts_with("backtrace::")
        || name.starts_with("common::set_panic_hook")
        || name.starts_with("common::stack_trace")
        || name.starts_with("core::ops::")
        || name.starts_with("futures::")
        || name.starts_with("hyper::")
        || name.starts_with("mm2::crash_reports::signal_handler")
        || name.starts_with("panic_unwind::")
        || name.starts_with("std::")
        || name.starts_with("scoped_tls::")
        || name.starts_with("test::run_test::")
        || name.starts_with("tokio::")
        || name.starts_with("tokio_core::")
        || name.starts_with("tokio_reactor::")
        || name.starts_with("tokio_executor::")
        || name.starts_with("tokio_timer::")
    {
        return;
    }

    let _ = writeln!(buf, "  {}:{}] {} {:?}", filename, lineno, name, instr_ptr);
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
pub fn stack_trace(
    format: &mut dyn FnMut(*mut c_void, &mut dyn Write, &backtrace::Symbol),
    output: &mut dyn FnMut(&str),
) {
    // cf. https://github.com/rust-lang/rust/pull/64154 (standard library backtrace)

    backtrace::trace(|frame| {
        backtrace::resolve(frame.ip(), |symbol| {
            let mut trace_buf = trace_buf();
            let trace = gstring!(trace_buf, {
                // frame.ip() is next instruction pointer typically so offset(-1) points to current instruction
                format(frame.ip().wrapping_offset(-1), trace_buf, symbol);
            });
            output(trace);
        });
        true
    });

    #[cfg(all(feature = "native", not(windows)))]
    output_pc_mem_addr(output)
}

#[cfg(all(feature = "native", not(windows)))]
fn output_pc_mem_addr(output: &mut dyn FnMut(&str)) {
    TargetSharedLibrary::each(|shlib| {
        let mut trace_buf = trace_buf();
        let name = gstring!(trace_buf, {
            let _ = write!(
                trace_buf,
                "Virtual memory addresses of {}",
                shlib.name().to_string_lossy()
            );
        });
        output(name);
        for seg in shlib.segments() {
            let segment = gstring!(trace_buf, {
                let _ = write!(
                    trace_buf,
                    "  {}:{}",
                    seg.name(),
                    seg.actual_virtual_memory_address(shlib)
                );
            });
            output(segment);
        }
        // First TargetSharedLibrary is initial executable, we are not interested in other libs
        IterationControl::Break
    });
}

/// Sets our own panic handler using patched backtrace crate. It was discovered that standard Rust panic
/// handlers print only "unknown" in Android backtraces which is not helpful.
/// Using custom hook with patched backtrace version solves this issue.
/// NB: https://github.com/rust-lang/backtrace-rs/issues/227
#[cfg(feature = "native")]
pub fn set_panic_hook() {
    use std::panic::{set_hook, PanicInfo};

    thread_local! {static ENTERED: Atomic<bool> = Atomic::new (false);}

    set_hook(Box::new(|info: &PanicInfo| {
        // Stack tracing and logging might panic (in `println!` for example).
        // Let us detect this and do nothing on second panic.
        // We'll likely still get a crash after the hook is finished
        // (experimenting with this I'm getting the "thread panicked while panicking. aborting." on Windows)
        // but that crash will have a better stack trace compared to the one with deep hook recursion.
        if let Ok(Err(_)) = ENTERED.try_with(|e| e.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed))
        {
            return;
        }

        let mut trace = String::new();
        stack_trace(&mut stack_trace_frame, &mut |l| trace.push_str(l));
        log!((info));
        log!("backtrace\n"(trace));

        let _ = ENTERED.try_with(|e| e.compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed));
    }))
}

/// Simulates the panic-in-panic crash.
pub fn double_panic_crash() {
    struct Panicker;
    impl Drop for Panicker {
        fn drop(&mut self) { panic!("panic in drop") }
    }
    let panicker = Panicker;
    if 1 < 2 {
        panic!("first panic")
    }
    drop(panicker) // Delays the drop.
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
    if cfg!(windows) {
        return false;
    }

    let mut trace = String::with_capacity(1024);
    stack_trace(
        &mut |_ptr, mut fwr, sym| {
            if let Some(name) = sym.name() {
                let _ = witeln!(fwr, (name));
            }
        },
        &mut |tr| trace.push_str(tr),
    );

    if trace.contains("\nmm2::main\n") || trace.contains("\nmm2::run_lp_main\n") {
        return false;
    }

    if let Some(executable) = args().next() {
        if executable.ends_with(r"\mm2.exe") {
            return false;
        }
        if executable.ends_with("/mm2") {
            return false;
        }
    }

    true
}

pub type SlurpFut = Box<dyn Future<Item = (StatusCode, HeaderMap, Vec<u8>), Error = String> + Send + 'static>;

/// RPC response, returned by the RPC handlers.  
/// NB: By default the future is executed on the shared asynchronous reactor (`CORE`),
/// the handler is responsible for spawning the future on another reactor if it doesn't fit the `CORE` well.
pub type HyRes = Box<dyn Future<Item = Response<Vec<u8>>, Error = String> + Send>;

#[derive(Debug, Deserialize, Serialize)]
struct HostedHttpRequest {
    method: String,
    uri: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug, Deserialize, Serialize)]
struct HostedHttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

// To improve git history and ease of exploratory refactoring
// we're splitting the code in place with conditional compilation.
// wio stands for "web I/O" or "wasm I/O",
// it contains the parts which aren't directly available with WASM.

#[cfg(not(feature = "native"))]
pub mod wio {
    use super::SlurpFut;
    use futures::channel::oneshot::{channel, Receiver, Sender};
    use futures::compat::Compat;
    use futures::future::FutureExt;
    use futures::lock::Mutex;
    use futures01::future::IntoFuture;
    use http::header::{HeaderName, HeaderValue};
    use http::{HeaderMap, Method, Request, StatusCode};
    use rand::Rng;
    use serde_bencode::de::from_bytes as bdecode;
    use serde_bencode::ser::to_bytes as bencode;
    use std::collections::HashMap;
    use std::os::raw::c_char;
    use std::str::FromStr;

    pub async fn slurp_reqʹ(request: Request<Vec<u8>>) -> Result<(StatusCode, HeaderMap, Vec<u8>), String> {
        let (parts, body) = request.into_parts();

        let hhreq = super::HostedHttpRequest {
            method: parts.method.as_str().to_owned(),
            uri: fomat!((parts.uri)),
            headers: parts
                .headers
                .iter()
                .filter_map(|(name, value)| {
                    let name = name.as_str().to_owned();
                    let v = match value.to_str() {
                        Ok(ascii) => ascii,
                        Err(err) => {
                            log! ("!ascii '" (name) "': " (err));
                            return None;
                        },
                    };
                    Some((name, v.to_owned()))
                })
                .collect(),
            body,
        };

        let hhreq = try_s!(bencode(&hhreq));
        let hhres = try_s!(super::helperᶜ("slurp_req", hhreq).await);
        let hhres: super::HostedHttpResponse = try_s!(bdecode(&hhres));
        let status = try_s!(StatusCode::from_u16(hhres.status));

        let mut headers = HeaderMap::<HeaderValue>::with_capacity(hhres.headers.len());
        for (n, v) in hhres.headers {
            headers.insert(
                try_s!(HeaderName::from_str(&n[..])),
                try_s!(HeaderValue::from_str(&v[..])),
            );
        }

        Ok((status, headers, hhres.body))
    }

    pub fn slurp_req(request: Request<Vec<u8>>) -> SlurpFut { Box::new(Compat::new(Box::pin(slurp_reqʹ(request)))) }
}

#[cfg(feature = "native")]
pub mod wio {
    use crate::lift_body::LiftBody;
    use crate::SlurpFut;
    use bytes::Bytes;
    use futures::compat::Future01CompatExt;
    use futures::executor::ThreadPool;
    use futures01::sync::oneshot::{self, Receiver};
    use futures01::{Async, Future, Poll};
    use futures_cpupool::CpuPool;
    use gstuff::{duration_to_float, now_float};
    use http::{HeaderMap, Method, Request, StatusCode};
    use hyper::client::HttpConnector;
    use hyper::rt::Stream;
    use hyper::server::conn::Http;
    use hyper::Client;
    use hyper_rustls::HttpsConnector;
    use serde_bencode::de::from_bytes as bdecode;
    use serde_bencode::ser::to_bytes as bencode;
    use std::fmt;
    use std::sync::Mutex;
    use std::thread::JoinHandle;
    use std::time::Duration;
    use tokio::runtime::Runtime;

    fn start_core_thread() -> Runtime { unwrap!(tokio::runtime::Builder::new().build()) }

    lazy_static! {
        /// Shared asynchronous reactor.
        pub static ref CORE: Mutex<Runtime> = Mutex::new (start_core_thread());
        /// Shared CPU pool to run intensive/sleeping requests on a separate thread.
        ///
        /// Deprecated, prefer the futures 0.3 `POOL` instead.
        pub static ref CPUPOOL: CpuPool = CpuPool::new(8);
        /// Shared CPU pool to run intensive/sleeping requests on s separate thread.
        pub static ref POOL: Mutex<ThreadPool> = Mutex::new (unwrap! (ThreadPool::builder()
            .pool_size (8)
            .name_prefix ("POOL")
            .create(), "!ThreadPool"));
        /// Shared HTTP server.
        pub static ref HTTP: Http = Http::new();
    }

    /// With a shared reactor drives the future `f` to completion.
    ///
    /// NB: This function is only useful if you need to get the results of the execution.
    /// If the results are not necessary then a future can be scheduled directly on the reactor:
    ///
    ///     CORE.spawn (|_| f);
    pub fn drive<F, R, E>(f: F) -> Receiver<Result<R, E>>
    where
        F: Future<Item = R, Error = E> + Send + 'static,
        R: Send + 'static,
        E: Send + 'static,
    {
        let (sx, rx) = oneshot::channel();
        unwrap!(CORE.lock()).spawn(f.then(move |fr: Result<R, E>| -> Result<(), ()> {
            let _ = sx.send(fr);
            Ok(())
        }));
        rx
    }

    /// With a shared reactor drives the future `f` to completion.
    ///
    /// Similar to `fn drive`, but returns a stringified error,
    /// allowing us to collapse the `Receiver` and return the `R` directly.
    pub fn drive_s<F, R, E>(f: F) -> impl Future<Item = R, Error = String>
    where
        F: Future<Item = R, Error = E> + Send + 'static,
        R: Send + 'static,
        E: fmt::Display + Send + 'static,
    {
        drive(f).then(move |r| -> Result<R, String> {
            let r = try_s!(r); // Peel the `Receiver`.
            let r = try_s!(r); // `E` to `String`.
            Ok(r)
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
        fut: Box<dyn Future<Item = R, Error = String>>,
        started: f64,
        timeout: f64,
        monitor: Option<JoinHandle<()>>,
    }
    impl<R> Future for Timeout<R> {
        type Item = R;
        type Error = String;
        fn poll(&mut self) -> Poll<R, String> {
            match self.fut.poll() {
                Err(err) => Err(err),
                Ok(Async::Ready(r)) => Ok(Async::Ready(r)),
                Ok(Async::NotReady) => {
                    let now = now_float();
                    if now >= self.started + self.timeout {
                        Err(format!("timeout ({:.1} > {:.1})", now - self.started, self.timeout))
                    } else {
                        // Start waking up this future until it has a chance to timeout.
                        // For now it's just a basic separate thread. Will probably optimize later.
                        if self.monitor.is_none() {
                            let task = futures01::task::current();
                            let deadline = self.started + self.timeout;
                            self.monitor = Some(unwrap!(std::thread::Builder::new()
                                .name("timeout monitor".into())
                                .spawn(move || {
                                    loop {
                                        std::thread::sleep(Duration::from_secs(1));
                                        task.notify();
                                        if now_float() > deadline + 2. {
                                            break;
                                        }
                                    }
                                })));
                        }
                        Ok(Async::NotReady)
                    }
                },
            }
        }
    }
    impl<R> Timeout<R> {
        pub fn new(fut: Box<dyn Future<Item = R, Error = String>>, timeout: Duration) -> Timeout<R> {
            Timeout {
                fut,
                started: now_float(),
                timeout: duration_to_float(timeout),
                monitor: None,
            }
        }
    }

    unsafe impl<R> Send for Timeout<R> {}

    /// Initialize the crate.
    pub fn init() {
        // Pre-allocate the stack trace buffer in order to avoid allocating it from a signal handler.
        super::black_box(&*super::trace_buf());
        super::black_box(&*super::trace_name_buf());
    }

    lazy_static! {
        /// NB: With a shared client there is a possibility that keep-alive connections will be reused.
        pub static ref HYPER: Client<HttpsConnector<HttpConnector>, LiftBody<Vec<u8>>> = {
            let dns_threads = 2;
            let https = HttpsConnector::new (dns_threads);
            Client::builder()
                .executor (unwrap! (CORE.lock()) .executor())
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
                .build (https)
        };
    }

    /// Executes a Hyper request, returning the response status, headers and body.
    pub fn slurp_req(request: Request<Vec<u8>>) -> SlurpFut {
        let (head, body) = request.into_parts();
        let request = Request::from_parts(head, LiftBody::from(body));

        let uri = fomat!((request.uri()));
        let request_f = HYPER.request(request);
        let response_f = request_f.then(move |res| -> SlurpFut {
            // Can fail with:
            // "an IO error occurred: An existing connection was forcibly closed by the remote host. (os error 10054)" (on Windows)
            // "an error occurred trying to connect: No connection could be made because the target machine actively refused it. (os error 10061)"
            // "an error occurred trying to connect: Connection refused (os error 111)"
            let res = match res {
                Ok(r) => r,
                Err(err) => return Box::new(futures01::future::err(ERRL!("Error accessing '{}': {}", uri, err))),
            };
            let status = res.status();
            let headers = res.headers().clone();
            let body = res.into_body();
            let body_f = body.concat2();
            let combined_f = body_f.then(move |body| -> Result<(StatusCode, HeaderMap, Vec<u8>), String> {
                let body = try_s!(body);
                Ok((status, headers, body.to_vec()))
            });
            Box::new(combined_f)
        });
        Box::new(drive_s(response_f))
    }

    pub async fn slurp_reqʹ(request: Request<Vec<u8>>) -> Result<(StatusCode, HeaderMap, Vec<u8>), String> {
        slurp_req(request).compat().await
    }

    pub async fn slurp_reqʰ(req: Bytes) -> Result<Vec<u8>, String> {
        let hhreq: super::HostedHttpRequest = try_s!(bdecode(&req));
        //log! ("slurp_reqʰ] " [=hhreq]);

        let mut req = Request::builder();
        req.method(try_s!(Method::from_bytes(hhreq.method.as_bytes())));
        req.uri(hhreq.uri);
        for (n, v) in hhreq.headers {
            req.header(&n[..], &v[..]);
        }
        let req = try_s!(req.body(hhreq.body));

        let (status, headers, body) = try_s!(slurp_reqʹ(req).await);

        let hhres = super::HostedHttpResponse {
            status: status.as_u16(),
            headers: headers
                .iter()
                .filter_map(|(name, value)| {
                    let name = name.as_str().to_owned();
                    let v = match value.to_str() {
                        Ok(ascii) => ascii,
                        Err(err) => {
                            log! ("!ascii '" (name) "': " (err));
                            return None;
                        },
                    };
                    Some((name, v.to_owned()))
                })
                .collect(),
            body,
        };
        //log! ("HostedHttpResponse: " [=hhres]);

        let hhres = try_s!(bencode(&hhres));
        Ok(hhres)
    }
}

#[cfg(feature = "native")]
pub mod executor {
    use futures::task::Context;
    use futures::{Future as Future03, FutureExt, Poll as Poll03, TryFutureExt};
    use gstuff::now_float;
    use std::pin::Pin;
    use std::thread;
    use std::time::Duration;

    pub fn spawn(future: impl Future03<Output = ()> + Send + 'static) {
        let f = future.unit_error().boxed().compat();
        unwrap!(crate::wio::CORE.lock()).spawn(f);
    }

    /// Schedule the given `future` to be executed shortly after the given `utc` time is reached.
    pub fn spawn_after(utc: f64, future: impl Future03<Output = ()> + Send + 'static) {
        use crossbeam::channel;
        use gstuff::Constructible;
        use std::collections::BTreeMap;
        use std::sync::Once;

        type SheduleChannelItem = (f64, Pin<Box<dyn Future03<Output = ()> + Send + 'static>>);
        static START: Once = Once::new();
        static SCHEDULE: Constructible<channel::Sender<SheduleChannelItem>> = Constructible::new();
        START.call_once(|| {
            unwrap!(
                thread::Builder::new().name("spawn_after".into()).spawn(move || {
                    let (tx, rx) = channel::bounded(0);
                    unwrap!(SCHEDULE.pin(tx), "spawn_after] Can't pin the channel");
                    type Task = Pin<Box<dyn Future03<Output = ()> + Send + 'static>>;
                    let mut tasks: BTreeMap<Duration, Vec<Task>> = BTreeMap::new();
                    let mut ready = Vec::with_capacity(4);
                    loop {
                        let now = Duration::from_secs_f64(now_float());
                        let mut next_stop = Duration::from_secs_f64(0.1);
                        for (utc, _) in tasks.iter() {
                            if *utc <= now {
                                ready.push(*utc)
                            } else {
                                next_stop = *utc - now;
                                break;
                            }
                        }
                        for utc in ready.drain(..) {
                            let v = match tasks.remove(&utc) {
                                Some(v) => v,
                                None => continue,
                            };
                            //log! ("spawn_after] spawning " (v.len()) " tasks at " [utc]);
                            for f in v {
                                spawn(f)
                            }
                        }
                        let (utc, f) = match rx.recv_timeout(next_stop) {
                            Ok(t) => t,
                            Err(channel::RecvTimeoutError::Disconnected) => break,
                            Err(channel::RecvTimeoutError::Timeout) => continue,
                        };
                        tasks
                            .entry(Duration::from_secs_f64(utc))
                            .or_insert_with(Vec::new)
                            .push(f)
                    }
                }),
                "Can't spawn a spawn_after thread"
            );
        });
        loop {
            match SCHEDULE.as_option() {
                None => {
                    thread::yield_now();
                    continue;
                },
                Some(tx) => {
                    unwrap!(tx.send((utc, Box::pin(future))), "Can't reach spawn_after");
                    break;
                },
            }
        }
    }

    /// A future that completes at a given time.  
    pub struct Timer {
        till_utc: f64,
    }

    impl Timer {
        pub fn till(till_utc: f64) -> Timer { Timer { till_utc } }
        pub fn sleep(seconds: f64) -> Timer {
            Timer {
                till_utc: now_float() + seconds,
            }
        }
        pub fn till_utc(&self) -> f64 { self.till_utc }
    }

    impl Future03 for Timer {
        type Output = ();
        fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll03<Self::Output> {
            let delta = self.till_utc - now_float();
            if delta <= 0. {
                return Poll03::Ready(());
            }
            // NB: We should get a new `Waker` on every `poll` in case the future migrates between executors.
            // cf. https://rust-lang.github.io/async-book/02_execution/03_wakeups.html
            let waker = cx.waker().clone();
            spawn_after(self.till_utc, async move { waker.wake() });
            Poll03::Pending
        }
    }

    #[test]
    fn test_timer() {
        let started = now_float();
        let ti = Timer::sleep(0.2);
        let delta = now_float() - started;
        assert!(delta < 0.04, "{}", delta);
        super::block_on(ti);
        let delta = now_float() - started;
        println!("time delta is {}", delta);
        assert!(delta > 0.2);
        assert!(delta < 0.4)
    }
}

#[cfg(not(feature = "native"))] pub mod executor;

/// Returns a JSON error HyRes on a failure.
#[macro_export]
macro_rules! try_h {
    ($e: expr) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => return $crate::rpc_err_response(500, &ERRL!("{}", err)),
        }
    };
}

/// Executes a GET request, returning the response status, headers and body.
pub fn slurp_url(url: &str) -> SlurpFut { wio::slurp_req(try_fus!(Request::builder().uri(url).body(Vec::new()))) }

#[test]
#[ignore]
fn test_slurp_req() {
    let (status, headers, body) = unwrap!(slurp_url("https://httpbin.org/get").wait());
    assert!(status.is_success(), format!("{:?} {:?} {:?}", status, headers, body));
}

/// Fetch URL by HTTPS and parse JSON response
pub fn fetch_json<T>(url: &str) -> Box<dyn Future<Item = T, Error = String>>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    Box::new(slurp_url(url).and_then(|result| {
        // try to parse as json with serde_json
        let result = try_s!(serde_json::from_slice(&result.2));

        Ok(result)
    }))
}

/// Send POST JSON HTTPS request and parse response
pub fn post_json<T>(url: &str, json: String) -> Box<dyn Future<Item = T, Error = String>>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    let request = try_fus!(Request::builder()
        .method("POST")
        .uri(url)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(json.into()));

    Box::new(wio::slurp_req(request).and_then(|result| {
        // try to parse as json with serde_json
        let result = try_s!(serde_json::from_slice(&result.2));

        Ok(result)
    }))
}

/// Wraps a JSON string into the `HyRes` RPC response future.
pub fn rpc_response<T>(status: u16, body: T) -> HyRes
where
    Vec<u8>: From<T>,
{
    let rf = match Response::builder()
        .status(status)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Vec::from(body))
    {
        Ok(r) => future::ok::<Response<Vec<u8>>, String>(r),
        Err(err) => future::err::<Response<Vec<u8>>, String>(ERRL!("{}", err)),
    };
    Box::new(rf)
}

#[derive(Serialize)]
struct ErrResponse {
    error: String,
}

/// Converts the given `err` message into the `{error: $err}` JSON string.
pub fn err_to_rpc_json_string(err: &str) -> String {
    let err = ErrResponse { error: err.to_owned() };
    json::to_string(&err).unwrap()
}

pub fn err_tp_rpc_json(error: String) -> Json { json::to_value(ErrResponse { error }).unwrap() }

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
type ExternalResourceSync<R> =
    Box<dyn Fn() -> Box<dyn Future<Item = R, Error = String> + Send + 'static> + Send + 'static>;

/// Memory space accessible to the `Future` tail spawned by the `RefreshedExternalResource`.
struct RerShelf<R: Send + 'static> {
    /// The time when the `Future` generated by `sync` has filled this shell.
    time: f64,
    /// Results of the `sync`-generated `Future`.
    result: Result<R, String>,
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
    listeners: Arc<Mutex<Vec<Task>>>,
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
    pub fn new(every_n_sec: f64, timeout_sec: f64, sync: ExternalResourceSync<R>) -> RefreshedExternalResource<R> {
        assert_eq!(size_of::<usize>(), 8);
        RefreshedExternalResource {
            sync: Mutex::new(sync),
            every_n_sec,
            timeout_sec: timeout_sec.max(every_n_sec),
            last_start: AtomicUsize::new(0f64.to_bits() as usize),
            shelf: Arc::new(Mutex::new(None)),
            listeners: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn add_listeners(&self, mut tasks: Vec<Task>) -> Result<(), String> {
        let mut listeners = try_s!(self.listeners.lock());
        listeners.append(&mut tasks);
        Ok(())
    }

    /// Performs the maintenance operations necessary to periodically refresh the resource.
    pub fn tick(&self) -> Result<(), String> {
        let now = now_float();
        let last_finish = match *try_s!(self.shelf.lock()) {
            Some(ref rer_shelf) => rer_shelf.time,
            None => 0.,
        };
        let last_start = f64::from_bits(self.last_start.load(Ordering::Relaxed) as u64);

        if now - last_start > self.timeout_sec || (last_finish > last_start && now - last_start > self.every_n_sec) {
            self.last_start.store(now.to_bits() as usize, Ordering::Relaxed);
            let sync = try_s!(self.sync.lock());
            let f = (*sync)();
            let shelf_tx = self.shelf.clone();
            let listeners = self.listeners.clone();
            let f = f.then(move |result| -> Result<(), ()> {
                let mut shelf = match shelf_tx.lock() {
                    Ok(l) => l,
                    Err(err) => {
                        log! ({"RefreshedExternalResource::tick] Can't lock the shelf: {}", err});
                        return Err(());
                    },
                };
                let shelf_time = match *shelf {
                    Some(ref r) => r.time,
                    None => 0.,
                };
                if now > shelf_time {
                    // This check prevents out-of-order shelf updates.
                    *shelf = Some(RerShelf {
                        time: now_float(),
                        result,
                    });
                    drop(shelf); // Don't hold the lock unnecessarily.
                    {
                        let mut listeners = match listeners.lock() {
                            Ok(l) => l,
                            Err(err) => {
                                log! ({"RefreshedExternalResource::tick] Can't lock the listeners: {}", err});
                                return Err(());
                            },
                        };
                        for task in listeners.drain(..) {
                            task.notify()
                        }
                    }
                }
                Ok(())
            });
            executor::spawn(f.compat().map(|_| ())); // Polls `f` in background.
        }

        Ok(())
    }

    /// The time, in seconds since UNIX epoch, when the refresh `Future` resolved.
    pub fn last_finish(&self) -> Result<f64, String> {
        Ok(match *try_s!(self.shelf.lock()) {
            Some(ref rer_shelf) => rer_shelf.time,
            None => 0.,
        })
    }

    pub fn with_result<V, F: FnMut(Option<&Result<R, String>>) -> Result<V, String>>(
        &self,
        mut cb: F,
    ) -> Result<V, String> {
        let shelf = try_s!(self.shelf.lock());
        match *shelf {
            Some(ref rer_shelf) => cb(Some(&rer_shelf.result)),
            None => cb(None),
        }
    }
}

/// From<io::Error> is required to be implemented by futures-timer timeout.
/// We can't implement it for String directly due to Rust restrictions.
/// So this solution looks like simplest at least for now. We have to remap errors to get proper type.
pub struct StringError(pub String);

impl From<std::io::Error> for StringError {
    fn from(e: std::io::Error) -> StringError { StringError(ERRL!("{}", e)) }
}

#[derive(Debug)]
pub struct QueuedCommand {
    pub response_sock: i32,
    pub stats_json_only: i32,
    pub queue_id: u32,
    pub msg: String,
    // retstrp: *mut *mut c_char,
}

/// Register an RPC command that came internally or from the peer-to-peer bus.
#[no_mangle]
#[cfg(feature = "native")]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn lp_queue_command_for_c(
    retstrp: *mut *mut c_char,
    buf: *mut c_char,
    response_sock: i32,
    stats_json_only: i32,
    queue_id: u32,
) {
    if !retstrp.is_null() {
        *retstrp = null_mut()
    }

    if buf.is_null() {
        panic!("!buf")
    }
    let msg = String::from(unwrap!(CStr::from_ptr(buf).to_str()));
    let _cmd = QueuedCommand {
        msg,
        queue_id,
        response_sock,
        stats_json_only,
    };
    panic!("We need a context ID");
    //unwrap! ((*COMMAND_QUEUE).0.send (cmd))
}

pub fn lp_queue_command(ctx: &mm_ctx::MmArc, msg: String) -> Result<(), String> {
    // If we're helping a WASM then leave a copy of the broadcast for them.
    if let Some(ref mut cq) = *try_s!(ctx.command_queueʰ.lock()) {
        // Monotonic increment.
        let now = if let Some(last) = cq.last() {
            (last.0 + 1).max(now_ms())
        } else {
            now_ms()
        };
        cq.push((now, msg.clone()))
    }

    let cmd = QueuedCommand {
        msg,
        queue_id: 0,
        response_sock: -1,
        stats_json_only: 0,
    };
    try_s!(ctx.command_queue.unbounded_send(cmd));
    Ok(())
}

pub fn var(name: &str) -> Result<String, String> {
    /// Obtains the environment variable `name` from the host, copying it into `rbuf`.
    /// Returns the length of the value copied to `rbuf` or -1 if there was an error.
    #[cfg(not(feature = "native"))]
    #[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
    extern "C" {
        pub fn host_env(name: *const c_char, nameˡ: i32, rbuf: *mut c_char, rcap: i32) -> i32;
    }

    #[cfg(feature = "native")]
    {
        match std::env::var(name) {
            Ok(v) => Ok(v),
            Err(_err) => ERR!("No {}", name),
        }
    }

    #[cfg(not(feature = "native"))]
    {
        // Get the environment variable from the host.
        use std::mem::zeroed;
        use std::str::from_utf8;

        let mut buf: [u8; 4096] = unsafe { zeroed() };
        let rc = unsafe {
            host_env(
                name.as_ptr() as *const c_char,
                name.len() as i32,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as i32,
            )
        };
        if rc <= 0 {
            return ERR!("No {}", name);
        }
        let s = try_s!(from_utf8(&buf[0..rc as usize]));
        Ok(String::from(s))
    }
}

pub fn block_on<F>(f: F) -> F::Output
where
    F: Future03,
{
    if var("TRACE_BLOCK_ON").map(|v| v == "true") == Ok(true) {
        let mut trace = String::with_capacity(4096);
        stack_trace(&mut stack_trace_frame, &mut |l| trace.push_str(l));
        log!("block_on at\n"(trace));
    }

    futures::executor::block_on(f)
}

use backtrace::SymbolName;
#[cfg(feature = "native")] pub use gstuff::{now_float, now_ms};

#[cfg(not(feature = "native"))]
pub fn now_ms() -> u64 {
    #[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
    extern "C" {
        pub fn date_now() -> f64;
    }
    unsafe { date_now() as u64 }
}
#[cfg(not(feature = "native"))]
pub fn now_float() -> f64 {
    use gstuff::duration_to_float;
    use std::time::Duration;
    duration_to_float(Duration::from_millis(now_ms()))
}

#[cfg(feature = "native")]
pub fn slurp(path: &dyn AsRef<Path>) -> Result<Vec<u8>, String> { Ok(gstuff::slurp(path)) }

#[cfg(not(feature = "native"))]
pub fn slurp(path: &dyn AsRef<Path>) -> Result<Vec<u8>, String> {
    use std::mem::MaybeUninit;

    #[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
    extern "C" {
        pub fn host_slurp(path_p: *const c_char, path_l: i32, rbuf: *mut c_char, rcap: i32) -> i32;
    }

    let path = try_s!(path.as_ref().to_str().ok_or("slurp: path not unicode"));
    let mut rbuf: [u8; 262144] = unsafe { MaybeUninit::uninit().assume_init() };
    let rc = unsafe {
        host_slurp(
            path.as_ptr() as *const c_char,
            path.len() as i32,
            rbuf.as_mut_ptr() as *mut c_char,
            rbuf.len() as i32,
        )
    };
    if rc < 0 {
        return ERR!("!host_slurp: {}", rc);
    }
    Ok(Vec::from(&rbuf[..rc as usize]))
}

#[cfg(feature = "native")]
pub fn temp_dir() -> PathBuf { env::temp_dir() }

#[cfg(not(feature = "native"))]
pub fn temp_dir() -> PathBuf {
    #[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
    extern "C" {
        pub fn temp_dir(rbuf: *mut c_char, rcap: i32) -> i32;
    }
    let mut buf: [u8; 4096] = unsafe { zeroed() };
    let rc = unsafe { temp_dir(buf.as_mut_ptr() as *mut c_char, buf.len() as i32) };
    if rc <= 0 {
        panic!("!temp_dir")
    }
    let path = unwrap!(std::str::from_utf8(&buf[0..rc as usize]));
    Path::new(path).into()
}

#[cfg(feature = "native")]
pub fn remove_file(path: &dyn AsRef<Path>) -> Result<(), String> {
    try_s!(fs::remove_file(path));
    Ok(())
}

#[cfg(not(feature = "native"))]
pub fn remove_file(path: &dyn AsRef<Path>) -> Result<(), String> {
    use std::os::raw::c_char;

    #[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
    extern "C" {
        pub fn host_rm(ptr: *const c_char, len: i32) -> i32;
    }

    let path = try_s!(path.as_ref().to_str().ok_or("Non-unicode path"));
    let rc = unsafe { host_rm(path.as_ptr() as *const c_char, path.len() as i32) };
    if rc != 0 {
        return ERR!("!host_rm: {}", rc);
    }
    Ok(())
}

#[cfg(feature = "native")]
pub fn write(path: &dyn AsRef<Path>, contents: &dyn AsRef<[u8]>) -> Result<(), String> {
    try_s!(fs::write(path, contents));
    Ok(())
}

#[cfg(not(feature = "native"))]
pub fn write(path: &dyn AsRef<Path>, contents: &dyn AsRef<[u8]>) -> Result<(), String> {
    use std::os::raw::c_char;

    #[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
    extern "C" {
        pub fn host_write(path_p: *const c_char, path_l: i32, ptr: *const c_char, len: i32) -> i32;
    }

    let path = try_s!(path.as_ref().to_str().ok_or("Non-unicode path"));
    let content = contents.as_ref();
    let rc = unsafe {
        host_write(
            path.as_ptr() as *const c_char,
            path.len() as i32,
            content.as_ptr() as *const c_char,
            content.len() as i32,
        )
    };
    if rc != 0 {
        return ERR!("!host_write: {}", rc);
    }
    Ok(())
}

/// Read a folder and return a list of files with their last-modified ms timestamps.
#[cfg(feature = "native")]
pub fn read_dir(dir: &dyn AsRef<Path>) -> Result<Vec<(u64, PathBuf)>, String> {
    let entries = try_s!(dir.as_ref().read_dir())
        .filter_map(|dir_entry| {
            let entry = match dir_entry {
                Ok(ent) => ent,
                Err(e) => {
                    log!("Error " (e) " reading from dir " (dir.as_ref().display()));
                    return None;
                },
            };

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    log!("Error " (e) " getting file " (entry.path().display()) " meta");
                    return None;
                },
            };

            let m_time = match metadata.modified() {
                Ok(time) => time,
                Err(e) => {
                    log!("Error " (e) " getting file " (entry.path().display()) " m_time");
                    return None;
                },
            };

            let lm = unwrap!(m_time.duration_since(UNIX_EPOCH), "!duration_since").as_millis();
            assert!(lm < u64::max_value() as u128);
            let lm = lm as u64;

            let path = entry.path();
            if path.extension() == Some(OsStr::new("json")) {
                Some((lm, path))
            } else {
                None
            }
        })
        .collect();

    Ok(entries)
}

#[cfg(not(feature = "native"))]
pub fn read_dir(dir: &dyn AsRef<Path>) -> Result<Vec<(u64, PathBuf)>, String> {
    use std::mem::MaybeUninit;

    #[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
    extern "C" {
        pub fn host_read_dir(path_p: *const c_char, path_l: i32, rbuf: *mut c_char, rcap: i32) -> i32;
    }

    let path = try_s!(dir.as_ref().to_str().ok_or("read_dir: dir path not unicode"));
    let mut rbuf: [u8; 262144] = unsafe { MaybeUninit::uninit().assume_init() };
    let rc = unsafe {
        host_read_dir(
            path.as_ptr() as *const c_char,
            path.len() as i32,
            rbuf.as_mut_ptr() as *mut c_char,
            rbuf.len() as i32,
        )
    };
    if rc <= 0 {
        return ERR!("!host_read_dir: {}", rc);
    }
    let jens: Vec<(u64, String)> = try_s!(json::from_slice(&rbuf[..rc as usize]));

    let mut entries: Vec<(u64, PathBuf)> = Vec::with_capacity(jens.len());
    for (lm, name) in jens {
        let path = dir.as_ref().join(name);
        entries.push((lm, path))
    }

    Ok(entries)
}

/// If the `MM_LOG` variable is present then tries to open that file.  
/// Prints a warning to `stdout` if there's a problem opening the file.  
/// Returns `None` if `MM_LOG` variable is not present or if the specified path can't be opened.
fn open_log_file() -> Option<fs::File> {
    let mm_log = match var("MM_LOG") {
        Ok(v) => v,
        Err(_) => return None,
    };

    // For security reasons we want the log path to always end with ".log".
    if !mm_log.ends_with(".log") {
        println!("open_log_file] MM_LOG doesn't end with '.log'");
        return None;
    }

    match fs::OpenOptions::new().append(true).create(true).open(&mm_log) {
        Ok(f) => Some(f),
        Err(err) => {
            println!("open_log_file] Can't open {}: {}", mm_log, err);
            None
        },
    }
}

#[cfg(feature = "native")]
pub fn writeln(line: &str) {
    use std::panic::catch_unwind;

    lazy_static! {
        static ref LOG_FILE: Mutex<Option<fs::File>> = Mutex::new(open_log_file());
    }

    // `catch_unwind` protects the tests from error
    //
    //     thread 'CORE' panicked at 'cannot access stdout during shutdown'
    //
    // (which might be related to https://github.com/rust-lang/rust/issues/29488).
    let _ = catch_unwind(|| {
        if let Ok(mut log_file) = LOG_FILE.lock() {
            if let Some(ref mut log_file) = *log_file {
                let _ = witeln!(log_file, (line));
                return;
            }
        }
        println!("{}", line);
    });
}

#[cfg(not(feature = "native"))]
const fn make_tail() -> [u8; 0x10000] { [0; 0x10000] }

#[cfg(not(feature = "native"))]
static mut PROCESS_LOG_TAIL: [u8; 0x10000] = make_tail();
#[cfg(not(feature = "native"))]
static TAIL_CUR: Atomic<usize> = Atomic::new(0);

#[cfg(all(not(feature = "native"), not(feature = "w-bindgen")))]
pub fn writeln(line: &str) {
    use std::ffi::CString;

    extern "C" {
        pub fn console_log(ptr: *const c_char, len: i32);
    }
    let lineᶜ = unwrap!(CString::new(line));
    unsafe { console_log(lineᶜ.as_ptr(), line.len() as i32) }

    // Keep a tail of the log in RAM for the integration tests.
    unsafe {
        if line.len() < PROCESS_LOG_TAIL.len() {
            let posⁱ = TAIL_CUR.load(Ordering::Relaxed);
            let posⱼ = posⁱ + line.len();
            let (posˢ, posⱼ) = if posⱼ > PROCESS_LOG_TAIL.len() {
                (0, line.len())
            } else {
                (posⁱ, posⱼ)
            };
            if TAIL_CUR
                .compare_exchange(posⁱ, posⱼ, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                for (cur, ix) in (posˢ..posⱼ).zip(0..line.len()) {
                    PROCESS_LOG_TAIL[cur] = line.as_bytes()[ix]
                }
            }
        }
    }
}

#[cfg(all(not(feature = "native"), feature = "w-bindgen"))]
pub fn writeln(line: &str) {
    use web_sys::console;
    console::log_1(&line.into());
}

/// Set up a panic hook that prints the panic location and the message.
/// (The default Rust handler doesn't have the means to print the message.
///  Note that we're also getting the stack trace from Node.js and rustfilt).
#[cfg(not(feature = "native"))]
#[no_mangle]
pub extern "C" fn set_panic_hook() {
    use gstuff::filename;
    use std::panic::{set_hook, PanicInfo};

    set_hook(Box::new(|info: &PanicInfo| {
        let mut msg = String::with_capacity(256);
        let _ = wite!(&mut msg, (info));
        writeln(&msg)
    }))
}

pub fn small_rng() -> SmallRng { SmallRng::seed_from_u64(now_ms()) }

/// Ask the WASM host to send HTTP request to the native helpers.
/// Returns request ID used to wait for the reply.
#[cfg(not(feature = "native"))]
#[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
extern "C" {
    fn http_helper_if(helper: *const u8, helper_len: i32, payload: *const u8, payload_len: i32, timeout_ms: i32)
        -> i32;
}

#[cfg(not(feature = "native"))]
#[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
extern "C" {
    /// Check with the WASM host to see if the given HTTP request is ready.
    ///
    /// Returns the amount of bytes copied to rbuf,  
    /// or `-1` if the request is not yet finished,  
    /// or `0 - amount of bytes` in case the intended size was larger than the `rcap`.
    ///
    /// The bytes copied to rbuf are in the bencode format,
    /// `{status: $number, ct: $bytes, cs: $bytes, body: $bytes}`
    /// (the `HelperResponse`).
    ///
    /// * `helper_request_id` - Request ID previously returned by `http_helper_if`.
    /// * `rbuf` - The buffer to copy the response payload into if the request is finished.
    /// * `rcap` - The size of the `rbuf` buffer.
    pub fn http_helper_check(helper_request_id: i32, rbuf: *mut u8, rcap: i32) -> i32;
}

lazy_static! {
    /// Maps helper request ID to the corresponding Waker,
    /// allowing WASM host to wake the `HelperReply`.
    static ref HELPER_REQUESTS: Mutex<HashMap<i32, Waker>> = Mutex::new (HashMap::new());
}

/// WASM host invokes this method to signal the readiness of the HTTP request.
#[no_mangle]
#[cfg(not(feature = "native"))]
pub extern "C" fn http_ready(helper_request_id: i32) {
    let mut helper_requests = unwrap!(HELPER_REQUESTS.lock());
    if let Some(waker) = helper_requests.remove(&helper_request_id) {
        waker.wake()
    }
}

#[derive(Deserialize, Debug)]
pub struct HelperResponse {
    pub status: u32,
    #[serde(rename = "ct")]
    pub content_type: Option<ByteBuf>,
    #[serde(rename = "cs")]
    pub checksum: Option<ByteBuf>,
    pub body: ByteBuf,
}
/// Mostly used to log the errors coming from the other side.
impl fmt::Display for HelperResponse {
    fn fmt(&self, ft: &mut fmt::Formatter) -> fmt::Result {
        wite! (ft, (self.status) ", " (binprint (&self.body, b'.')))
    }
}

#[cfg(not(feature = "native"))]
pub async fn helperᶜ(helper: &'static str, args: Vec<u8>) -> Result<Vec<u8>, String> {
    let helper_request_id = unsafe {
        http_helper_if(
            helper.as_ptr(),
            helper.len() as i32,
            args.as_ptr(),
            args.len() as i32,
            9999,
        )
    };

    struct HelperReply {
        helper: &'static str,
        helper_request_id: i32,
    }
    impl std::future::Future for HelperReply {
        type Output = Result<Vec<u8>, String>;
        fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll03<Self::Output> {
            let mut buf: [u8; 65535] = unsafe { std::mem::MaybeUninit::uninit().assume_init() };
            let rlen = unsafe { http_helper_check(self.helper_request_id, buf.as_mut_ptr(), buf.len() as i32) };
            if rlen < -1 {
                // Response is larger than capacity.
                return Poll03::Ready(ERR!("Helper result is too large ({})", rlen));
            }
            if rlen >= 0 {
                return Poll03::Ready(Ok(Vec::from(&buf[0..rlen as usize])));
            }

            // NB: Need a fresh waker each time `Pending` is returned, to support switching tasks.
            // cf. https://rust-lang.github.io/async-book/02_execution/03_wakeups.html
            let waker = cx.waker().clone();
            unwrap!(HELPER_REQUESTS.lock()).insert(self.helper_request_id, waker);

            Poll03::Pending
        }
    }
    impl Drop for HelperReply {
        fn drop(&mut self) { unwrap!(HELPER_REQUESTS.lock()).remove(&self.helper_request_id); }
    }
    let rv: Vec<u8> = try_s!(
        HelperReply {
            helper,
            helper_request_id
        }
        .await
    );
    let rv: HelperResponse = try_s!(bdecode(&rv));
    if rv.status != 200 {
        return ERR!("!{}: {}", helper, rv);
    }
    // TODO: Check `rv.checksum` if present.
    Ok(rv.body.into_vec())
}

#[derive(Serialize, Deserialize)]
pub struct BroadcastP2pMessageArgs {
    pub ctx: u32,
    pub msg: String,
}

#[derive(Debug, Clone)]
/// Ordered from low to height inclusive range.
pub struct OrdRange<T>(RangeInclusive<T>);

impl<T> Deref for OrdRange<T> {
    type Target = RangeInclusive<T>;

    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<T: PartialOrd> OrdRange<T> {
    /// Construct the OrderedRange from the start-end pair.
    pub fn new(start: T, end: T) -> Result<Self, String> {
        if start > end {
            return Err("".into());
        }

        Ok(Self(start..=end))
    }
}

impl<T: Copy> OrdRange<T> {
    /// Flatten a start-end pair into the vector.
    pub fn flatten(&self) -> Vec<T> { vec![*self.start(), *self.end()] }
}

/// Invokes callback `cb_id` in the WASM host, passing a `(ptr,len)` string to it.
#[cfg(not(feature = "native"))]
#[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
extern "C" {
    pub fn call_back(cb_id: i32, ptr: *const c_char, len: i32);
}
pub mod for_tests;

fn without_trailing_zeroes(decimal: &str, dot: usize) -> &str {
    let mut pos = decimal.len() - 1;
    loop {
        let ch = decimal.as_bytes()[pos];
        if ch != b'0' {
            break &decimal[0..=pos];
        }
        if pos == dot {
            break &decimal[0..pos];
        }
        pos -= 1
    }
}

/// Round half away from zero (aka commercial rounding).
pub fn round_to(bd: &BigDecimal, places: u8) -> String {
    // Normally we'd do
    //
    //     let divisor = pow (10, places)
    //     round (self * divisor) / divisor
    //
    // But we don't have a `round` function in `BigDecimal` at present, so we're on our own.

    let bds = format!("{}", bd);
    let bda = bds.as_bytes();
    let dot = bda.iter().position(|&ch| ch == b'.');
    let dot = match dot {
        Some(dot) => dot,
        None => return bds,
    };

    if bda.len() - dot <= places as usize {
        return String::from(without_trailing_zeroes(&bds, dot));
    }

    let mut pos = bda.len() - 1;
    let mut ch = bda[pos];
    let mut prev_digit = 0;
    loop {
        let digit = ch - b'0';
        let rounded = if prev_digit > 5 { digit + 1 } else { digit };
        //println! ("{} at {}: prev_digit {}, digit {}, rounded {}", bds, pos, prev_digit, digit, rounded);

        if pos < dot {
            //println! ("{}, pos < dot, stopping at pos {}", bds, pos);
            let mut integer: i64 = unwrap!((&bds[0..=pos]).parse());
            if prev_digit > 5 {
                if bda[0] == b'-' {
                    integer = unwrap!(integer.checked_sub(1))
                } else {
                    integer = unwrap!(integer.checked_add(1))
                }
            }
            return format!("{}", integer);
        }

        if pos == dot + places as usize && rounded < 10 {
            //println! ("{}, stopping at pos {}", bds, pos);
            break format!("{}{}", &bds[0..pos], rounded);
        }

        pos -= 1;
        if pos == dot {
            pos -= 1
        } // Skip over the dot.
        ch = bda[pos];
        prev_digit = rounded
    }
}

#[test]
fn test_round_to() {
    assert_eq!(round_to(&BigDecimal::from(0.999), 2), "1");
    assert_eq!(round_to(&BigDecimal::from(-0.999), 2), "-1");

    assert_eq!(round_to(&BigDecimal::from(10.999), 2), "11");
    assert_eq!(round_to(&BigDecimal::from(-10.999), 2), "-11");

    assert_eq!(round_to(&BigDecimal::from(99.9), 1), "99.9");
    assert_eq!(round_to(&BigDecimal::from(-99.9), 1), "-99.9");

    assert_eq!(round_to(&BigDecimal::from(99.9), 0), "100");
    assert_eq!(round_to(&BigDecimal::from(-99.9), 0), "-100");

    let ouch = BigDecimal::from(1) / BigDecimal::from(7);
    assert_eq!(round_to(&ouch, 3), "0.143");

    let ouch = BigDecimal::from(1) / BigDecimal::from(3);
    assert_eq!(round_to(&ouch, 0), "0");
    assert_eq!(round_to(&ouch, 1), "0.3");
    assert_eq!(round_to(&ouch, 2), "0.33");
    assert_eq!(round_to(&ouch, 9), "0.333333333");

    assert_eq!(round_to(&BigDecimal::from(0.123), 99), "0.123");
    assert_eq!(round_to(&BigDecimal::from(-0.123), 99), "-0.123");

    assert_eq!(round_to(&BigDecimal::from(0), 99), "0");
    assert_eq!(round_to(&BigDecimal::from(-0), 99), "0");

    assert_eq!(round_to(&BigDecimal::from(0.123), 0), "0");
    assert_eq!(round_to(&BigDecimal::from(-0.123), 0), "0");

    assert_eq!(round_to(&BigDecimal::from(0), 0), "0");
    assert_eq!(round_to(&BigDecimal::from(-0), 0), "0");
}

#[cfg(feature = "native")]
pub fn new_uuid() -> Uuid { Uuid::new_v4() }

#[cfg(not(feature = "native"))]
pub fn new_uuid() -> Uuid {
    use rand::RngCore;
    use uuid::{Builder, Variant, Version};

    let mut rng = small_rng();
    let mut bytes = [0; 16];

    rng.fill_bytes(&mut bytes);

    Builder::from_bytes(bytes)
        .set_variant(Variant::RFC4122)
        .set_version(Version::Random)
        .build()
}

pub fn first_char_to_upper(input: &str) -> String {
    let mut v: Vec<char> = input.chars().collect();
    if let Some(c) = v.first_mut() {
        c.make_ascii_uppercase()
    }
    v.into_iter().collect()
}

#[test]
fn test_first_char_to_upper() {
    assert_eq!("", first_char_to_upper(""));
    assert_eq!("K", first_char_to_upper("k"));
    assert_eq!("Komodo", first_char_to_upper("komodo"));
    assert_eq!(".komodo", first_char_to_upper(".komodo"));
}

pub fn json_dir_entries(path: &dyn AsRef<Path>) -> Result<Vec<DirEntry>, String> {
    Ok(try_s!(path.as_ref().read_dir())
        .filter_map(|dir_entry| {
            let entry = match dir_entry {
                Ok(ent) => ent,
                Err(e) => {
                    log!("Error " (e) " reading from dir " (path.as_ref().display()));
                    return None;
                },
            };

            if entry.path().extension() == Some(OsStr::new("json")) {
                Some(entry)
            } else {
                None
            }
        })
        .collect())
}

/// Calculates the median of the set represented as slice
pub fn median<T: Add<Output = T> + Div<Output = T> + Copy + From<u8> + Ord>(input: &mut [T]) -> Option<T> {
    // median is undefined on empty sets
    if input.is_empty() {
        return None;
    }
    input.sort();
    let median_index = input.len() / 2;
    if input.len() % 2 == 0 {
        Some((input[median_index - 1] + input[median_index]) / T::from(2u8))
    } else {
        Some(input[median_index])
    }
}

#[test]
fn test_median() {
    let mut input = [3, 2, 1];
    let expected = Some(2u32);
    let actual = median(&mut input);
    assert_eq!(expected, actual);

    let mut input = [3, 1];
    let expected = Some(2u32);
    let actual = median(&mut input);
    assert_eq!(expected, actual);

    let mut input = [1, 3, 2, 8, 10];
    let expected = Some(3u32);
    let actual = median(&mut input);
    assert_eq!(expected, actual);
}

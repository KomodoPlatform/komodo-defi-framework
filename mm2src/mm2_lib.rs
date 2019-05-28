#![feature(non_ascii_idents)]
#![feature(drain_filter)]

#[macro_use] extern crate common;
#[allow(unused_imports)]
#[macro_use] extern crate duct;
#[macro_use] extern crate fomat_macros;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serialization_derive;
#[macro_use] extern crate unwrap;

#[path = "mm2.rs"]
mod mm2;

use crate::common::mm_ctx::MmArc;
use crate::common::log::LOG_OUTPUT;
use gstuff::any_to_str;
use libc::c_char;
use std::ffi::{CStr};
use std::panic::catch_unwind;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;

static LP_MAIN_RUNNING: AtomicBool = AtomicBool::new (false);
static CTX: AtomicU32 = AtomicU32::new (0);

#[derive(Debug)]
enum MainErr {
    Ok = 0,
    AlreadyRuns,
    ConfIsNull,
    ConfNotUtf8,
    NoOutputLock,
    CantThread
}

/// Starts the MM2 in a detached singleton thread.
#[no_mangle]
pub extern fn mm2_main (
  conf: *const c_char, log_cb: extern fn (line: *const c_char)) -> i8 {
    macro_rules! log {
        ($($args: tt)+) => {{
            let msg = fomat! ("mm2_lib:" ((line!())) "] " $($args)+ '\0');
            log_cb (msg.as_ptr() as *const c_char);
        }}
    }
    macro_rules! eret {
        ($rc: expr, $($args: tt)+) => {{log! ("error " ($rc as i8) ", " [$rc] ": " $($args)+); return $rc as i8}};
        ($rc: expr) => {{log! ("error " ($rc as i8) ", " [$rc]); return $rc as i8}};
    }

    if LP_MAIN_RUNNING.load (Ordering::Relaxed) {eret! (MainErr::AlreadyRuns)}

    if conf.is_null() {eret! (MainErr::ConfIsNull)}
    let conf = unsafe {CStr::from_ptr (conf)};
    let conf = match conf.to_str() {Ok (s) => s, Err (e) => eret! (MainErr::ConfNotUtf8, (e))};
    let conf = conf.to_owned();

    {
        let mut log_output = match LOG_OUTPUT.lock() {Ok (l) => l, Err (e) => eret! (MainErr::NoOutputLock, (e))};
        *log_output = Some (log_cb);
    }

    let rc = thread::Builder::new().name ("lp_main".into()) .spawn (move || {
        if LP_MAIN_RUNNING.compare_and_swap (false, true, Ordering::Relaxed) {
            log! ("lp_main already started!");
            return
        }
        let ctx_cb = &|ctx| CTX.store (ctx, Ordering::Relaxed);
        match catch_unwind (move || mm2::run_lp_main (Some (&conf), ctx_cb)) {
            Ok (Ok (_)) => log! ("run_lp_main finished"),
            Ok (Err (err)) => log! ("run_lp_main error: " (err)),
            Err (err) => log! ("run_lp_main panic: " [any_to_str (&*err)])
        };
        LP_MAIN_RUNNING.store (false, Ordering::Relaxed)
    });
    if let Err (e) = rc {eret! (MainErr::CantThread, (e))}
    MainErr::Ok as i8
}

/// Checks if the MM2 singleton thread is currently running or not.  
/// 0 .. not running.  
/// 1 .. running, but no context yet.  
/// 2 .. context, but no RPC yet.  
/// 3 .. RPC is up.
#[no_mangle]
pub extern fn mm2_main_status() -> i8 {
    if LP_MAIN_RUNNING.load (Ordering::Relaxed) {
        let ctx = CTX.load (Ordering::Relaxed);
        if ctx != 0 {
            if let Ok (ctx) = MmArc::from_ffi_handle (ctx) {
                if ctx.rpc_started.load (Ordering::Relaxed) {
                    3
                } else {2}
            } else {2}
        } else {1}
    } else {0}
}

/// Run a few hand-picked tests.  
/// 
/// The tests are wrapped into a library method in order to run them in such embedded environments
/// where running "cargo test" is not an easy option.
/// 
/// MM2 is mostly used as a library in environments where we can't simpy run it as a separate process
/// and we can't spawn multiple MM2 instances in the same process YET
/// therefore our usual process-spawning tests can not be used here.
/// 
/// Returns the `torch` (as in Olympic flame torch) if the tests have passed. Panics otherwise.
#[no_mangle]
pub extern fn mm2_test (torch: i32, log_cb: extern fn (line: *const c_char)) -> i32 {
    if let Ok (mut log_output) = LOG_OUTPUT.lock() {
        *log_output = Some (log_cb);
    } else {
        panic! ("Can't lock LOG_OUTPUT")
    }

    log! ("test_status…");
    common::log::tests::test_status();

    log! ("peers_dht…");
    peers::peers_tests::peers_dht();

    log! ("peers_direct_send…");
    peers::peers_tests::peers_direct_send();

    log! ("peers_http_fallback_kv…");
    peers::peers_tests::peers_http_fallback_kv();

    log! ("peers_http_fallback_recv…");
    peers::peers_tests::peers_http_fallback_recv();

    torch
}

#![feature(non_ascii_idents)]
#![feature(drain_filter)]
#![feature(integer_atomics)]
#![feature(async_await)]

#![cfg_attr(not(feature = "native"), allow(unused_imports))]

#[macro_use] extern crate common;
#[macro_use] extern crate enum_primitive_derive;
#[macro_use] extern crate fomat_macros;
#[macro_use] extern crate gstuff;
//#[macro_use] extern crate lazy_static;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serialization_derive;
#[macro_use] extern crate unwrap;

#[path = "mm2.rs"]
mod mm2;

use crate::common::mm_ctx::MmArc;
#[cfg(feature = "native")]
use crate::common::log::LOG_OUTPUT;
use futures::Future;
use futures03::executor::block_on;
use gstuff::{any_to_str, now_float};
#[cfg(feature = "native")]
use libc::c_char;
use num_traits::FromPrimitive;
use serde_json::{self as json};
use std::ffi::{CStr, CString};
use std::panic::catch_unwind;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

static LP_MAIN_RUNNING: AtomicBool = AtomicBool::new (false);
static CTX: AtomicU32 = AtomicU32::new (0);

#[derive(Debug, PartialEq, Primitive)]
enum MainErr {
    Ok = 0,
    AlreadyRuns = 1,
    ConfIsNull = 2,
    ConfNotUtf8 = 3,
    NoOutputLock = 4,
    CantThread = 5
}

/// Starts the MM2 in a detached singleton thread.
#[no_mangle]
#[cfg(feature = "native")]
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
    CTX.store (0, Ordering::Relaxed);  // Remove the old context ID during restarts.

    if conf.is_null() {eret! (MainErr::ConfIsNull)}
    let conf = unsafe {CStr::from_ptr (conf)};
    let conf = match conf.to_str() {Ok (s) => s, Err (e) => eret! (MainErr::ConfNotUtf8, (e))};
    let conf = conf.to_owned();

    #[cfg(feature = "native")] {
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
                if ctx.rpc_started.copy_or (false) {
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
#[cfg(feature = "native")]
pub extern fn mm2_test (torch: i32, log_cb: extern fn (line: *const c_char)) -> i32 {
    #[cfg(feature = "native")] {
        if let Ok (mut log_output) = LOG_OUTPUT.lock() {
            *log_output = Some (log_cb);
        } else {
            panic! ("Can't lock LOG_OUTPUT")
        }
    }

    static RUNNING: AtomicBool = AtomicBool::new (false);
    if RUNNING.compare_and_swap (false, true, Ordering::Relaxed) != false {
        log! ("mm2_test] Running already!");
        return -1
    }

    // #402: Stop the MM in order to test the library restart.
    let prev = if LP_MAIN_RUNNING.load (Ordering::Relaxed) {
        let ctx_id = CTX.load (Ordering::Relaxed);
        log! ("mm2_test] Stopping MM instance " (ctx_id) "…");
        let ctx = match MmArc::from_ffi_handle (ctx_id) {
            Ok (ctx) => ctx,
            Err (err) => {log! ("mm2_test] Invalid CTX? !from_ffi_handle: " (err)); return -1}
        };
        let conf = unwrap! (json::to_string (&ctx.conf));
        let hy_res = mm2::rpc::lp_commands::stop (ctx);
        let r = match hy_res.wait() {Ok (r) => r, Err (err) => {log! ("mm2_test] !stop: " (err)); return -1}};
        if !r.status().is_success() {log! ("mm2_test] stop status " (r.status())); return -1}

        // Wait for `LP_MAIN_RUNNING` to flip.
        let since = now_float();
        loop {
            thread::sleep (Duration::from_millis (100));
            if !LP_MAIN_RUNNING.load (Ordering::Relaxed) {break}
            if now_float() - since > 60. {log! ("mm2_test] LP_MAIN_RUNNING won't flip"); return -1}
        }

        Some ((ctx_id, conf))
    } else {None};

    // The global stop flag should be zeroed in order for some of the tests to work.
    let grace = 5;  // Grace time for late threads to discover the stop flag before we reset it.
    thread::sleep (Duration::from_secs (grace));

    // NB: We have to catch the panic because the error isn't logged otherwise.
    // (In the release mode the `ud2` op will trigger a crash or debugger on panic
    // but we don't have debugging symbols in the Rust code then).
    let rc = catch_unwind (|| {
        log! ("mm2_test] test_status…");
        common::log::tests::test_status();

        log! ("mm2_test] peers_dht…");
        block_on (peers::peers_tests::peers_dht());

        #[cfg(feature = "native")] {
            log! ("mm2_test] peers_direct_send…");
            peers::peers_tests::peers_direct_send();
        }

        log! ("mm2_test] peers_http_fallback_kv…");
        peers::peers_tests::peers_http_fallback_kv();

        log! ("mm2_test] peers_http_fallback_recv…");
        peers::peers_tests::peers_http_fallback_recv();
    });

    if let Err (err) = rc {
        log! ("mm2_test] There was an error: " (any_to_str (&*err) .unwrap_or ("-")));
        return -1
    }

    // #402: Restart the MM.
    if let Some ((prev_ctx_id, conf)) = prev {
        log! ("mm2_test] Restarting MM…");
        let confᶜ = unwrap! (CString::new (&conf[..]));
        let rc = mm2_main (confᶜ.as_ptr(), log_cb);
        let rc = unwrap! (MainErr::from_i8 (rc));
        if rc != MainErr::Ok {log! ("!mm2_main: " [rc]); return -1}

        // Wait for the new MM instance to allocate context.
        let since = now_float();
        loop {
            thread::sleep (Duration::from_millis (10));
            if LP_MAIN_RUNNING.load (Ordering::Relaxed) && CTX.load (Ordering::Relaxed) != 0 {break}
            if now_float() - since > 60.0 {log! ("mm2_test] Won't start"); return -1}
        }

        let ctx_id = CTX.load (Ordering::Relaxed);
        if ctx_id == prev_ctx_id {log! ("mm2_test] Context ID is the same"); return -1}
        log! ("mm2_test] New MM instance " (ctx_id) " started");
    }

    RUNNING.store (false, Ordering::Relaxed);
    log! ("mm2_test] All done, passing the torch.");
    torch
}

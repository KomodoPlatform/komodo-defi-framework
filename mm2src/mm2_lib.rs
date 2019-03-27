#![feature(non_ascii_idents)]

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

use crate::common::log::LOG_OUTPUT;
use crate::common::lp;
use libc::c_char;
use std::fs;
use std::ffi::{CStr, CString};
use std::io::Cursor;
use std::mem::transmute;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

static MM2_RUNNING: AtomicBool = AtomicBool::new (false);

#[derive(Debug)]
enum MainErr {
    Ok = 0,
    AlreadyRuns,
    ConfIsNull,
    ConfNotUtf8,
    WrDirNotUtf8,
    WrDirTooLong,
    WrDirNotDir,
    NoOutputLock,
    NilInErr,
    NotImplemented,
    CantThread
}

/// Starts the MM2 in a detached singleton thread.
#[no_mangle]
pub extern fn mm2_main (
  conf: *const c_char, wr_dir: *const c_char, log_cb: extern fn (line: *const c_char)) -> i8 {
    macro_rules! eret {
        (@b $rc: expr, $($args: tt)+) => {{
            let emsg = fomat! ("mm2_lib:" ((line!())) "] " $($args)+ '\0');
            log_cb (emsg.as_ptr() as *const c_char);
            return $rc as i8
        }};
        ($rc: expr, $($args: tt)+) => {eret! (@b $rc, [$rc] ": " $($args)+)};
        ($rc: expr) => {eret! (@b $rc, [$rc])};
    }

    if MM2_RUNNING.load (Ordering::Relaxed) {eret! (MainErr::AlreadyRuns)}

    if conf.is_null() {eret! (MainErr::ConfIsNull)}
    let conf = unsafe {CStr::from_ptr (conf)};
    let conf = match conf.to_str() {Ok (s) => s, Err (e) => eret! (MainErr::ConfNotUtf8, (e))};

    if !wr_dir.is_null() {
        // Use `wr_dir` as the default location for "DB".
        let wr_dir = unsafe {CStr::from_ptr (wr_dir)};
        let wr_dir = match wr_dir.to_str() {Ok (s) => s, Err (e) => eret! (MainErr::WrDirNotUtf8, (e))};
        let _ = fs::create_dir (wr_dir);
        if !Path::new (wr_dir) .is_dir() {eret! (MainErr::WrDirNotDir)}
        let global: &mut [c_char] = unsafe {&mut lp::GLOBAL_DBDIR[..]};
        let global: &mut [u8] = unsafe {transmute (global)};
        let mut cur = Cursor::new (global);
        use std::io::Write;
        if write! (&mut cur, "{}\0", wr_dir) .is_err() {
            unsafe {lp::GLOBAL_DBDIR[0] = 0}
            eret! (MainErr::WrDirTooLong)
        }
    }

    {
        let mut log_output = match LOG_OUTPUT.lock() {Ok (l) => l, Err (e) => eret! (MainErr::NoOutputLock, (e))};
        *log_output = Some (log_cb);
    }

    let rc = thread::Builder::new().name ("lp_main".into()) .spawn (move || {
    });
    if let Err (_) = rc {eret! (MainErr::CantThread)}

    if let Err (err) = mm2::run_lp_main (conf) {
        let line = fomat! ("run_lp_main error: " (err));
        let line = match CString::new (line) {Ok (cs) => cs, Err (_) => eret! (MainErr::NilInErr)};
        log_cb (line.as_ptr());
    }
    eret! (MainErr::NotImplemented)  // Singleton thread not implemented yet.
}

/// Checks if the MM2 singleton thread is currently running (1) or not (0).
#[no_mangle]
pub extern fn mm2_main_status() -> i8 {
    if MM2_RUNNING.load (Ordering::Relaxed) {1} else {0}
}

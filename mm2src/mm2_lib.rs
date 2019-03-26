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
use libc::c_char;
use std::ffi::{CStr, CString};

enum MainErr {
    Ok = 0,
    ConfIsNull,
    ConfNotUtf8,
    NoOutputLock,
    NilInErr,
    NotImplemented
}

/// Starts the MM2 in a detached singleton thread.
#[no_mangle]
pub extern fn mm2_main (conf: *const c_char, log_cb: extern fn (line: *const c_char)) -> i8 {
    log_cb (b"mm2_main] hi!\0".as_ptr() as *const c_char);  // Delme. Testing the logging callback.
    if conf.is_null() {return MainErr::ConfIsNull as i8}
    let conf = unsafe {CStr::from_ptr (conf)};
    let conf = match conf.to_str() {Ok (s) => s, Err (_) => return MainErr::ConfNotUtf8 as i8};

    {
        let mut log_output = match LOG_OUTPUT.lock() {Ok (l) => l, Err (_) => return MainErr::NoOutputLock as i8};
        *log_output = Some (log_cb);
    }

    if let Err (err) = mm2::run_lp_main (conf) {
        let line = fomat! ("run_lp_main error: " (err));
        let line = match CString::new (line) {Ok (cs) => cs, Err (_) => return MainErr::NilInErr as i8};
        log_cb (line.as_ptr());
    }
    MainErr::NotImplemented as i8  // Singleton thread not implemented yet.
}

/// Checks if the MM2 singleton thread is currently running.
#[no_mangle]
pub extern fn mm2_main_status() -> i8 {
    // TODO
    -1  // Not implemented yet.
}

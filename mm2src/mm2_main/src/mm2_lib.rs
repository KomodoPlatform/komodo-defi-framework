#![allow(uncommon_codepoints)]
#![feature(async_closure)]
#![feature(drain_filter)]
#![feature(hash_raw_entry)]
#![feature(integer_atomics)]
#![recursion_limit = "512"]
#![cfg_attr(target_arch = "wasm32", allow(unused_imports))]

#[macro_use] extern crate common;
#[macro_use] extern crate enum_primitive_derive;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serialization_derive;
#[macro_use] extern crate ser_error_derive;

#[path = "mm2.rs"] mod mm2;

#[cfg(not(target_arch = "wasm32"))]
#[path = "mm2_lib/mm2_native_lib.rs"]
mod mm2_native_lib;

#[cfg(target_arch = "wasm32")]
#[path = "mm2_lib/mm2_wasm_lib.rs"]
mod mm2_wasm_lib;

use mm2_core::mm_ctx::MmArc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(target_arch = "wasm32")] use wasm_bindgen::prelude::*;

static LP_MAIN_RUNNING: AtomicBool = AtomicBool::new(false);
static CTX: AtomicU32 = AtomicU32::new(0);

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub enum MainStatus {
    /// MM2 is not running yet.
    NotRunning = 0,
    /// MM2 is running, but no context yet.
    NoContext = 1,
    /// MM2 is running, but no RPC yet.
    NoRpc = 2,
    /// MM2's RPC is up.
    RpcIsUp = 3,
}

/// Checks if the MM2 singleton thread is currently running or not.
pub fn mm2_status() -> MainStatus {
    if !LP_MAIN_RUNNING.load(Ordering::Relaxed) {
        return MainStatus::NotRunning;
    }

    let ctx = CTX.load(Ordering::Relaxed);
    if ctx == 0 {
        return MainStatus::NoContext;
    }

    let ctx = match MmArc::from_ffi_handle(ctx) {
        Ok(ctx) => ctx,
        Err(_) => return MainStatus::NoRpc,
    };

    if ctx.rpc_started.copy_or(false) {
        MainStatus::RpcIsUp
    } else {
        MainStatus::NoRpc
    }
}

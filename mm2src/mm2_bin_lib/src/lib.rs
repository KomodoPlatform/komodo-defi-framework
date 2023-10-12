use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use enum_primitive_derive::Primitive;
use mm2_core::mm_ctx::MmArc;
use mm2_main::mm2::lp_dispatcher::{dispatch_lp_event, StopCtxEvent};
#[cfg(target_arch = "wasm32")] use wasm_bindgen::prelude::*;

#[cfg(not(target_arch = "wasm32"))] mod mm2_native_lib;
#[cfg(target_arch = "wasm32")] mod mm2_wasm_lib;

const MM_VERSION: &str = env!("MM_VERSION");
const MM_DATETIME: &str = env!("MM_DATETIME");

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
fn mm2_status() -> MainStatus {
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

enum PrepareForStopResult {
    CanBeStopped(MmArc),
    /// Please note that the status is not always an error.
    /// [`StopStatus::Ok`] means that the global state was incorrect (`mm2_run` didn't work, although it should have),
    /// and there is no need to stop an mm2 instance manually.
    ReadyStopStatus(StopStatus),
}

#[derive(Debug, PartialEq, Primitive)]
pub enum StopStatus {
    Ok = 0,
    NotRunning = 1,
    ErrorStopping = 2,
    StoppingAlready = 3,
}

/// Checks if we can stop a MarketMaker2 instance.
fn prepare_for_mm2_stop() -> PrepareForStopResult {
    // The log callback might be initialized already, so try to use the common logs.
    use common::log::warn;

    if !LP_MAIN_RUNNING.load(Ordering::Relaxed) {
        return PrepareForStopResult::ReadyStopStatus(StopStatus::NotRunning);
    }

    let ctx = CTX.load(Ordering::Relaxed);
    if ctx == 0 {
        warn!("mm2_stop] lp_main is running without ctx");
        LP_MAIN_RUNNING.store(false, Ordering::Relaxed);
        return PrepareForStopResult::ReadyStopStatus(StopStatus::Ok);
    }

    let ctx = match MmArc::from_ffi_handle(ctx) {
        Ok(ctx) => ctx,
        Err(_) => {
            warn!("mm2_stop] lp_main is still running, although ctx has already been dropped");
            LP_MAIN_RUNNING.store(false, Ordering::Relaxed);
            // There is no need to rewrite the `CTX`, because it will be removed on `mm2_main`.
            return PrepareForStopResult::ReadyStopStatus(StopStatus::Ok);
        },
    };

    if ctx.is_stopping() {
        return PrepareForStopResult::ReadyStopStatus(StopStatus::StoppingAlready);
    }

    PrepareForStopResult::CanBeStopped(ctx)
}

async fn finalize_mm2_stop(ctx: MmArc) {
    dispatch_lp_event(ctx.clone(), StopCtxEvent.into()).await;
    let _ = ctx.stop();
}

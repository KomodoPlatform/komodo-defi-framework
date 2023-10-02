use mm2_core::mm_ctx::MmArc;
use mm2_libp2p::behaviours::atomicdex::AdexCmdTx;
#[cfg(test)] use mocktopus::macros::*;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct P2PContext {
    /// Using Mutex helps to prevent cloning which can actually result to channel being unbounded in case of using 1 tx clone per 1 message.
    pub cmd_tx: Mutex<AdexCmdTx>,
}

// `mockable` violates these
#[allow(
    clippy::forget_ref,
    clippy::forget_copy,
    clippy::swap_ptr_to_ref,
    clippy::forget_non_drop,
    clippy::let_unit_value
)]
#[cfg_attr(test, mockable)]
impl P2PContext {
    pub fn new(cmd_tx: AdexCmdTx) -> Self {
        P2PContext {
            cmd_tx: Mutex::new(cmd_tx),
        }
    }

    pub fn store_to_mm_arc(self, ctx: &MmArc) { *ctx.p2p_ctx.lock().unwrap() = Some(Arc::new(self)) }

    pub fn fetch_from_mm_arc(ctx: &MmArc) -> Arc<Self> {
        ctx.p2p_ctx
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone()
            .downcast()
            .unwrap()
    }
}

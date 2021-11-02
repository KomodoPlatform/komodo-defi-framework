use crate::mm2::lp_swap::MakerSwapStatusChanged;
use common::event_dispatcher::Dispatcher;
use common::mm_ctx::{from_ctx, MmArc};
use futures::lock::Mutex as AsyncMutex;
use std::sync::Arc;

#[derive(Clone)]
pub enum LpEvents {
    MakerSwapStatusChanged(MakerSwapStatusChanged),
}

#[derive(Default)]
pub struct DispatcherContext {
    pub dispatcher: AsyncMutex<Dispatcher<LpEvents>>,
}

impl DispatcherContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<DispatcherContext>, String> {
        Ok(try_s!(from_ctx(&ctx.dispatcher_ctx, move || {
            Ok(DispatcherContext::default())
        })))
    }
}

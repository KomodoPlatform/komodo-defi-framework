use common::event_dispatcher::{Dispatcher, EventUniqueIdTraits};
use common::mm_ctx::{from_ctx, MmArc};
use common::mm_number::BigDecimal;
use futures::lock::Mutex as AsyncMutex;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub enum LpEvents {
    MakerSwapStatusChanged {
        uuid: Uuid,
        taker_coin: String,
        maker_coin: String,
        taker_amount: BigDecimal,
        maker_amount: BigDecimal,
        event_status: String,
    },
    Default,
}

impl Default for LpEvents {
    // Just to satisfy the dispatcher
    fn default() -> Self { LpEvents::Default }
}

impl EventUniqueIdTraits for LpEvents {
    fn get_event_unique_id(&self) -> usize {
        match self {
            LpEvents::MakerSwapStatusChanged { .. } => 0,
            LpEvents::Default => 1,
        }
    }
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

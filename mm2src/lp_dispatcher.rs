use crate::mm2::lp_ordermatch::TradingBotEvent;
use crate::mm2::lp_swap::MakerSwapStatusChanged;
use common::event_dispatcher::{Dispatcher, EventUniqueId};
use common::mm_ctx::{from_ctx, MmArc};
use futures::lock::Mutex as AsyncMutex;
use std::any::TypeId;
use std::sync::Arc;

#[derive(Clone)]
pub enum LpEvents {
    MakerSwapStatusChanged(MakerSwapStatusChanged),
    TradingBotEvent(TradingBotEvent),
}

impl From<TradingBotEvent> for LpEvents {
    fn from(evt: TradingBotEvent) -> Self { LpEvents::TradingBotEvent(evt) }
}

impl EventUniqueId for LpEvents {
    fn event_id(&self) -> TypeId {
        match self {
            LpEvents::MakerSwapStatusChanged(_) => MakerSwapStatusChanged::event_id(),
            LpEvents::TradingBotEvent(event) => event.event_id(),
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

use crate::init_utxo::{UtxoInitTaskManager, UtxoInitTaskManagerShared};
use common::mm_ctx::{from_ctx, MmArc};
use std::sync::Arc;

pub struct CoinsActivationContext {
    pub(crate) init_utxo_task_manager: UtxoInitTaskManagerShared,
}

impl CoinsActivationContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<CoinsActivationContext>, String> {
        from_ctx(&ctx.coins_activation_ctx, move || {
            Ok(CoinsActivationContext {
                init_utxo_task_manager: UtxoInitTaskManager::new_shared(),
            })
        })
    }
}

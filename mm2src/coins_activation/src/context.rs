use crate::utxo_activation::{QtumTaskManagerShared, UtxoStandardTaskManagerShared};
use common::mm_ctx::{from_ctx, MmArc};
use rpc_task::RpcTaskManager;
use std::sync::Arc;

pub struct CoinsActivationContext {
    pub(crate) init_utxo_standard_task_manager: UtxoStandardTaskManagerShared,
    pub(crate) init_qtum_task_manager: QtumTaskManagerShared,
}

impl CoinsActivationContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<CoinsActivationContext>, String> {
        from_ctx(&ctx.coins_activation_ctx, move || {
            Ok(CoinsActivationContext {
                init_utxo_standard_task_manager: RpcTaskManager::new_shared(),
                init_qtum_task_manager: RpcTaskManager::new_shared(),
            })
        })
    }
}

use std::sync::Arc;

use mm2_core::mm_ctx::{from_ctx, MmArc};
use rpc_task::RpcTaskManager;

#[cfg(not(target_arch = "wasm32"))]
use crate::lightning_activation::LightningTaskManagerShared;
use crate::utxo_activation::{QtumTaskManagerShared, UtxoStandardTaskManagerShared};
#[cfg(not(target_arch = "wasm32"))]
use crate::z_coin_activation::ZcoinTaskManagerShared;

pub struct CoinsActivationContext {
    pub(crate) init_utxo_standard_task_manager: UtxoStandardTaskManagerShared,
    pub(crate) init_qtum_task_manager: QtumTaskManagerShared,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) init_z_coin_task_manager: ZcoinTaskManagerShared,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) init_lightning_task_manager: LightningTaskManagerShared,
}

impl CoinsActivationContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<CoinsActivationContext>, String> {
        from_ctx(&ctx.coins_activation_ctx, move || {
            Ok(CoinsActivationContext {
                init_utxo_standard_task_manager: RpcTaskManager::new_shared(),
                init_qtum_task_manager: RpcTaskManager::new_shared(),
                #[cfg(not(target_arch = "wasm32"))]
                init_z_coin_task_manager: RpcTaskManager::new_shared(),
                #[cfg(not(target_arch = "wasm32"))]
                init_lightning_task_manager: RpcTaskManager::new_shared(),
            })
        })
    }
}

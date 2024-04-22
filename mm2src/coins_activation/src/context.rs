use crate::eth_with_token_activation::EthTaskManagerShared;
use crate::init_erc20_token_activation::Erc20TokenTaskManagerShared;
#[cfg(not(target_arch = "wasm32"))]
use crate::lightning_activation::LightningTaskManagerShared;
use crate::utxo_activation::{QtumTaskManagerShared, UtxoStandardTaskManagerShared};
use crate::z_coin_activation::ZcoinTaskManagerShared;
use mm2_core::mm_ctx::{from_ctx, MmArc};
use rpc_task::RpcTaskManager;
use std::sync::Arc;

pub struct CoinsActivationContext {
    pub(crate) init_utxo_standard_task_manager: UtxoStandardTaskManagerShared,
    pub(crate) init_qtum_task_manager: QtumTaskManagerShared,
    pub(crate) init_z_coin_task_manager: ZcoinTaskManagerShared,
    pub(crate) init_eth_task_manager: EthTaskManagerShared,
    pub(crate) init_erc20_token_task_manager: Erc20TokenTaskManagerShared,
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
                init_z_coin_task_manager: RpcTaskManager::new_shared(),
                init_eth_task_manager: RpcTaskManager::new_shared(),
                init_erc20_token_task_manager: RpcTaskManager::new_shared(),
                #[cfg(not(target_arch = "wasm32"))]
                init_lightning_task_manager: RpcTaskManager::new_shared(),
            })
        })
    }
}

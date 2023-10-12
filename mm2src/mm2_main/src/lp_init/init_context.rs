use std::sync::Arc;

use mm2_core::mm_ctx::{from_ctx, MmArc};
use rpc_task::RpcTaskManager;

use crate::mm2::lp_native_dex::init_hw::InitHwTaskManagerShared;
#[cfg(target_arch = "wasm32")]
use crate::mm2::lp_native_dex::init_metamask::InitMetamaskManagerShared;

pub struct MmInitContext {
    pub init_hw_task_manager: InitHwTaskManagerShared,
    #[cfg(target_arch = "wasm32")]
    pub init_metamask_manager: InitMetamaskManagerShared,
}

impl MmInitContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<MmInitContext>, String> {
        from_ctx(&ctx.mm_init_ctx, move || {
            Ok(MmInitContext {
                init_hw_task_manager: RpcTaskManager::new_shared(),
                #[cfg(target_arch = "wasm32")]
                init_metamask_manager: RpcTaskManager::new_shared(),
            })
        })
    }
}

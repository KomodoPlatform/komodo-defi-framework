use crate::mm2::lp_native_dex::init_hw::InitHwTaskManagerShared;
use common::mm_ctx::{from_ctx, MmArc};
use rpc_task::RpcTaskManager;
use std::sync::Arc;

pub struct MmInitContext {
    pub init_hw_task_manager: InitHwTaskManagerShared,
}

impl MmInitContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<MmInitContext>, String> {
        from_ctx(&ctx.mm_init_ctx, move || {
            Ok(MmInitContext {
                init_hw_task_manager: RpcTaskManager::new_shared(),
            })
        })
    }
}

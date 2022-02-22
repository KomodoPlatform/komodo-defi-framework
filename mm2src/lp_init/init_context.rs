use crate::mm2::lp_native_dex::mm_init_task::MmInitTaskManagerShared;
use common::mm_ctx::{from_ctx, MmArc};
use gstuff::Constructible;
use rpc_task::{RpcTaskManager, TaskId};
use std::sync::Arc;

pub struct MmInitContext {
    pub mm_init_task_id: Constructible<TaskId>,
    pub mm_init_task_manager: MmInitTaskManagerShared,
}

impl MmInitContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<MmInitContext>, String> {
        from_ctx(&ctx.mm_init_ctx, move || {
            Ok(MmInitContext {
                mm_init_task_id: Constructible::default(),
                mm_init_task_manager: RpcTaskManager::new_shared(),
            })
        })
    }
}

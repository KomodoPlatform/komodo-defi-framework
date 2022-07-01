use crate::account::storage::{AccountStorageBoxed, AccountStorageBuilder};
use mm2_core::mm_ctx::{from_ctx, MmArc};
use std::sync::Arc;

pub(crate) struct AccountContext {
    pub(crate) storage: AccountStorageBoxed,
}

impl AccountContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<AccountContext>, String> {
        from_ctx(&ctx.account_ctx, move || {
            Ok(AccountContext {
                storage: AccountStorageBuilder::new(ctx).build(),
            })
        })
    }
}

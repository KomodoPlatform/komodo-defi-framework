use std::sync::Arc;

use mm2_core::mm_ctx::{from_ctx, MmArc};

use crate::account::storage::{AccountStorage, AccountStorageBoxed, AccountStorageBuilder, AccountStorageResult};

pub(crate) struct AccountContext {
    storage: AccountStorageBoxed,
}

impl AccountContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub(crate) fn from_ctx(ctx: &MmArc) -> Result<Arc<AccountContext>, String> {
        from_ctx(&ctx.account_ctx, move || {
            Ok(AccountContext {
                storage: AccountStorageBuilder::new(ctx).build().map_err(|e| e.to_string())?,
            })
        })
    }

    /// Initializes the storage and returns a reference to it.
    pub(crate) async fn storage(&self) -> AccountStorageResult<&dyn AccountStorage> {
        self.storage.init().await?;
        Ok(self.storage.as_ref())
    }
}

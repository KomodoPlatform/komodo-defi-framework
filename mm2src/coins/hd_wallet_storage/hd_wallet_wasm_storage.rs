use crate::hd_wallet_storage::{HDAccountInfo, HDWalletId, HDWalletStorageError, HDWalletStorageInternalOps,
                               HDWalletStorageResult};
use crate::{CoinsContext, HDWalletDbLocked};
use async_trait::async_trait;
use common::indexed_db::cursor_prelude::*;
use common::indexed_db::{DbIdentifier, DbInstance, DbTable, DbTransactionError, DbUpgrader, IndexedDb,
                         IndexedDbBuilder, InitDbError, InitDbResult, ItemId, OnUpgradeResult, SharedDb,
                         TableSignature, WeakDb};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use crypto::XPub;

const DB_NAME: &str = "hd_wallet";
const DB_VERSION: u32 = 1;

impl From<DbTransactionError> for HDWalletStorageError {
    fn from(e: DbTransactionError) -> Self {
        let desc = e.to_string();
        match e {
            DbTransactionError::NoSuchTable { .. }
            | DbTransactionError::ErrorCreatingTransaction(_)
            | DbTransactionError::ErrorOpeningTable { .. }
            | DbTransactionError::ErrorSerializingIndex { .. }
            | DbTransactionError::MultipleItemsByUniqueIndex { .. }
            | DbTransactionError::NoSuchIndex { .. }
            | DbTransactionError::InvalidIndex { .. }
            | DbTransactionError::UnexpectedState(_)
            | DbTransactionError::TransactionAborted => HDWalletStorageError::Internal(desc),
            DbTransactionError::ErrorDeserializingItem(_) => HDWalletStorageError::ErrorDeserializing(desc),
            DbTransactionError::ErrorSerializingItem(_) => HDWalletStorageError::ErrorSerializing(desc),
            DbTransactionError::ErrorGettingItems(_) => HDWalletStorageError::ErrorLoading(desc),
            DbTransactionError::ErrorUploadingItem(_) | DbTransactionError::ErrorDeletingItems(_) => {
                HDWalletStorageError::ErrorSaving(desc)
            },
        }
    }
}

impl From<CursorError> for HDWalletStorageError {
    fn from(e: CursorError) -> Self {
        let stringified_error = e.to_string();
        match e {
            // We don't expect that the `String` and `u32` types serialization to fail.
            CursorError::ErrorSerializingIndexFieldValue {..}
            // We don't expect that the `String` and `u32` types deserialization to fail.
            | CursorError::ErrorDeserializingIndexValue{..}
            | CursorError::ErrorOpeningCursor {..}
            | CursorError::AdvanceError {..}
            | CursorError::InvalidKeyRange {..}
            | CursorError::TypeMismatch{..}
            | CursorError::IncorrectNumberOfKeysPerIndex {..}
            | CursorError::UnexpectedState(..)
            | CursorError::IncorrectUsage{..} => HDWalletStorageError::Internal(stringified_error),
            CursorError::ErrorDeserializingItem{..} => HDWalletStorageError::ErrorDeserializing(stringified_error),
        }
    }
}

impl From<InitDbError> for HDWalletStorageError {
    fn from(e: InitDbError) -> Self { HDWalletStorageError::Internal(e.to_string()) }
}

#[derive(Deserialize, Serialize)]
pub struct HDAccountStorageItem {
    /// The HD wallet identifier. Multiple `HDAccountStorageItem` items can be matched to this id.
    pub wallet_id: HDWalletId,
    pub account_id: u32,
    pub account_derivation_path: String,
    pub account_xpub: XPub,
    /// The number of addresses that we know have been used by the user.
    pub external_addresses_number: u32,
    pub internal_addresses_number: u32,
}

impl TableSignature for HDAccountStorageItem {
    fn table_name() -> &'static str { "hd_account" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        match (old_version, new_version) {
            (0, 1) => {
                let table = upgrader.create_table(Self::table_name())?;
                table.create_index("wallet_id", false)?;
                table.create_index("account_id", false)?;
                table.create_multi_index("wallet_account_id", &["wallet_id", "account_id"], true)?;
            },
            _ => (),
        }
        Ok(())
    }
}

impl HDAccountStorageItem {
    fn new(wallet_id: HDWalletId, account_info: HDAccountInfo) -> HDAccountStorageItem {
        HDAccountStorageItem {
            wallet_id,
            account_id: account_info.account_id,
            account_derivation_path: account_info.account_derivation_path,
            account_xpub: account_info.account_xpub,
            external_addresses_number: account_info.external_addresses_number,
            internal_addresses_number: account_info.internal_addresses_number,
        }
    }
}

impl From<HDAccountStorageItem> for HDAccountInfo {
    fn from(account: HDAccountStorageItem) -> Self {
        HDAccountInfo {
            account_id: account.account_id,
            account_derivation_path: account.account_derivation_path,
            account_xpub: account.account_xpub,
            external_addresses_number: account.external_addresses_number,
            internal_addresses_number: account.internal_addresses_number,
        }
    }
}

pub struct HDWalletDb {
    inner: IndexedDb,
}

#[async_trait]
impl DbInstance for HDWalletDb {
    fn db_name() -> &'static str { DB_NAME }

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<HDAccountStorageItem>()
            .build()
            .await?;
        Ok(HDWalletDb { inner })
    }
}

/// The wrapper over the [`CoinsContext::hd_wallet_db`] weak pointer.
pub struct HDWalletIndexedDbStorage {
    db: WeakDb<HDWalletDb>,
}

#[async_trait]
impl HDWalletStorageInternalOps for HDWalletIndexedDbStorage {
    fn new(ctx: &MmArc) -> HDWalletStorageResult<Self>
    where
        Self: Sized,
    {
        let coins_ctx = CoinsContext::from_ctx(ctx).map_to_mm(HDWalletStorageError::Internal)?;
        let db = SharedDb::downgrade(&coins_ctx.hd_wallet_db);
        Ok(HDWalletIndexedDbStorage { db })
    }

    async fn load_accounts(&self, wallet_id: HDWalletId) -> HDWalletStorageResult<Vec<HDAccountInfo>> {
        let shared_db = self.get_shared_db()?;
        let locked_db = Self::lock_db(&shared_db).await?;

        let transaction = locked_db.inner.transaction().await?;
        let table = transaction.table::<HDAccountStorageItem>().await?;

        let accounts = table
            .get_items("wallet_id", wallet_id)
            .await?
            .into_iter()
            .map(|(_item_id, item)| HDAccountInfo::from(item))
            .collect();
        Ok(accounts)
    }

    async fn load_account(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<HDAccountInfo>> {
        let shared_db = self.get_shared_db()?;
        let locked_db = Self::lock_db(&shared_db).await?;

        let transaction = locked_db.inner.transaction().await?;
        let table = transaction.table::<HDAccountStorageItem>().await?;

        let maybe_account = Self::find_account(&table, wallet_id, account_id).await?;
        match maybe_account {
            Some((_account_item_id, account_item)) => Ok(Some(HDAccountInfo::from(account_item))),
            None => Ok(None),
        }
    }

    async fn update_external_addresses_number(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_external_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        self.update_account(wallet_id, account_id, |account| {
            account.external_addresses_number = new_external_addresses_number;
        })
        .await
    }

    async fn update_internal_addresses_number(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        self.update_account(wallet_id, account_id, |account| {
            account.internal_addresses_number = new_internal_addresses_number;
        })
        .await
    }

    async fn update_addresses_numbers(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_external_addresses_number: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        self.update_account(wallet_id, account_id, |account| {
            account.external_addresses_number = new_external_addresses_number;
            account.internal_addresses_number = new_internal_addresses_number;
        })
        .await
    }

    async fn upload_new_account(&self, wallet_id: HDWalletId, account: HDAccountInfo) -> HDWalletStorageResult<()> {
        let shared_db = self.get_shared_db()?;
        let locked_db = Self::lock_db(&shared_db).await?;

        let transaction = locked_db.inner.transaction().await?;
        let table = transaction.table::<HDAccountStorageItem>().await?;

        let new_account = HDAccountStorageItem::new(wallet_id, account);
        table
            .add_item(&new_account)
            .await
            .map(|_| ())
            .mm_err(HDWalletStorageError::from)
    }
}

impl HDWalletIndexedDbStorage {
    fn get_shared_db(&self) -> HDWalletStorageResult<SharedDb<HDWalletDb>> {
        self.db
            .upgrade()
            .or_mm_err(|| HDWalletStorageError::Internal("'HDWalletIndexedDbStorage::db' doesn't exist".to_owned()))
    }

    async fn lock_db(db: &SharedDb<HDWalletDb>) -> HDWalletStorageResult<HDWalletDbLocked<'_>> {
        db.get_or_initialize().await.mm_err(HDWalletStorageError::from)
    }

    async fn find_account<'a>(
        table: &DbTable<'a, HDAccountStorageItem>,
        wallet_id: HDWalletId,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<(ItemId, HDAccountStorageItem)>> {
        // Use the cursor to find an item with the specified `wallet_id` and `account_id`.
        let mut accounts = table
            .open_cursor("wallet_account_id")
            .await?
            .only("wallet_id", wallet_id)?
            .only("account_id", account_id)?
            .collect()
            .await?;

        if accounts.len() > 1 {
            let error = DbTransactionError::MultipleItemsByUniqueIndex {
                index: "wallet_account_id".to_owned(),
                got_items: accounts.len(),
            };
            return MmError::err(HDWalletStorageError::ErrorLoading(error.to_string()));
        }
        Ok(accounts.into_iter().next())
    }

    async fn update_account<F>(&self, wallet_id: HDWalletId, account_id: u32, f: F) -> HDWalletStorageResult<()>
    where
        F: FnOnce(&mut HDAccountStorageItem),
    {
        let shared_db = self.get_shared_db()?;
        let locked_db = Self::lock_db(&shared_db).await?;

        let transaction = locked_db.inner.transaction().await?;
        let table = transaction.table::<HDAccountStorageItem>().await?;

        let (account_item_id, mut account) = Self::find_account(&table, wallet_id.clone(), account_id)
            .await?
            .or_mm_err(|| HDWalletStorageError::HDAccountNotFound { wallet_id, account_id })?;

        // Apply `f` to `account` and upload the changes to the storage.
        f(&mut account);
        table
            .replace_item(account_item_id, &account)
            .await
            .map(|_| ())
            .mm_err(HDWalletStorageError::from)
    }
}

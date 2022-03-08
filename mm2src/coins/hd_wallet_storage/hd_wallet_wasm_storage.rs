use crate::hd_wallet_storage::{HDAccountStorageItem, HDWalletId, HDWalletStorageError, HDWalletStorageInternalOps,
                               HDWalletStorageResult};
use crate::CoinsContext;
use async_trait::async_trait;
use common::indexed_db::cursor_prelude::*;
use common::indexed_db::{DbIdentifier, DbInstance, DbLocked, DbTable, DbTransactionError, DbUpgrader, IndexedDb,
                         IndexedDbBuilder, InitDbError, InitDbResult, ItemId, OnUpgradeResult, SharedDb,
                         TableSignature, WeakDb};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use crypto::XPub;

const DB_NAME: &str = "hd_wallet";
const DB_VERSION: u32 = 1;

pub type HDWalletDbLocked<'a> = DbLocked<'a, HDWalletDb>;

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
pub struct HDAccountTable {
    /// The HD wallet identifier. Multiple `HDAccountTable` items can be matched to this id.
    pub wallet_id: HDWalletId,
    pub account_id: u32,
    pub account_xpub: XPub,
    /// The number of addresses that we know have been used by the user.
    pub external_addresses_number: u32,
    pub internal_addresses_number: u32,
}

impl TableSignature for HDAccountTable {
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

impl HDAccountTable {
    fn new(wallet_id: HDWalletId, account_info: HDAccountStorageItem) -> HDAccountTable {
        HDAccountTable {
            wallet_id,
            account_id: account_info.account_id,
            account_xpub: account_info.account_xpub,
            external_addresses_number: account_info.external_addresses_number,
            internal_addresses_number: account_info.internal_addresses_number,
        }
    }
}

impl From<HDAccountTable> for HDAccountStorageItem {
    fn from(account: HDAccountTable) -> Self {
        HDAccountStorageItem {
            account_id: account.account_id,
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
            .with_table::<HDAccountTable>()
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

    async fn load_accounts(&self, wallet_id: HDWalletId) -> HDWalletStorageResult<Vec<HDAccountStorageItem>> {
        let shared_db = self.get_shared_db()?;
        let locked_db = Self::lock_db(&shared_db).await?;

        let transaction = locked_db.inner.transaction().await?;
        let table = transaction.table::<HDAccountTable>().await?;

        let accounts = table
            .get_items("wallet_id", wallet_id)
            .await?
            .into_iter()
            .map(|(_item_id, item)| HDAccountStorageItem::from(item))
            .collect();
        Ok(accounts)
    }

    async fn load_account(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<HDAccountStorageItem>> {
        let shared_db = self.get_shared_db()?;
        let locked_db = Self::lock_db(&shared_db).await?;

        let transaction = locked_db.inner.transaction().await?;
        let table = transaction.table::<HDAccountTable>().await?;

        let maybe_account = Self::find_account(&table, wallet_id, account_id).await?;
        match maybe_account {
            Some((_account_item_id, account_item)) => Ok(Some(HDAccountStorageItem::from(account_item))),
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

    async fn upload_new_account(
        &self,
        wallet_id: HDWalletId,
        account: HDAccountStorageItem,
    ) -> HDWalletStorageResult<()> {
        let shared_db = self.get_shared_db()?;
        let locked_db = Self::lock_db(&shared_db).await?;

        let transaction = locked_db.inner.transaction().await?;
        let table = transaction.table::<HDAccountTable>().await?;

        let new_account = HDAccountTable::new(wallet_id, account);
        table
            .add_item(&new_account)
            .await
            .map(|_| ())
            .mm_err(HDWalletStorageError::from)
    }

    async fn clear_accounts(&self, wallet_id: HDWalletId) -> HDWalletStorageResult<()> {
        let shared_db = self.get_shared_db()?;
        let locked_db = Self::lock_db(&shared_db).await?;

        let transaction = locked_db.inner.transaction().await?;
        let table = transaction.table::<HDAccountTable>().await?;

        table
            .delete_items_by_index("wallet_id", wallet_id)
            .await
            .map(|_ids| ())
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
        table: &DbTable<'a, HDAccountTable>,
        wallet_id: HDWalletId,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<(ItemId, HDAccountTable)>> {
        // Use the cursor to find an item with the specified `wallet_id` and `account_id`.
        let accounts = table
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
        F: FnOnce(&mut HDAccountTable),
    {
        let shared_db = self.get_shared_db()?;
        let locked_db = Self::lock_db(&shared_db).await?;

        let transaction = locked_db.inner.transaction().await?;
        let table = transaction.table::<HDAccountTable>().await?;

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

mod tests {
    use super::*;
    use crate::hd_wallet_storage::{HDAccountStorageItem, HDWalletCoinStorage};
    use crate::CoinsContext;
    use common::mm_ctx::MmCtxBuilder;
    use itertools::Itertools;
    use primitives::hash::H160;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    async fn get_all_db_items<Table: TableSignature>(ctx: &MmArc) -> Vec<Table> {
        let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
        let db = coins_ctx.hd_wallet_db.get_or_initialize().await.unwrap();
        let transaction = db.inner.transaction().await.unwrap();
        let table = transaction.table::<Table>().await.unwrap();
        table
            .get_all_items()
            .await
            .expect("Error getting items")
            .into_iter()
            .map(|(_item_id, item)| item)
            .collect()
    }

    #[wasm_bindgen_test]
    async fn test_unique_wallets() {
        let rick_user0_device0_account0 = HDAccountStorageItem {
            account_id: 0,
            account_xpub: "xpub6DEHSksajpRPM59RPw7Eg6PKdU7E2ehxJWtYdrfQ6JFmMGBsrR6jA78ANCLgzKYm4s5UqQ4ydLEYPbh3TRVvn5oAZVtWfi4qJLMntpZ8uGJ".to_owned(),
            external_addresses_number: 1,
            internal_addresses_number: 2,
        };
        let rick_user0_device0_account1 = HDAccountStorageItem {
            account_id: 1,
            account_xpub: "xpub6DEHSksajpRPQq2FdGT6JoieiQZUpTZ3WZn8fcuLJhFVmtCpXbuXxp5aPzaokwcLV2V9LE55Dwt8JYkpuMv7jXKwmyD28WbHYjBH2zhbW2p".to_owned(),
            external_addresses_number: 1,
            internal_addresses_number: 2,
        };
        let rick_user0_device1_account0 = HDAccountStorageItem {
            account_id: 0,
            account_xpub: "xpub6EuV33a2DXxAhoJTRTnr8qnysu81AA4YHpLY6o8NiGkEJ8KADJ35T64eJsStWsmRf1xXkEANVjXFXnaUKbRtFwuSPCLfDdZwYNZToh4LBCd".to_owned(),
            external_addresses_number: 3,
            internal_addresses_number: 4,
        };
        let rick_user1_device0_account0 = HDAccountStorageItem {
            account_id: 0,
            account_xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz".to_owned(),
            external_addresses_number: 5,
            internal_addresses_number: 6,
        };
        let morty_user0_device0_account0 = HDAccountStorageItem {
            account_id: 0,
            account_xpub: "xpub6AHA9hZDN11k2ijHMeS5QqHx2KP9aMBRhTDqANMnwVtdyw2TDYRmF8PjpvwUFcL1Et8Hj59S3gTSMcUQ5gAqTz3Wd8EsMTmF3DChhqPQBnU".to_owned(),
            external_addresses_number: 7,
            internal_addresses_number: 8,
        };

        let ctx = MmCtxBuilder::new().with_test_db_namespace().into_mm_arc();
        let user0_rmd160 = H160::from("0000000000000000000000000000000000000000");
        let user1_rmd160 = H160::from("0000000000000000000000000000000000000001");
        let device0_rmd160 = H160::from("0000000000000000000000000000000000000020");
        let device1_rmd160 = H160::from("0000000000000000000000000000000000000030");

        let rick_user0_device0_db =
            HDWalletCoinStorage::with_rmd160(&ctx, "RICK".to_owned(), user0_rmd160, device0_rmd160)
                .expect("!HDWalletCoinStorage::new");
        let rick_user0_device1_db =
            HDWalletCoinStorage::with_rmd160(&ctx, "RICK".to_owned(), user0_rmd160, device1_rmd160)
                .expect("!HDWalletCoinStorage::new");
        let rick_user1_device0_db =
            HDWalletCoinStorage::with_rmd160(&ctx, "RICK".to_owned(), user1_rmd160, device0_rmd160)
                .expect("!HDWalletCoinStorage::new");
        let morty_user0_device0_db =
            HDWalletCoinStorage::with_rmd160(&ctx, "MORTY".to_owned(), user0_rmd160, device0_rmd160)
                .expect("!HDWalletCoinStorage::new");

        rick_user0_device0_db
            .upload_new_account(rick_user0_device0_account0.clone())
            .await
            .expect("!HDWalletCoinStorage::upload_new_account: RICK user=0 device=0 account=0");
        rick_user0_device0_db
            .upload_new_account(rick_user0_device0_account1.clone())
            .await
            .expect("!HDWalletCoinStorage::upload_new_account: RICK user=0 device=0 account=1");
        rick_user0_device1_db
            .upload_new_account(rick_user0_device1_account0.clone())
            .await
            .expect("!HDWalletCoinStorage::upload_new_account: RICK user=0 device=1 account=0");
        rick_user1_device0_db
            .upload_new_account(rick_user1_device0_account0.clone())
            .await
            .expect("!HDWalletCoinStorage::upload_new_account: RICK user=1 device=0 account=0");
        morty_user0_device0_db
            .upload_new_account(morty_user0_device0_account0.clone())
            .await
            .expect("!HDWalletCoinStorage::upload_new_account: MORTY user=0 device=0 account=0");

        // All accounts must be in the only one database.
        // Rows in the database must differ by only `wallet_id` and `account_id` values.
        let all_accounts: Vec<_> = get_all_db_items::<HDAccountTable>(&ctx)
            .await
            .into_iter()
            .map(HDAccountStorageItem::from)
            .sorted_by(|x, y| x.external_addresses_number.cmp(&y.external_addresses_number))
            .collect();
        assert_eq!(all_accounts, vec![
            rick_user0_device0_account0.clone(),
            rick_user0_device0_account1.clone(),
            rick_user0_device1_account0.clone(),
            rick_user1_device0_account0.clone(),
            morty_user0_device0_account0.clone()
        ]);

        let mut actual = rick_user0_device0_db
            .load_all_accounts()
            .await
            .expect("HDWalletCoinStorage::load_all_accounts: RICK user=0 device=0");
        actual.sort_by(|x, y| x.account_id.cmp(&y.account_id));
        assert_eq!(actual, vec![rick_user0_device0_account0, rick_user0_device0_account1]);

        let actual = rick_user0_device1_db
            .load_all_accounts()
            .await
            .expect("HDWalletCoinStorage::load_all_accounts: RICK user=0 device=1");
        assert_eq!(actual, vec![rick_user0_device1_account0]);

        let actual = rick_user1_device0_db
            .load_all_accounts()
            .await
            .expect("HDWalletCoinStorage::load_all_accounts: RICK user=1 device=0");
        assert_eq!(actual, vec![rick_user1_device0_account0]);

        let actual = morty_user0_device0_db
            .load_all_accounts()
            .await
            .expect("HDWalletCoinStorage::load_all_accounts: MORTY user=0 device=0");
        assert_eq!(actual, vec![morty_user0_device0_account0]);
    }
}

use crate::account::storage::{AccountStorage, AccountStorageError, AccountStorageResult};
use crate::account::{AccountId, AccountInfo, AccountType, AccountWithCoins, AccountWithEnabledFlag, EnabledAccountId,
                     EnabledAccountType, HwPubkey, MAX_ACCOUNT_DESCRIPTION_LENGTH, MAX_ACCOUNT_NAME_LENGTH,
                     MAX_TICKER_LENGTH};
use async_trait::async_trait;
use db_common::sql_constraint::UniqueConstraint;
use db_common::sql_create::{SqlColumn, SqlCreateTable, SqlType};
use db_common::sql_delete::SqlDelete;
use db_common::sql_insert::SqlInsert;
use db_common::sql_query::SqlQuery;
use db_common::sqlite::rusqlite::types::Type;
use db_common::sqlite::rusqlite::{Connection, Error as SqlError, Row};
use db_common::sqlite::SqliteConnShared;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::{Arc, MutexGuard};

const DEVICE_PUBKEY_MAX_LENGTH: usize = 20;
const BALANCE_MAX_LENGTH: usize = 255;

mod account_table {
    /// The table name.
    pub(super) const TABLE_NAME: &str = "gui_account";

    // The following constants are the column names.
    pub(super) const ACCOUNT_TYPE: &str = "account_type";
    pub(super) const ACCOUNT_IDX: &str = "account_idx";
    pub(super) const DEVICE_PUBKEY: &str = "device_pubkey";
    pub(super) const NAME: &str = "name";
    pub(super) const DESCRIPTION: &str = "description";
    pub(super) const BALANCE_USD: &str = "balance_usd";

    /// The table constraint.
    pub(super) const ACCOUNT_ID_CONSTRAINT: &str = "account_id_constraint";
}

mod account_coins_table {
    /// The table name.
    pub(super) const TABLE_NAME: &str = "gui_account_coins";

    // The following constants are the column names.
    pub(super) const ACCOUNT_TYPE: &str = "account_type";
    pub(super) const ACCOUNT_IDX: &str = "account_idx";
    pub(super) const DEVICE_PUBKEY: &str = "device_pubkey";
    pub(super) const COIN: &str = "coin";

    /// The table constraint.
    pub(super) const ACCOUNT_ID_COIN_CONSTRAINT: &str = "account_id_coin_constraint";
}

mod enabled_account_table {
    /// The table name.
    pub(super) const TABLE_NAME: &str = "gui_account_enabled";

    // The following constants are the column names.
    pub(super) const ACCOUNT_TYPE: &str = "account_type";
    pub(super) const ACCOUNT_IDX: &str = "account_idx";
}

impl From<SqlError> for AccountStorageError {
    fn from(e: SqlError) -> Self {
        let error = e.to_string();
        match e {
            SqlError::FromSqlConversionFailure(_, _, _)
            | SqlError::IntegralValueOutOfRange(_, _)
            | SqlError::InvalidColumnIndex(_)
            | SqlError::InvalidColumnType(_, _, _) => AccountStorageError::ErrorDeserializing(error),
            SqlError::Utf8Error(_) | SqlError::NulError(_) | SqlError::ToSqlConversionFailure(_) => {
                AccountStorageError::ErrorSerializing(error)
            },
            _ => AccountStorageError::Internal(error),
        }
    }
}

impl AccountId {
    /// An alternative to [`AccountId::to_tuple`] that returns SQL compatible types.
    fn to_sql_tuple(&self) -> (i64, Option<i64>, Option<String>) {
        let (account_type, account_idx, device_pubkey) = self.to_tuple();
        (
            account_type as i64,
            account_idx.map(|idx| idx as i64),
            device_pubkey.map(|pubkey| pubkey.to_string()),
        )
    }

    /// An alternative to [`AccountId::try_from_tuple`] that takes SQL compatible types.
    pub(crate) fn try_from_sql_tuple(
        account_type: i64,
        account_idx: Option<u32>,
        device_pubkey: Option<String>,
    ) -> AccountStorageResult<AccountId> {
        let account_type = AccountType::try_from(account_type)?;
        let device_pubkey = device_pubkey
            // Map `Option<String>` into `Option<Result<HwPubkey, _>>`
            .map(|pubkey| HwPubkey::from_str(&pubkey))
            // Transpose `Option<Result<HwPubkey, _>>` into `Result<Option<HwPubkey, _>>`
            .transpose()
            .map_to_mm(|e| AccountStorageError::ErrorDeserializing(e.to_string()))?;
        AccountId::try_from_tuple(account_type, account_idx, device_pubkey)
    }
}

impl EnabledAccountId {
    /// An alternative to [`EnabledAccountId::to_pair`] that returns SQL compatible types.
    fn to_sql_pair(&self) -> (i64, Option<i64>) {
        let (account_type, account_idx) = self.to_pair();
        (account_type as i64, account_idx.map(|idx| idx as i64))
    }

    /// An alternative to [`EnabledAccountId::try_from_pair`] that takes SQL compatible types.
    pub(crate) fn try_from_sql_pair(
        account_type: i64,
        account_idx: Option<u32>,
    ) -> AccountStorageResult<EnabledAccountId> {
        let account_type = EnabledAccountType::try_from(account_type)?;
        EnabledAccountId::try_from_pair(account_type, account_idx)
    }
}

pub(crate) struct SqliteAccountStorage {
    conn: SqliteConnShared,
}

impl SqliteAccountStorage {
    pub(crate) fn new(ctx: &MmArc) -> AccountStorageResult<SqliteAccountStorage> {
        let shared = ctx
            .sqlite_connection
            .as_option()
            .or_mm_err(|| AccountStorageError::Internal("'MmCtx::sqlite_connection' is not initialized".to_owned()))?;
        Ok(SqliteAccountStorage {
            conn: Arc::clone(shared),
        })
    }

    fn lock_conn(&self) -> AccountStorageResult<MutexGuard<Connection>> {
        self.conn
            .lock()
            .map_to_mm(|e| AccountStorageError::Internal(format!("Error locking sqlite connection: {}", e)))
    }

    fn init_account_table(conn: &Connection) -> AccountStorageResult<()> {
        let mut create_sql = SqlCreateTable::new(conn, account_table::TABLE_NAME);
        create_sql
            .if_not_exist()
            .column(SqlColumn::new(account_table::ACCOUNT_TYPE, SqlType::Integer).not_null())
            .column(SqlColumn::new(account_table::ACCOUNT_IDX, SqlType::Integer))
            .column(SqlColumn::new(
                account_table::DEVICE_PUBKEY,
                SqlType::Varchar(DEVICE_PUBKEY_MAX_LENGTH),
            ))
            .column(SqlColumn::new(account_table::NAME, SqlType::Varchar(MAX_ACCOUNT_NAME_LENGTH)).not_null())
            .column(SqlColumn::new(
                account_table::DESCRIPTION,
                SqlType::Varchar(MAX_ACCOUNT_DESCRIPTION_LENGTH),
            ))
            .column(SqlColumn::new(account_table::BALANCE_USD, SqlType::Varchar(BALANCE_MAX_LENGTH)).not_null())
            .constraint(
                UniqueConstraint::new([
                    account_table::ACCOUNT_TYPE,
                    account_table::ACCOUNT_IDX,
                    account_table::DEVICE_PUBKEY,
                ])?
                .name(account_table::ACCOUNT_ID_CONSTRAINT),
            );
        create_sql.create().map_to_mm(AccountStorageError::from)
    }

    fn init_account_coins_table(conn: &Connection) -> AccountStorageResult<()> {
        let mut create_sql = SqlCreateTable::new(conn, account_coins_table::TABLE_NAME);
        create_sql
            .if_not_exist()
            .column(SqlColumn::new(account_coins_table::ACCOUNT_TYPE, SqlType::Integer).not_null())
            .column(SqlColumn::new(account_coins_table::ACCOUNT_IDX, SqlType::Integer))
            .column(SqlColumn::new(
                account_coins_table::DEVICE_PUBKEY,
                SqlType::Varchar(DEVICE_PUBKEY_MAX_LENGTH),
            ))
            .column(SqlColumn::new(account_coins_table::COIN, SqlType::Varchar(MAX_TICKER_LENGTH)).not_null())
            .constraint(
                UniqueConstraint::new([
                    account_coins_table::ACCOUNT_TYPE,
                    account_coins_table::ACCOUNT_IDX,
                    account_coins_table::DEVICE_PUBKEY,
                    account_coins_table::COIN,
                ])?
                .name(account_coins_table::ACCOUNT_ID_COIN_CONSTRAINT),
            );
        create_sql.create().map_to_mm(AccountStorageError::from)
    }

    fn init_enabled_account_table(conn: &Connection) -> AccountStorageResult<()> {
        let mut create_sql = SqlCreateTable::new(conn, enabled_account_table::TABLE_NAME);
        create_sql
            .if_not_exist()
            .column(SqlColumn::new(enabled_account_table::ACCOUNT_TYPE, SqlType::Integer).not_null())
            .column(SqlColumn::new(enabled_account_table::ACCOUNT_IDX, SqlType::Integer));
        create_sql.create().map_to_mm(AccountStorageError::from)
    }

    fn load_enabled_account_id(conn: &Connection) -> AccountStorageResult<EnabledAccountId> {
        let mut query = SqlQuery::select_from(conn, enabled_account_table::TABLE_NAME)?;
        query
            .field(enabled_account_table::ACCOUNT_TYPE)?
            .field(enabled_account_table::ACCOUNT_IDX)?;
        query
            .query_single_row(enabled_account_id_from_row)?
            .or_mm_err(|| AccountStorageError::NoEnabledAccount)
    }

    /// Loads `AccountWithCoins`.
    /// This method takes `conn` to ensure data coherence.
    fn load_account_with_coins(
        conn: &Connection,
        account_id: &AccountId,
    ) -> AccountStorageResult<Option<AccountWithCoins>> {
        let account_info = match Self::load_account(conn, account_id)? {
            Some(acc) => acc,
            None => return Ok(None),
        };
        let mut query = SqlQuery::select_from(conn, account_coins_table::TABLE_NAME)?;

        let (account_type, account_id, device_pubkey) = account_id.to_sql_tuple();
        query
            .field(account_coins_table::COIN)?
            .and_where_eq(account_table::ACCOUNT_TYPE, account_type)?
            .and_where_eq(account_table::ACCOUNT_IDX, account_id)?
            .and_where_eq_param(account_table::DEVICE_PUBKEY, device_pubkey)?;
        let coins = query.query(|row| row.get::<_, String>(0))?.into_iter().collect();
        Ok(Some(AccountWithCoins { account_info, coins }))
    }

    /// Tries to load an account info.
    /// This method takes `conn` to ensure data coherence.
    fn load_account(conn: &Connection, account_id: &AccountId) -> AccountStorageResult<Option<AccountInfo>> {
        let mut query = SqlQuery::select_from(conn, account_table::TABLE_NAME)?;
        query
            .field(account_table::ACCOUNT_TYPE)?
            .field(account_table::ACCOUNT_IDX)?
            .field(account_table::NAME)?
            .field(account_table::DESCRIPTION)?
            .field(account_table::BALANCE_USD)?;

        let (account_type, account_id, device_pubkey) = account_id.to_sql_tuple();
        query
            .and_where_eq(account_table::ACCOUNT_TYPE, account_type)?
            .and_where_eq(account_table::ACCOUNT_IDX, account_id)?
            .and_where_eq_param(account_table::DEVICE_PUBKEY, device_pubkey)?;
        query
            .query_single_row(account_from_row)
            .map_to_mm(AccountStorageError::from)
    }

    fn load_accounts(conn: &Connection) -> AccountStorageResult<BTreeMap<AccountId, AccountInfo>> {
        let mut query = SqlQuery::select_from(conn, account_table::TABLE_NAME)?;
        query
            .field(account_table::ACCOUNT_TYPE)?
            .field(account_table::ACCOUNT_IDX)?
            .field(account_table::DEVICE_PUBKEY)?
            .field(account_table::NAME)?
            .field(account_table::DESCRIPTION)?
            .field(account_table::BALANCE_USD)?;
        let accounts = query
            .query(account_from_row)?
            .into_iter()
            .map(|account| (account.account_id.clone(), account))
            .collect();
        Ok(accounts)
    }

    fn account_exists(conn: &Connection, account_id: &AccountId) -> AccountStorageResult<bool> {
        let mut query = SqlQuery::select_from(conn, account_table::TABLE_NAME)?;
        query.count(account_table::NAME)?;

        let (account_type, account_idx, device_pubkey) = account_id.to_sql_tuple();
        query
            .and_where_eq(account_table::ACCOUNT_TYPE, account_type)?
            .and_where_eq(account_table::ACCOUNT_IDX, account_idx)?
            .and_where_eq_param(account_table::DEVICE_PUBKEY, device_pubkey)?;

        let accounts = query
            .query_single_row(count_from_row)?
            .or_mm_err(|| AccountStorageError::Internal("'count' should have returned one row exactly".to_string()))?;

        Ok(accounts > 0)
    }

    fn upload_account(conn: &Connection, account: AccountInfo) -> AccountStorageResult<()> {
        let mut sql_insert = SqlInsert::new(&conn, account_table::TABLE_NAME);

        let (account_type, account_idx, device_pubkey) = account.account_id.to_sql_tuple();
        sql_insert
            .column(account_table::ACCOUNT_TYPE, account_type)?
            .column(account_table::ACCOUNT_IDX, account_idx)?
            .column_param(account_table::DEVICE_PUBKEY, device_pubkey)?
            .column_param(account_table::NAME, account.name)?
            .column_param(account_table::DESCRIPTION, account.description)?
            .column_param(account_table::BALANCE_USD, account.balance_usd.to_string())?;
        sql_insert.insert()?;
        Ok(())
    }
}

#[async_trait]
impl AccountStorage for SqliteAccountStorage {
    async fn init(&self) -> AccountStorageResult<()> {
        let mut conn = self.lock_conn()?;
        let transaction = conn.transaction()?;

        SqliteAccountStorage::init_account_table(&transaction)?;
        SqliteAccountStorage::init_account_coins_table(&transaction)?;
        SqliteAccountStorage::init_enabled_account_table(&transaction)?;

        transaction.commit()?;
        Ok(())
    }

    async fn load_accounts_with_enabled_flag(
        &self,
    ) -> AccountStorageResult<BTreeMap<AccountId, AccountWithEnabledFlag>> {
        let conn = self.lock_conn()?;
        let enabled_account_id = AccountId::from(Self::load_enabled_account_id(&conn)?);

        let mut found_enabled = false;
        let accounts = Self::load_accounts(&conn)?
            .into_iter()
            .map(|(account_id, account_info)| {
                let enabled = account_id == enabled_account_id;
                found_enabled |= enabled;
                Ok((account_id, AccountWithEnabledFlag { account_info, enabled }))
            })
            .collect::<AccountStorageResult<BTreeMap<_, _>>>()?;

        // If `AccountStorage::load_enabled_account_id` returns an `AccountId`,
        // then corresponding account must be in `AccountTable`.
        if !found_enabled {
            return MmError::err(AccountStorageError::unknown_account_in_enabled_table(
                enabled_account_id,
            ));
        }
        Ok(accounts)
    }

    async fn load_enabled_account_id(&self) -> AccountStorageResult<EnabledAccountId> {
        let conn = self.lock_conn()?;
        Self::load_enabled_account_id(&conn)
    }

    async fn load_enabled_account_with_coins(&self) -> AccountStorageResult<AccountWithCoins> {
        let conn = self.lock_conn()?;
        let account_id = AccountId::from(Self::load_enabled_account_id(&conn)?);

        Self::load_account_with_coins(&conn, &account_id)?
            .or_mm_err(|| AccountStorageError::unknown_account_in_enabled_table(account_id))
    }

    async fn enable_account(&self, enabled_account_id: EnabledAccountId) -> AccountStorageResult<()> {
        let conn = self.lock_conn()?;

        // First, check if the account exists.
        let account_id = AccountId::from(enabled_account_id);
        if !Self::account_exists(&conn, &account_id)? {
            return MmError::err(AccountStorageError::NoSuchAccount(account_id));
        }

        // Remove the previous enabled account by clearing the table.
        SqlDelete::new(&conn, enabled_account_table::TABLE_NAME).delete()?;

        let mut sql_insert = SqlInsert::new(&conn, enabled_account_table::TABLE_NAME);

        let (account_type, account_idx) = enabled_account_id.to_sql_pair();
        sql_insert
            .column(enabled_account_table::ACCOUNT_TYPE, account_type)?
            .column(enabled_account_table::ACCOUNT_IDX, account_idx)?;

        sql_insert.insert()?;
        Ok(())
    }

    async fn upload_account(&self, account: AccountInfo) -> AccountStorageResult<()> {
        let conn = self.lock_conn()?;

        // First, check if the account doesn't exist.
        if Self::account_exists(&conn, &account.account_id)? {
            return MmError::err(AccountStorageError::AccountExistsAlready(account.account_id));
        }

        Self::upload_account(&conn, account)
    }

    async fn set_name(&self, account_id: AccountId, name: String) -> AccountStorageResult<()> { todo!() }

    async fn set_description(&self, account_id: AccountId, description: String) -> AccountStorageResult<()> { todo!() }

    async fn set_balance(&self, account_id: AccountId, balance_usd: BigDecimal) -> AccountStorageResult<()> { todo!() }

    async fn activate_coin(&self, account_id: AccountId, ticker: String) -> AccountStorageResult<()> { todo!() }

    async fn deactivate_coin(&self, account_id: AccountId, ticker: &str) -> AccountStorageResult<()> { todo!() }
}

fn account_id_from_row(row: &Row<'_>) -> Result<AccountId, SqlError> {
    let account_type: i64 = row.get(0)?;
    let account_idx: Option<u32> = row.get(1)?;
    let device_pubkey: Option<String> = row.get(2)?;
    AccountId::try_from_sql_tuple(account_type, account_idx, device_pubkey)
        .map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

fn enabled_account_id_from_row(row: &Row<'_>) -> Result<EnabledAccountId, SqlError> {
    let account_type: i64 = row.get(0)?;
    let account_idx: Option<u32> = row.get(1)?;
    EnabledAccountId::try_from_sql_pair(account_type, account_idx)
        .map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

fn account_from_row(row: &Row<'_>) -> Result<AccountInfo, SqlError> {
    let account_id = account_id_from_row(row)?;
    let name = row.get(3)?;
    let description = row.get(4)?;
    let balance_usd = bigdecimal_from_row(row, 5)?;
    Ok(AccountInfo {
        account_id,
        name,
        description,
        balance_usd,
    })
}

fn count_from_row(row: &Row<'_>) -> Result<i64, SqlError> { row.get(0) }

fn bigdecimal_from_row(row: &Row<'_>, idx: usize) -> Result<BigDecimal, SqlError> {
    let decimal: String = row.get(idx)?;
    BigDecimal::from_str(&decimal).map_err(|e| SqlError::FromSqlConversionFailure(idx, Type::Text, Box::new(e)))
}

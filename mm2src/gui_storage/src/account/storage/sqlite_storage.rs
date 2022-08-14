use crate::account::storage::{AccountStorage, AccountStorageError, AccountStorageResult};
use crate::account::{AccountId, AccountInfo, AccountType, AccountWithCoins, AccountWithEnabledFlag, EnabledAccountId,
                     EnabledAccountType, MAX_ACCOUNT_DESCRIPTION_LENGTH, MAX_ACCOUNT_NAME_LENGTH, MAX_COIN_LENGTH};
use async_trait::async_trait;
use db_common::sql_constraint::UniqueConstraint;
use db_common::sql_create::{SqlColumn, SqlCreateTable, SqlType};
use db_common::sql_query::SqlQuery;
use db_common::sqlite::rusqlite::types::Type;
use db_common::sqlite::rusqlite::{Connection, Error as SqlError, Row};
use db_common::sqlite::{SqliteConnShared, StringError};
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use std::collections::BTreeMap;
use std::sync::MutexGuard;

mod account_table {
    /// The table name.
    pub(super) const TABLE_NAME: &str = "gui_account";

    // The following constants are the column names.
    pub(super) const ACCOUNT_TYPE: &str = "account_type";
    pub(super) const ACCOUNT_IDX: &str = "account_idx";
    pub(super) const NAME: &str = "name";
    pub(super) const DESCRIPTION: &str = "description";
    pub(super) const BALANCE_USD: &str = "balance_usd";

    /// The table constraint.
    pub(super) const ACCOUNT_TYPE_IDX_CONSTRAINT: &str = "account_type_idx_constraint";
}

mod account_coins_table {
    /// The table name.
    pub(super) const TABLE_NAME: &str = "gui_account_coins";

    // The following constants are the column names.
    pub(super) const ACCOUNT_TYPE: &str = "account_type";
    pub(super) const ACCOUNT_IDX: &str = "account_idx";
    pub(super) const COIN: &str = "coin";

    /// The table constraint.
    pub(super) const ACCOUNT_TYPE_IDX_COIN_CONSTRAINT: &str = "account_type_idx_coin_constraint";
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

impl AccountType {
    fn to_column(&self) -> u8 {
        match self {
            AccountType::Iguana => 0,
            AccountType::HD => 1,
            AccountType::HW => 2,
        }
    }

    fn from_column(column_idx: usize, value: u8) -> Result<AccountType, SqlError> {
        match value {
            0 => Ok(AccountType::Iguana),
            1 => Ok(AccountType::HD),
            2 => Ok(AccountType::HW),
            other => {
                let error = StringError::from(format!("Unknown 'account_type' value: {}", other)).into_boxed();
                Err(SqlError::FromSqlConversionFailure(column_idx, Type::Text, error))
            },
        }
    }
}

impl EnabledAccountType {
    fn to_column(&self) -> u8 { AccountType::from(*self).to_column() }

    fn from_column(column_idx: usize, value: u8) -> Result<EnabledAccountType, SqlError> {
        match AccountType::from_column(column_idx, value)? {
            AccountType::Iguana => Ok(EnabledAccountType::Iguana),
            AccountType::HD => Ok(EnabledAccountType::HD),
            AccountType::HW => {
                let error = StringError::from("HW account cannot be enabled").into_boxed();
                Err(SqlError::FromSqlConversionFailure(column_idx, Type::Text, error))
            },
        }
    }
}

pub(crate) struct SqliteAccountStorage {
    conn: SqliteConnShared,
}

impl SqliteAccountStorage {
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
            .column(
                SqlColumn::new(account_table::NAME, SqlType::Varchar(MAX_ACCOUNT_NAME_LENGTH))
                    .not_null()
                    .unique(),
            )
            .column(SqlColumn::new(
                account_table::DESCRIPTION,
                SqlType::Varchar(MAX_ACCOUNT_DESCRIPTION_LENGTH),
            ))
            .column(SqlColumn::new(account_table::BALANCE_USD, SqlType::Real).not_null())
            .constraint(
                UniqueConstraint::new([account_table::ACCOUNT_TYPE, account_table::ACCOUNT_IDX])?
                    .name(account_table::ACCOUNT_TYPE_IDX_CONSTRAINT),
            );
        create_sql.create().map_to_mm(AccountStorageError::from)
    }

    fn init_account_coins_table(conn: &Connection) -> AccountStorageResult<()> {
        let mut create_sql = SqlCreateTable::new(conn, account_coins_table::TABLE_NAME);
        create_sql
            .if_not_exist()
            .column(SqlColumn::new(account_coins_table::ACCOUNT_TYPE, SqlType::Integer).not_null())
            .column(SqlColumn::new(account_coins_table::ACCOUNT_IDX, SqlType::Integer))
            .column(SqlColumn::new(account_coins_table::COIN, SqlType::Varchar(MAX_COIN_LENGTH)).not_null())
            .constraint(
                UniqueConstraint::new([
                    account_coins_table::ACCOUNT_TYPE,
                    account_coins_table::ACCOUNT_IDX,
                    account_coins_table::COIN,
                ])?
                .name(account_coins_table::ACCOUNT_TYPE_IDX_COIN_CONSTRAINT),
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

        let (account_type, account_id) = account_id.to_pair();
        query
            .field(account_coins_table::COIN)?
            .and_where_eq_param(account_table::ACCOUNT_TYPE, account_type.to_column())?
            .and_where_eq(account_table::ACCOUNT_IDX, account_id.map(|id| id as i64))?;
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

        let (account_type, account_id) = account_id.to_pair();
        query
            .and_where_eq_param(account_table::ACCOUNT_TYPE, account_type.to_column())?
            .and_where_eq(account_table::ACCOUNT_IDX, account_id.map(|id| id as i64))?;
        query
            .query_single_row(account_from_row)
            .map_to_mm(AccountStorageError::from)
    }

    fn load_accounts(conn: &Connection) -> AccountStorageResult<BTreeMap<AccountId, AccountInfo>> {
        let mut query = SqlQuery::select_from(conn, account_table::TABLE_NAME)?;
        query
            .field(account_table::ACCOUNT_TYPE)?
            .field(account_table::ACCOUNT_IDX)?
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

    async fn enable_account(&self, account_id: EnabledAccountId) -> AccountStorageResult<()> { todo!() }

    async fn upload_account(&self, account: AccountInfo) -> AccountStorageResult<()> { todo!() }

    async fn set_name(&self, account_id: AccountId, name: String) -> AccountStorageResult<()> { todo!() }

    async fn set_description(&self, account_id: AccountId, description: String) -> AccountStorageResult<()> { todo!() }

    async fn set_balance(&self, account_id: AccountId, balance_usd: BigDecimal) -> AccountStorageResult<()> { todo!() }

    async fn activate_coin(&self, account_id: AccountId, ticker: String) -> AccountStorageResult<()> { todo!() }

    async fn deactivate_coin(&self, account_id: AccountId, ticker: &str) -> AccountStorageResult<()> { todo!() }
}

fn account_id_from_row(row: &Row<'_>) -> Result<AccountId, SqlError> {
    let account_type: u8 = row.get(0)?;
    let account_type = AccountType::from_column(0, account_type)?;
    let account_idx: Option<u32> = row.get(1)?;
    AccountId::try_from_pair(account_type, account_idx)
        .map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

fn enabled_account_id_from_row(row: &Row<'_>) -> Result<EnabledAccountId, SqlError> {
    let account_type: u8 = row.get(0)?;
    let account_type = EnabledAccountType::from_column(0, account_type)?;
    let account_idx: Option<u32> = row.get(1)?;
    EnabledAccountId::try_from_pair(account_type, account_idx)
        .map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

fn account_from_row(row: &Row<'_>) -> Result<AccountInfo, SqlError> {
    let account_id = account_id_from_row(row)?;
    let name = row.get(2)?;
    let description = row.get(3)?;
    let balance_usd = bigdecimal_from_row(row, 4)?;
    Ok(AccountInfo {
        account_id,
        name,
        description,
        balance_usd,
    })
}

fn bigdecimal_from_row(row: &Row<'_>, idx: usize) -> Result<BigDecimal, SqlError> {
    let decimal: f64 = row.get(idx)?;
    BigDecimal::try_from(decimal).map_err(|e| SqlError::FromSqlConversionFailure(idx, Type::Real, Box::new(e)))
}

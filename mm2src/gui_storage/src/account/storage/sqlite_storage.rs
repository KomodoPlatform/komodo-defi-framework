use crate::account::storage::{AccountStorage, AccountStorageError, AccountStorageResult};
use crate::account::{AccountId, AccountInfo, AccountType, AccountWithCoins, AccountWithEnabledFlag,
                     MAX_ACCOUNT_DESCRIPTION_LENGTH, MAX_ACCOUNT_NAME_LENGTH, MAX_COIN_LENGTH};
use async_trait::async_trait;
use common::mm_number::BigDecimal;
use db_common::sql_constraint::UniqueConstraint;
use db_common::sql_create::{SqlColumn, SqlCreateTable, SqlType};
use db_common::sqlite::rusqlite::{Connection, Error as SqlError, Row, ToSql, NO_PARAMS};
use db_common::sqlite::SqliteConnShared;
use mm2_err_handle::prelude::*;
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Formatter;
use std::sync::MutexGuard;

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

impl fmt::Display for AccountType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            AccountType::Iguana => write!(f, "iguana"),
            AccountType::HD => write!(f, "hd"),
            AccountType::HW => write!(f, "hw"),
        }
    }
}

impl AccountType {
    const MAX_ACCOUNT_TYPE_LENGTH: usize = 6;
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
        let mut create_sql = SqlCreateTable::new(conn, "gui_account");
        create_sql
            .if_not_exist()
            .column(SqlColumn::new("account_type", SqlType::Integer).not_null())
            .column(SqlColumn::new("account_idx", SqlType::Integer))
            .column(
                SqlColumn::new("name", SqlType::Varchar(MAX_ACCOUNT_NAME_LENGTH))
                    .not_null()
                    .unique(),
            )
            .column(SqlColumn::new(
                "description",
                SqlType::Varchar(MAX_ACCOUNT_DESCRIPTION_LENGTH),
            ))
            .column(SqlColumn::new("balance_usd", SqlType::Real).not_null())
            .constraint(UniqueConstraint::new(["account_type", "account_idx"])?.name("account_type_idx_constraint"));
        create_sql.create().map_to_mm(AccountStorageError::from)
    }

    fn init_account_coins_table(conn: &Connection) -> AccountStorageResult<()> {
        let mut create_sql = SqlCreateTable::new(conn, "gui_account_coins");
        create_sql
            .if_not_exist()
            .column(SqlColumn::new("account_type", SqlType::Integer).not_null())
            .column(SqlColumn::new("account_idx", SqlType::Integer))
            .column(SqlColumn::new("coin", SqlType::Varchar(MAX_COIN_LENGTH)).not_null())
            .constraint(
                UniqueConstraint::new(["account_type", "account_idx", "coin"])?
                    .name("account_type_idx_coin_constraint"),
            );
        create_sql.create().map_to_mm(AccountStorageError::from)
    }

    fn init_enabled_account_table(conn: &Connection) -> AccountStorageResult<()> {
        let mut create_sql = SqlCreateTable::new(conn, "gui_account_enabled");
        create_sql
            .if_not_exist()
            .column(SqlColumn::new("account_type", SqlType::Varchar(AccountType::MAX_ACCOUNT_TYPE_LENGTH)).not_null())
            .column(SqlColumn::new("account_idx", SqlType::Integer));
        create_sql.create().map_to_mm(AccountStorageError::from)
    }
}

#[async_trait]
impl AccountStorage for SqliteAccountStorage {
    async fn init(&self) -> AccountStorageResult<()> {
        let mut conn = self.lock_conn()?;
        let transaction = conn.transaction()?;

        SqliteAccountStorage::init_account_table(&transaction)?;
        SqliteAccountStorage::init_account_coins_table(&transaction)?;
        SqliteAccountStorage::init_enabled_account_table(&transaction)
    }

    async fn load_accounts_with_enabled_flag(
        &self,
    ) -> AccountStorageResult<BTreeMap<AccountId, AccountWithEnabledFlag>> {
        todo!()
    }

    async fn load_enabled_account_id(&self) -> AccountStorageResult<AccountId> { todo!() }

    async fn load_enabled_account_with_coins(&self) -> AccountStorageResult<AccountWithCoins> { todo!() }

    async fn enable_account(&self, account_id: AccountId) -> AccountStorageResult<()> { todo!() }

    async fn upload_account(&self, account: AccountInfo) -> AccountStorageResult<()> { todo!() }

    async fn set_name(&self, account_id: AccountId, name: String) -> AccountStorageResult<()> { todo!() }

    async fn set_description(&self, account_id: AccountId, description: String) -> AccountStorageResult<()> { todo!() }

    async fn set_balance(&self, account_id: AccountId, balance_usd: BigDecimal) -> AccountStorageResult<()> { todo!() }

    async fn activate_coin(&self, account_id: AccountId, ticker: String) -> AccountStorageResult<()> { todo!() }

    async fn deactivate_coin(&self, account_id: AccountId, ticker: &str) -> AccountStorageResult<()> { todo!() }
}

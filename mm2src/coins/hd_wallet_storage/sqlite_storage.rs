use crate::hd_wallet_storage::{HDAccountStorageItem, HDWalletId, HDWalletStorageError, HDWalletStorageInternalOps,
                               HDWalletStorageResult};
use async_trait::async_trait;
use common::async_blocking;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use db_common::sqlite::rusqlite::{Connection, Error as SqlError, Row, ToSql, NO_PARAMS};
use db_common::sqlite::{SqliteConnShared, SqliteConnWeak};
use derive_more::Display;
use std::convert::TryFrom;
use std::sync::MutexGuard;

const CREATE_HD_ACCOUNT_TABLE: &str = "CREATE TABLE IF NOT EXISTS hd_account (
    coin VARCHAR(255) NOT NULL,
    mm2_rmd160 VARCHAR(255) NOT NULL,
    hd_wallet_rmd160 VARCHAR(255) NOT NULL,
    account_id INTEGER NOT NULL,
    account_xpub VARCHAR(255) NOT NULL,
    external_addresses_number INTEGER NOT NULL,
    internal_addresses_number INTEGER NOT NULL
);";

const INSERT_ACCOUNT: &str = "INSERT INTO hd_account
    (coin, mm2_rmd160, hd_wallet_rmd160, account_id, account_xpub, external_addresses_number, internal_addresses_number)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);";

const DELETE_ACCOUNTS_BY_WALLET_ID: &str =
    "DELETE FROM hd_account WHERE coin=?1 AND mm2_rmd160=?2 AND hd_wallet_rmd160=?3;";

const SELECT_ACCOUNT: &str = "SELECT account_id, account_xpub, external_addresses_number, internal_addresses_number
    FROM hd_account
    WHERE coin=?1 AND mm2_rmd160=?2 AND hd_wallet_rmd160=?3 AND account_id=?4;";

const SELECT_ACCOUNTS_BY_WALLET_ID: &str =
    "SELECT account_id, account_xpub, external_addresses_number, internal_addresses_number
    FROM hd_account
    WHERE coin=?1 AND mm2_rmd160=?2 AND hd_wallet_rmd160=?3;";

/// The max number of SQL query params.
const PARAMS_CAPACITY: usize = 7;

impl From<SqlError> for HDWalletStorageError {
    fn from(e: SqlError) -> Self {
        let error = e.to_string();
        match e {
            SqlError::FromSqlConversionFailure(_, _, _)
            | SqlError::IntegralValueOutOfRange(_, _)
            | SqlError::InvalidColumnIndex(_)
            | SqlError::InvalidColumnType(_, _, _) => HDWalletStorageError::ErrorDeserializing(error),
            SqlError::Utf8Error(_) | SqlError::NulError(_) | SqlError::ToSqlConversionFailure(_) => {
                HDWalletStorageError::ErrorSerializing(error)
            },
            _ => HDWalletStorageError::Internal(error),
        }
    }
}

impl TryFrom<&Row<'_>> for HDAccountStorageItem {
    type Error = SqlError;

    fn try_from(row: &Row<'_>) -> Result<Self, Self::Error> {
        Ok(HDAccountStorageItem {
            account_id: row.get(0)?,
            account_xpub: row.get(1)?,
            external_addresses_number: row.get(2)?,
            internal_addresses_number: row.get(3)?,
        })
    }
}

impl HDAccountStorageItem {
    fn to_sql_params_with_wallet_id(&self, wallet_id: HDWalletId) -> Vec<String> {
        let mut params = Vec::with_capacity(PARAMS_CAPACITY);
        wallet_id.fill_sql_params(&mut params);
        self.fill_sql_params(&mut params);
        params
    }

    fn fill_sql_params(&self, params: &mut Vec<String>) {
        params.push(self.account_id.to_string());
        params.push(self.account_xpub.clone());
        params.push(self.external_addresses_number.to_string());
        params.push(self.internal_addresses_number.to_string());
    }
}

impl HDWalletId {
    fn to_sql_params(&self) -> Vec<String> {
        let mut params = Vec::with_capacity(PARAMS_CAPACITY);
        self.fill_sql_params(&mut params);
        params
    }

    fn fill_sql_params(&self, params: &mut Vec<String>) {
        params.push(self.coin.clone());
        params.push(self.mm2_rmd160.clone());
        params.push(self.hd_wallet_rmd160.clone());
    }
}

#[derive(Clone)]
pub struct HDWalletSqliteStorage {
    conn: SqliteConnWeak,
}

#[async_trait]
impl HDWalletStorageInternalOps for HDWalletSqliteStorage {
    async fn init(ctx: &MmArc) -> HDWalletStorageResult<Self>
    where
        Self: Sized,
    {
        let shared = ctx
            .sqlite_connection
            .as_option()
            .or_mm_err(|| HDWalletStorageError::Internal("'MmCtx::sqlite_connection' is not initialized".to_owned()))?;
        let storage = HDWalletSqliteStorage {
            conn: SqliteConnShared::downgrade(shared),
        };
        storage.init_tables().await?;
        Ok(storage)
    }

    async fn load_accounts(&self, wallet_id: HDWalletId) -> HDWalletStorageResult<Vec<HDAccountStorageItem>> {
        let selfi = self.clone();
        async_blocking(move || {
            let conn_shared = selfi.get_shared_conn()?;
            let conn = Self::lock_conn(&conn_shared)?;

            let mut statement = conn.prepare(SELECT_ACCOUNTS_BY_WALLET_ID)?;

            let params = wallet_id.to_sql_params();
            let rows = statement
                .query_map(params, |row: &Row<'_>| HDAccountStorageItem::try_from(row))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
    }

    async fn load_account(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<HDAccountStorageItem>> {
        let selfi = self.clone();
        async_blocking(move || {
            let conn_shared = selfi.get_shared_conn()?;
            let conn = Self::lock_conn(&conn_shared)?;

            let mut params = wallet_id.to_sql_params();
            params.push(account_id.to_string());
            query_single_row(&conn, SELECT_ACCOUNT, params, |row: &Row<'_>| {
                HDAccountStorageItem::try_from(row)
            })
            .mm_err(HDWalletStorageError::from)
        })
        .await
    }

    async fn update_external_addresses_number(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_external_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        self.update_addresses_number(
            UpdatingProperty::ExternalAddressesNumber,
            wallet_id,
            account_id,
            new_external_addresses_number,
        )
        .await
    }

    async fn update_internal_addresses_number(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        self.update_addresses_number(
            UpdatingProperty::InternalAddressesNumber,
            wallet_id,
            account_id,
            new_internal_addresses_number,
        )
        .await
    }

    async fn upload_new_account(
        &self,
        wallet_id: HDWalletId,
        account: HDAccountStorageItem,
    ) -> HDWalletStorageResult<()> {
        let selfi = self.clone();
        async_blocking(move || {
            let conn_shared = selfi.get_shared_conn()?;
            let conn = Self::lock_conn(&conn_shared)?;

            let params = account.to_sql_params_with_wallet_id(wallet_id);
            conn.execute(INSERT_ACCOUNT, params)
                .map(|_| ())
                .map_to_mm(HDWalletStorageError::from)
        })
        .await
    }

    async fn clear_accounts(&self, wallet_id: HDWalletId) -> HDWalletStorageResult<()> {
        let selfi = self.clone();
        async_blocking(move || {
            let conn_shared = selfi.get_shared_conn()?;
            let conn = Self::lock_conn(&conn_shared)?;

            let params = wallet_id.to_sql_params();
            conn.execute(DELETE_ACCOUNTS_BY_WALLET_ID, params)
                .map(|_| ())
                .map_to_mm(HDWalletStorageError::from)
        })
        .await
    }
}

impl HDWalletSqliteStorage {
    fn get_shared_conn(&self) -> HDWalletStorageResult<SqliteConnShared> {
        self.conn
            .upgrade()
            .or_mm_err(|| HDWalletStorageError::Internal("'HDWalletSqliteStorage::conn' doesn't exist".to_owned()))
    }

    fn lock_conn(conn: &SqliteConnShared) -> HDWalletStorageResult<MutexGuard<Connection>> {
        conn.lock()
            .map_to_mm(|e| HDWalletStorageError::Internal(format!("Error locking sqlite connection: {}", e)))
    }

    async fn init_tables(&self) -> HDWalletStorageResult<()> {
        let conn_shared = self.get_shared_conn()?;
        let conn = Self::lock_conn(&conn_shared)?;
        conn.execute(CREATE_HD_ACCOUNT_TABLE, NO_PARAMS)
            .map(|_| ())
            .map_to_mm(HDWalletStorageError::from)
    }

    async fn update_addresses_number(
        &self,
        updating_property: UpdatingProperty,
        wallet_id: HDWalletId,
        account_id: u32,
        new_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        let sql = format!(
            "UPDATE hd_account SET {}=?1 WHERE coin=?2 AND mm2_rmd160=?3 AND hd_wallet_rmd160=?4 AND account_id=?5;",
            updating_property
        );

        let selfi = self.clone();
        async_blocking(move || {
            let conn_shared = selfi.get_shared_conn()?;
            let conn = Self::lock_conn(&conn_shared)?;

            let mut params = vec![new_addresses_number.to_string()];
            wallet_id.fill_sql_params(&mut params);
            params.push(account_id.to_string());

            conn.execute(&sql, params)
                .map(|_| ())
                .map_to_mm(HDWalletStorageError::from)
        })
        .await
    }
}

#[derive(Display)]
enum UpdatingProperty {
    #[display(fmt = "external_addresses_number")]
    ExternalAddressesNumber,
    #[display(fmt = "internal_addresses_number")]
    InternalAddressesNumber,
}

/// TODO remove this when `db_common::query_single_row` is merged into `dev`.
fn query_single_row<T, P, F>(conn: &Connection, query: &str, params: P, map_fn: F) -> MmResult<Option<T>, SqlError>
where
    P: IntoIterator,
    P::Item: ToSql,
    F: FnOnce(&Row<'_>) -> Result<T, SqlError>,
{
    let maybe_result = conn.query_row(query, params, map_fn);
    if let Err(SqlError::QueryReturnedNoRows) = maybe_result {
        return Ok(None);
    }

    let result = maybe_result?;
    Ok(Some(result))
}

/// This function is used in `hd_wallet_storage::tests`.
#[cfg(test)]
pub(super) async fn get_all_storage_items(ctx: &MmArc) -> Vec<HDAccountStorageItem> {
    const SELECT_ALL_ACCOUNTS: &str =
        "SELECT account_id, account_xpub, external_addresses_number, internal_addresses_number FROM hd_account";

    let conn = ctx.sqlite_connection();
    let mut statement = conn.prepare(SELECT_ALL_ACCOUNTS).unwrap();
    statement
        .query_map(NO_PARAMS, |row: &Row<'_>| HDAccountStorageItem::try_from(row))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

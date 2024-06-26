use crate::mm_ctx::{log_sqlite_file_open_attempt, path_to_dbdir, MmCtx};
use async_std::sync::RwLock as AsyncRwLock;
use common::log::error;
use db_common::async_sql_conn::AsyncConnection;
use db_common::sqlite::rusqlite::Connection;
use futures::channel::mpsc::{channel, Receiver, Sender};
use futures::lock::Mutex as AsyncMutex;
use gstuff::try_s;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

pub const ASYNC_SQLITE_DB_ID: &str = "KOMODEFI.db";
const SYNC_SQLITE_DB_ID: &str = "MM2.db";
const SQLITE_SHARED_DB_ID: &str = "MM2-shared.db";

/// Represents the kind of database connection ID: either shared or single-user.
enum DbIdConnKind {
    Shared,
    Single,
}

/// A pool for managing SQLite connections, where each connection is keyed by a unique string identifier.
#[derive(Clone)]
pub struct SqliteConnPool {
    connections: Arc<RwLock<HashMap<String, Arc<Mutex<Connection>>>>>,
    // default db_id
    rmd160_hex: String,
    // default shared_db_id
    shared_db_id: String,
    db_root: Option<String>,
}

impl SqliteConnPool {
    /// Initializes a single-user database connection.
    pub fn init(ctx: &MmCtx, db_id: Option<&str>) -> Result<(), String> {
        Self::init_impl(ctx, db_id, DbIdConnKind::Single)
    }

    /// Initializes a shared database connection.
    pub fn init_shared(ctx: &MmCtx, db_id: Option<&str>) -> Result<(), String> {
        Self::init_impl(ctx, db_id, DbIdConnKind::Shared)
    }

    /// Internal implementation to initialize a database connection.
    fn init_impl(ctx: &MmCtx, db_id: Option<&str>, kind: DbIdConnKind) -> Result<(), String> {
        let db_id = Self::db_id_from_ctx(ctx, db_id, &kind);
        let sqlite_file_path = match kind {
            DbIdConnKind::Shared => ctx.shared_dbdir(Some(&db_id)).join(SQLITE_SHARED_DB_ID),
            DbIdConnKind::Single => ctx.dbdir(Some(&db_id)).join(SYNC_SQLITE_DB_ID),
        };

        // Connection pool is already initialized, insert new connection.
        if let Some(pool) = ctx.sqlite_conn_pool.as_option() {
            let conn = Self::open_connection(sqlite_file_path);
            let mut pool = pool.connections.write().unwrap();
            pool.insert(db_id, conn);

            return Ok(());
        }

        // Connection pool is not already initialized, create new connection pool.
        let conn = Self::open_connection(sqlite_file_path);
        let connections = Arc::new(RwLock::new(HashMap::from([(db_id, conn)])));
        let db_root = ctx.conf["dbdir"].as_str();
        try_s!(ctx.sqlite_conn_pool.pin(Self {
            connections,
            rmd160_hex: ctx.rmd160_hex(),
            shared_db_id: hex::encode(*ctx.shared_db_id()),
            db_root: db_root.map(|d| d.to_owned())
        }));

        Ok(())
    }

    /// Test method for initializing a single-user database connection in-memory.
    pub fn init_test(ctx: &MmCtx) -> Result<(), String> { Self::init_impl_test(ctx, None, DbIdConnKind::Single) }

    /// Test method for initializing a shared database connection in-memory.
    pub fn init_shared_test(ctx: &MmCtx) -> Result<(), String> { Self::init_impl_test(ctx, None, DbIdConnKind::Shared) }

    /// Internal test implementation to initialize a database connection in-memory.
    fn init_impl_test(ctx: &MmCtx, db_id: Option<&str>, kind: DbIdConnKind) -> Result<(), String> {
        let db_id = Self::db_id_from_ctx(ctx, db_id, &kind);
        if let Some(pool) = ctx.sqlite_conn_pool.as_option() {
            let connection = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
            let mut pool = pool.connections.write().unwrap();
            pool.insert(db_id, connection);

            return Ok(());
        }

        let connection = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let connections = Arc::new(RwLock::new(HashMap::from([(db_id, connection)])));
        let db_root = ctx.conf["dbdir"].as_str();
        try_s!(ctx.sqlite_conn_pool.pin(Self {
            connections,
            rmd160_hex: ctx.rmd160_hex(),
            shared_db_id: hex::encode(*ctx.shared_db_id()),
            db_root: db_root.map(|d| d.to_owned())
        }));

        Ok(())
    }

    /// Retrieves a single-user connection from the pool.
    pub fn sqlite_conn(&self, db_id: Option<&str>) -> Arc<Mutex<Connection>> {
        self.sqlite_conn_impl(db_id, DbIdConnKind::Single)
    }

    /// Retrieves a shared connection from the pool.
    pub fn sqlite_conn_shared(&self, db_id: Option<&str>) -> Arc<Mutex<Connection>> {
        self.sqlite_conn_impl(db_id, DbIdConnKind::Shared)
    }

    /// Internal implementation to retrieve or create a connection.
    fn sqlite_conn_impl(&self, db_id: Option<&str>, kind: DbIdConnKind) -> Arc<Mutex<Connection>> {
        let db_id = self.db_id(db_id, &kind);
        let connections = self.connections.read().unwrap();
        if let Some(connection) = connections.get(&db_id) {
            return Arc::clone(connection);
        }
        drop(connections);

        let mut connections = self.connections.write().unwrap();
        let sqlite_file_path = self.sqlite_file_path(&db_id, &kind);
        let connection = Self::open_connection(sqlite_file_path);
        connections.insert(db_id, Arc::clone(&connection));

        connection
    }

    /// Retrieves a single-user connection from the pool.
    pub fn run_sql_query<F, R>(&self, db_id: Option<&str>, f: F) -> R
    where
        F: FnOnce(MutexGuard<Connection>) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.run_sql_query_impl(db_id, DbIdConnKind::Single, f)
    }

    /// Retrieves a shared connection from the pool.
    pub fn run_sql_query_shared<F, R>(&self, db_id: Option<&str>, f: F) -> R
    where
        F: FnOnce(MutexGuard<Connection>) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.run_sql_query_impl(db_id, DbIdConnKind::Shared, f)
    }

    /// Internal run a sql query.
    fn run_sql_query_impl<F, R>(&self, db_id: Option<&str>, kind: DbIdConnKind, f: F) -> R
    where
        F: FnOnce(MutexGuard<Connection>) -> R + Send + 'static,
        R: Send + 'static,
    {
        let db_id = self.db_id(db_id, &kind);
        let connections = self.connections.read().unwrap();
        if let Some(connection) = connections.get(&db_id) {
            let conn = connection.lock().unwrap();
            return f(conn);
        }
        drop(connections);

        let mut connections = self.connections.write().unwrap();
        let sqlite_file_path = self.sqlite_file_path(&db_id, &kind);
        let connection = Self::open_connection(sqlite_file_path);
        connections.insert(db_id, Arc::clone(&connection));

        f(connection.lock().unwrap())
    }

    pub fn add_test_db(&self, db_id: String) {
        let mut connections = self.connections.write().unwrap();
        connections.insert(db_id, Arc::new(Mutex::new(Connection::open_in_memory().unwrap())));
    }

    /// Opens a database connection based on the database ID and connection kind.
    fn open_connection(sqlite_file_path: PathBuf) -> Arc<Mutex<Connection>> {
        log_sqlite_file_open_attempt(&sqlite_file_path);
        Arc::new(Mutex::new(
            Connection::open(sqlite_file_path).expect("failed to open db"),
        ))
    }

    fn db_dir(&self, db_id: &str) -> PathBuf { path_to_dbdir(self.db_root.as_deref(), db_id) }
    fn db_id(&self, db_id: Option<&str>, kind: &DbIdConnKind) -> String {
        match kind {
            DbIdConnKind::Shared => db_id
                .map(|e| e.to_owned())
                .unwrap_or_else(|| self.shared_db_id.to_owned()),
            DbIdConnKind::Single => db_id
                .map(|e| e.to_owned())
                .unwrap_or_else(|| self.rmd160_hex.to_owned()),
        }
    }
    fn db_id_from_ctx(ctx: &MmCtx, db_id: Option<&str>, kind: &DbIdConnKind) -> String {
        match kind {
            DbIdConnKind::Shared => db_id
                .map(|e| e.to_owned())
                .unwrap_or_else(|| hex::encode(ctx.shared_db_id().as_slice())),
            DbIdConnKind::Single => db_id.map(|e| e.to_owned()).unwrap_or_else(|| ctx.rmd160_hex()),
        }
    }
    fn sqlite_file_path(&self, db_id: &str, kind: &DbIdConnKind) -> PathBuf {
        self.db_dir(&db_id).join(match kind {
            DbIdConnKind::Shared => SQLITE_SHARED_DB_ID,
            DbIdConnKind::Single => SYNC_SQLITE_DB_ID,
        })
    }
}

/// A pool for managing async SQLite connections, where each connection is keyed by a unique string identifier.
#[derive(Clone)]
pub struct AsyncSqliteConnPool {
    connections: Arc<AsyncRwLock<HashMap<String, Arc<AsyncMutex<AsyncConnection>>>>>,
    sqlite_file_path: PathBuf,
    rmd160_hex: String,
}

impl AsyncSqliteConnPool {
    /// Initialize a database connection.
    pub async fn init(ctx: &MmCtx, db_id: Option<&str>) -> Result<(), String> {
        let db_id = db_id.map(|e| e.to_owned()).unwrap_or_else(|| ctx.rmd160_hex());

        if let Some(pool) = ctx.async_sqlite_conn_pool.as_option() {
            let conn = Self::open_connection(&pool.sqlite_file_path).await;
            let mut pool = pool.connections.write().await;
            pool.insert(db_id, conn);

            return Ok(());
        }

        let sqlite_file_path = ctx.dbdir(Some(&db_id)).join(ASYNC_SQLITE_DB_ID);
        let conn = Self::open_connection(&sqlite_file_path).await;
        let connections = Arc::new(AsyncRwLock::new(HashMap::from([(db_id, conn)])));
        try_s!(ctx.async_sqlite_conn_pool.pin(Self {
            connections,
            sqlite_file_path,
            rmd160_hex: ctx.rmd160_hex(),
        }));

        Ok(())
    }

    /// Initialize a database connection.
    pub async fn init_test(ctx: &MmCtx, db_id: Option<&str>) -> Result<(), String> {
        let db_id = db_id.map(|e| e.to_owned()).unwrap_or_else(|| ctx.rmd160_hex());

        if let Some(pool) = ctx.async_sqlite_conn_pool.as_option() {
            let mut pool = pool.connections.write().await;
            let conn = Arc::new(AsyncMutex::new(AsyncConnection::open_in_memory().await.unwrap()));
            pool.insert(db_id, conn);

            return Ok(());
        }

        let conn = Arc::new(AsyncMutex::new(AsyncConnection::open_in_memory().await.unwrap()));
        // extra connection to test accessing different db test
        let conn2 = Arc::new(AsyncMutex::new(AsyncConnection::open_in_memory().await.unwrap()));
        let connections = HashMap::from([(db_id, conn), ("TEST_DB_ID".to_owned(), conn2)]);
        let connections = Arc::new(AsyncRwLock::new(connections));
        try_s!(ctx.async_sqlite_conn_pool.pin(Self {
            connections,
            sqlite_file_path: PathBuf::new(),
            rmd160_hex: ctx.rmd160_hex(),
        }));
        Ok(())
    }

    /// Retrieve or create a connection.
    pub async fn async_sqlite_conn(&self, db_id: Option<&str>) -> Arc<AsyncMutex<AsyncConnection>> {
        let db_id = db_id.unwrap_or(&self.rmd160_hex);

        let connections = self.connections.read().await;
        if let Some(connection) = connections.get(db_id) {
            return Arc::clone(connection);
        };

        let mut connections = self.connections.write().await;
        let connection = Self::open_connection(&self.sqlite_file_path).await;
        connections.insert(db_id.to_owned(), Arc::clone(&connection));
        connection
    }

    pub async fn close_connections(&self) {
        let mut connections = self.connections.write().await;
        for (id, connection) in connections.iter_mut() {
            let mut connection = connection.lock().await;
            if let Err(e) = connection.close().await {
                error!("Error stopping AsyncConnection: {}, connection_id=({id})", e);
            }
        }
    }

    async fn open_connection(sqlite_file_path: &PathBuf) -> Arc<AsyncMutex<AsyncConnection>> {
        log_sqlite_file_open_attempt(sqlite_file_path);

        Arc::new(AsyncMutex::new(
            AsyncConnection::open(sqlite_file_path)
                .await
                .expect("failed to open db"),
        ))
    }
}

pub struct DbIds {
    pub db_id: String,
    pub shared_db_id: String,
}

pub type DbMigrationHandler = Arc<AsyncMutex<Receiver<DbIds>>>;
pub type DbMigrationSender = Arc<AsyncMutex<Sender<DbIds>>>;

pub struct DbMigrationWatcher {
    sender: DbMigrationSender,
}

impl DbMigrationWatcher {
    pub fn init(ctx: &MmCtx) -> Result<DbMigrationHandler, String> {
        let (sender, receiver) = channel(1);

        let selfi = Arc::new(Self {
            sender: Arc::new(AsyncMutex::new(sender)),
        });
        try_s!(ctx.db_migration_watcher.pin(selfi));

        Ok(Arc::new(AsyncMutex::new(receiver)))
    }

    pub fn get_sender(&self) -> DbMigrationSender { self.sender.clone() }
}

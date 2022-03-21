//! # Usage
//!
//! As an example, the following table will be used:
//! ```
//! | uuid                                   | base_coin | rel_coin | base_coin_value | started_at |
//! | "c52659d7-4e13-41f5-9c1a-30cc2f646033" | "RICK"    | "MORTY"  | 10              | 1000000029 |
//! | "5acb0e63-8b26-469e-81df-7dd9e4a9ad15" | "RICK"    | "MORTY"  | 13              | 1000000030 |
//! | "9db641f5-4300-4527-9fa6-f1c391d42c35" | "RICK"    | "MORTY"  | 1.2             | 1000000031 |
//! ```
//! with the `search_index` index created by:
//! ```rust
//! TableUpgrader::create_multi_index(self, "search_index", &["base_coin", "rel_coin", "started_at"]).unwrap();
//! ```
//!
//! If you want to find all `RICK/MORTY` swaps where
//! 1) `10 <= base_coin_value <= 13`
//! 2) `started_at <= 1000000030`,
//! you can use [`WithBound::bound`] along with [`WithOnly::only`]:
//! ```rust
//! let table = open_table_somehow();
//! let all_rick_morty_swaps = table.open_cursor("search_index")
//!     .only("base_coin", "RICK", "MORTY")?
//!     .bound("base_coin_value", 10, 13)
//!     .bound("started_at", 1000000030.into(), u32::MAX.into())
//!     .collect()
//!     .await?;
//! ```
//!
//! # Under the hood
//!
//! In the example above, [`CursorOps::collect`] actually creates a JavaScript cursor with the specified key range:
//! ```js
//! var key_range = IDBKeyRange.bound(['RICK', 'MORTY', 10, 1000000030], ['RICK', 'MORTY', 13, 9999999999]);
//! var cursor = table.index('search_index').openCursor(key_range);
//! ```
//!
//! And after that, the database engine compares each record with the specified min and max bounds sequentially from one field to another.
//! Please note `['RICK', 'MORTY', 10, 1000000029]` <= `['RICK', 'MORTY', 11, 2000000000]`.
//!
//! # Important
//!
//! Please make sure all keys of the index are specified
//! by the [`IdbBoundCursorBuilder::only`] or/and [`IdbBoundCursorBuilder::bound`] methods,
//! and they are specified in the same order as they were declared on [`TableUpgrader::create_multi_index`].
//!
//! It's important because if you skip f the `started_at` key, for example, the bounds will be:
//! min = ['RICK', 'MORTY', 10], max = ['RICK', 'MORTY', 13],
//! but an actual record ['RICK', 'MORTY', 13, 1000000030] will not be included in the result,
//! because ['RICK', 'MORTY', 13] < ['RICK', 'MORTY', 13, 1000000030],
//! although it is expected to be within the specified bounds.

use crate::indexed_db::db_driver::cursor::{CollectCursorAction, CollectItemAction, CursorBoundValue, CursorOps,
                                           DbFilter, IdbCursorBuilder};
pub use crate::indexed_db::db_driver::cursor::{CursorError, CursorResult};
use crate::indexed_db::{ItemId, TableSignature};
use crate::mm_error::prelude::*;
use crate::serde::de::DeserializeOwned;
use async_trait::async_trait;
use futures::channel::{mpsc, oneshot};
use futures::StreamExt;
use serde::Serialize;
use serde_json::{self as json, Value as Json};
use std::fmt;
use std::marker::PhantomData;

pub(super) type DbCursorEventTx = mpsc::UnboundedSender<DbCursorEvent>;
pub(super) type DbCursorEventRx = mpsc::UnboundedReceiver<DbCursorEvent>;

pub enum DbCursorEvent {
    Collect {
        options: CursorCollectOptions,
        result_tx: oneshot::Sender<CursorResult<Vec<(ItemId, Json)>>>,
    },
}

pub enum CursorCollectOptions {
    SingleKey {
        field_name: String,
        field_value: Json,
        filter: Option<DbFilter>,
    },
    SingleKeyBound {
        field_name: String,
        lower_bound: CursorBoundValue,
        upper_bound: CursorBoundValue,
        filter: Option<DbFilter>,
    },
    MultiKey {
        keys: Vec<(String, Json)>,
        filter: Option<DbFilter>,
    },
    MultiKeyBound {
        only_keys: Vec<(String, Json)>,
        bound_keys: Vec<(String, CursorBoundValue, CursorBoundValue)>,
        filter: Option<DbFilter>,
    },
}

pub async fn cursor_event_loop(mut rx: DbCursorEventRx, cursor_builder: IdbCursorBuilder) {
    while let Some(event) = rx.next().await {
        match event {
            DbCursorEvent::Collect { options, result_tx } => {
                on_collect_cursor_event(result_tx, options, cursor_builder).await;
                return;
            },
        }
    }
}

async fn on_collect_cursor_event(
    result_tx: oneshot::Sender<CursorResult<Vec<(ItemId, Json)>>>,
    options: CursorCollectOptions,
    cursor_builder: IdbCursorBuilder,
) {
    async fn on_collect_cursor_event_impl(
        options: CursorCollectOptions,
        cursor_builder: IdbCursorBuilder,
    ) -> CursorResult<Vec<(ItemId, Json)>> {
        match options {
            CursorCollectOptions::SingleKey {
                field_name,
                field_value,
                filter,
            } => {
                cursor_builder
                    .single_key_cursor(field_name, field_value, filter)
                    .collect()
                    .await
            },
            CursorCollectOptions::SingleKeyBound {
                field_name,
                lower_bound,
                upper_bound,
                filter,
            } => {
                cursor_builder
                    .single_key_bound_cursor(field_name, lower_bound, upper_bound, filter)?
                    .collect()
                    .await
            },
            CursorCollectOptions::MultiKey { keys, filter } => {
                cursor_builder.multi_key_cursor(keys, filter)?.collect().await
            },
            CursorCollectOptions::MultiKeyBound {
                only_keys,
                bound_keys,
                filter,
            } => {
                cursor_builder
                    .multi_key_bound_cursor(only_keys, bound_keys, filter)?
                    .collect()
                    .await
            },
        }
    }

    let result = on_collect_cursor_event_impl(options, cursor_builder).await;
    result_tx.send(result).ok();
}

pub trait WithOnly: Sized {
    type ResultCursor;

    fn only<Value>(self, field_name: &str, field_value: Value) -> CursorResult<Self::ResultCursor>
    where
        Value: Serialize + fmt::Debug,
    {
        let field_value_str = format!("{:?}", field_value);
        let field_value = json::to_value(field_value).map_to_mm(|e| CursorError::ErrorSerializingIndexFieldValue {
            field: field_name.to_owned(),
            value: field_value_str,
            description: e.to_string(),
        })?;
        Ok(self.only_json(field_name, field_value))
    }

    fn only_json(self, field_name: &str, field_value: Json) -> Self::ResultCursor;
}

pub trait WithBound: Sized {
    type ResultCursor;

    fn bound<Value>(self, field_name: &str, lower_bound: Value, upper_bound: Value) -> Self::ResultCursor
    where
        CursorBoundValue: From<Value>,
    {
        let lower_bound = CursorBoundValue::from(lower_bound);
        let upper_bound = CursorBoundValue::from(upper_bound);
        self.bound_values(field_name, lower_bound, upper_bound)
    }

    fn bound_values(
        self,
        field_name: &str,
        lower_bound: CursorBoundValue,
        upper_bound: CursorBoundValue,
    ) -> Self::ResultCursor;
}

pub trait WithFilter: Sized {
    fn filter<F>(self, filter: F) -> Self
    where
        F: FnMut(&Json) -> (CollectItemAction, CollectCursorAction) + Send + 'static,
    {
        let filter = Box::new(filter);
        self.filter_boxed(filter)
    }

    fn filter_boxed(self, filter: DbFilter) -> Self;
}

#[async_trait]
pub trait CollectCursor<Table: DeserializeOwned> {
    async fn collect(self) -> CursorResult<Vec<(ItemId, Table)>>;
}

#[async_trait]
impl<Table: DeserializeOwned + 'static, T: CollectCursorImpl<Table> + Send> CollectCursor<Table> for T {
    async fn collect(self) -> CursorResult<Vec<(ItemId, Table)>> { self.collect_impl().await }
}

#[async_trait]
pub trait CollectCursorImpl<Table: DeserializeOwned + 'static>: Sized {
    fn into_collect_options(self) -> CursorCollectOptions;

    fn event_tx(&self) -> DbCursorEventTx;

    async fn collect_impl(self) -> CursorResult<Vec<(ItemId, Table)>> {
        let event_tx = self.event_tx();
        let options = self.into_collect_options();

        let (result_tx, result_rx) = oneshot::channel();
        let event = DbCursorEvent::Collect { result_tx, options };
        let items: Vec<(ItemId, Json)> = send_event_recv_response(&event_tx, event, result_rx).await?;

        items
            .into_iter()
            .map(|(item_id, item)| json::from_value(item).map(|item| (item_id, item)))
            .map(|res| res.map_to_mm(|e| CursorError::ErrorDeserializingItem(e.to_string())))
            // Item = CursorResult<(ItemId, Table)>
            .collect()
    }
}

pub struct DbEmptyCursor<'a, Table: TableSignature> {
    event_tx: DbCursorEventTx,
    filter: Option<DbFilter>,
    phantom: PhantomData<&'a Table>,
}

impl<'a, Table: TableSignature> WithOnly for DbEmptyCursor<'a, Table> {
    type ResultCursor = DbSingleKeyCursor<'a, Table>;

    fn only_json(self, field_name: &str, field_value: Json) -> Self::ResultCursor {
        DbSingleKeyCursor {
            event_tx: self.event_tx,
            field_name: field_name.to_owned(),
            field_value,
            filter: self.filter,
            phantom: PhantomData::default(),
        }
    }
}

impl<'a, Table: TableSignature> WithBound for DbEmptyCursor<'a, Table> {
    type ResultCursor = DbSingleKeyBoundCursor<'a, Table>;

    fn bound_values(
        self,
        field_name: &str,
        lower_bound: CursorBoundValue,
        upper_bound: CursorBoundValue,
    ) -> Self::ResultCursor {
        DbSingleKeyBoundCursor {
            event_tx: self.event_tx,
            field_name: field_name.to_owned(),
            lower_bound,
            upper_bound,
            filter: self.filter,
            phantom: PhantomData::default(),
        }
    }
}

impl<'a, Table: TableSignature> WithFilter for DbEmptyCursor<'a, Table> {
    fn filter_boxed(mut self, filter: DbFilter) -> Self {
        self.filter = Some(filter);
        self
    }
}

impl<'a, Table: TableSignature> DbEmptyCursor<'a, Table> {
    pub(super) fn new(event_tx: DbCursorEventTx) -> DbEmptyCursor<'a, Table> {
        DbEmptyCursor {
            event_tx,
            filter: None,
            phantom: PhantomData::default(),
        }
    }
}

pub struct DbSingleKeyCursor<'a, Table: TableSignature> {
    event_tx: DbCursorEventTx,
    field_name: String,
    field_value: Json,
    filter: Option<DbFilter>,
    phantom: PhantomData<&'a Table>,
}

impl<'a, Table: TableSignature> WithOnly for DbSingleKeyCursor<'a, Table> {
    type ResultCursor = DbMultiKeyCursor<'a, Table>;

    fn only_json(self, field_name: &str, field_value: Json) -> Self::ResultCursor {
        let keys = vec![
            (self.field_name, self.field_value),
            (field_name.to_owned(), field_value),
        ];
        DbMultiKeyCursor {
            event_tx: self.event_tx,
            keys,
            filter: self.filter,
            phantom: PhantomData::default(),
        }
    }
}

impl<'a, Table: TableSignature> WithBound for DbSingleKeyCursor<'a, Table> {
    type ResultCursor = DbMultiKeyBoundCursor<'a, Table>;

    fn bound_values(
        self,
        field_name: &str,
        lower_bound: CursorBoundValue,
        upper_bound: CursorBoundValue,
    ) -> Self::ResultCursor {
        let only_keys = vec![(self.field_name, self.field_value)];
        let bound_keys = vec![(field_name.to_owned(), lower_bound, upper_bound)];
        DbMultiKeyBoundCursor {
            event_tx: self.event_tx,
            only_keys,
            bound_keys,
            filter: self.filter,
            phantom: PhantomData::default(),
        }
    }
}

impl<'a, Table: TableSignature> WithFilter for DbSingleKeyCursor<'a, Table> {
    fn filter_boxed(mut self, filter: DbFilter) -> Self {
        self.filter = Some(filter);
        self
    }
}

#[async_trait]
impl<'a, Table: TableSignature> CollectCursorImpl<Table> for DbSingleKeyCursor<'a, Table> {
    fn into_collect_options(self) -> CursorCollectOptions {
        CursorCollectOptions::SingleKey {
            field_name: self.field_name,
            field_value: self.field_value,
            filter: self.filter,
        }
    }

    fn event_tx(&self) -> DbCursorEventTx { self.event_tx.clone() }
}

/// `DbSingleKeyBoundCursor` doesn't implement `WithOnly` trait, because indexes MUST start with `only` values.
pub struct DbSingleKeyBoundCursor<'a, Table: TableSignature> {
    event_tx: DbCursorEventTx,
    field_name: String,
    lower_bound: CursorBoundValue,
    upper_bound: CursorBoundValue,
    filter: Option<DbFilter>,
    phantom: PhantomData<&'a Table>,
}

impl<'a, Table: TableSignature> WithBound for DbSingleKeyBoundCursor<'a, Table> {
    type ResultCursor = DbMultiKeyBoundCursor<'a, Table>;

    fn bound_values(
        self,
        field_name: &str,
        lower_bound: CursorBoundValue,
        upper_bound: CursorBoundValue,
    ) -> Self::ResultCursor {
        let bound_keys = vec![
            (self.field_name, self.lower_bound, self.upper_bound),
            (field_name.to_owned(), lower_bound, upper_bound),
        ];
        DbMultiKeyBoundCursor {
            event_tx: self.event_tx,
            only_keys: Vec::new(),
            bound_keys,
            filter: self.filter,
            phantom: PhantomData::default(),
        }
    }
}

impl<'a, Table: TableSignature> WithFilter for DbSingleKeyBoundCursor<'a, Table> {
    fn filter_boxed(mut self, filter: DbFilter) -> Self {
        self.filter = Some(filter);
        self
    }
}

#[async_trait]
impl<'a, Table: TableSignature> CollectCursorImpl<Table> for DbSingleKeyBoundCursor<'a, Table> {
    fn into_collect_options(self) -> CursorCollectOptions {
        CursorCollectOptions::SingleKeyBound {
            field_name: self.field_name,
            lower_bound: self.lower_bound,
            upper_bound: self.upper_bound,
            filter: self.filter,
        }
    }

    fn event_tx(&self) -> DbCursorEventTx { self.event_tx.clone() }
}

pub struct DbMultiKeyCursor<'a, Table: TableSignature> {
    event_tx: DbCursorEventTx,
    keys: Vec<(String, Json)>,
    filter: Option<DbFilter>,
    phantom: PhantomData<&'a Table>,
}

impl<'a, Table: TableSignature> WithOnly for DbMultiKeyCursor<'a, Table> {
    type ResultCursor = DbMultiKeyCursor<'a, Table>;

    fn only_json(mut self, field_name: &str, field_value: Json) -> Self::ResultCursor {
        self.keys.push((field_name.to_owned(), field_value));
        self
    }
}

impl<'a, Table: TableSignature> WithBound for DbMultiKeyCursor<'a, Table> {
    type ResultCursor = DbMultiKeyBoundCursor<'a, Table>;

    fn bound_values(
        self,
        field_name: &str,
        lower_bound: CursorBoundValue,
        upper_bound: CursorBoundValue,
    ) -> Self::ResultCursor {
        DbMultiKeyBoundCursor {
            event_tx: self.event_tx,
            only_keys: self.keys,
            bound_keys: vec![(field_name.to_owned(), lower_bound, upper_bound)],
            filter: self.filter,
            phantom: PhantomData::default(),
        }
    }
}

impl<'a, Table: TableSignature> WithFilter for DbMultiKeyCursor<'a, Table> {
    fn filter_boxed(mut self, filter: DbFilter) -> Self {
        self.filter = Some(filter);
        self
    }
}

#[async_trait]
impl<'a, Table: TableSignature> CollectCursorImpl<Table> for DbMultiKeyCursor<'a, Table> {
    fn into_collect_options(self) -> CursorCollectOptions {
        CursorCollectOptions::MultiKey {
            keys: self.keys,
            filter: self.filter,
        }
    }

    fn event_tx(&self) -> DbCursorEventTx { self.event_tx.clone() }
}

/// `DbMultiKeyBoundCursor` doesn't implement `WithOnly` trait, because indexes MUST start with `only` values.
pub struct DbMultiKeyBoundCursor<'a, Table: TableSignature> {
    event_tx: DbCursorEventTx,
    only_keys: Vec<(String, Json)>,
    bound_keys: Vec<(String, CursorBoundValue, CursorBoundValue)>,
    filter: Option<DbFilter>,
    phantom: PhantomData<&'a Table>,
}

impl<'a, Table: TableSignature> WithBound for DbMultiKeyBoundCursor<'a, Table> {
    type ResultCursor = DbMultiKeyBoundCursor<'a, Table>;

    fn bound_values(
        mut self,
        field_name: &str,
        lower_bound: CursorBoundValue,
        upper_bound: CursorBoundValue,
    ) -> Self::ResultCursor {
        self.bound_keys.push((field_name.to_owned(), lower_bound, upper_bound));
        self
    }
}

impl<'a, Table: TableSignature> WithFilter for DbMultiKeyBoundCursor<'a, Table> {
    fn filter_boxed(mut self, filter: DbFilter) -> Self {
        self.filter = Some(filter);
        self
    }
}

#[async_trait]
impl<'a, Table: TableSignature> CollectCursorImpl<Table> for DbMultiKeyBoundCursor<'a, Table> {
    fn into_collect_options(self) -> CursorCollectOptions {
        CursorCollectOptions::MultiKeyBound {
            only_keys: self.only_keys,
            bound_keys: self.bound_keys,
            filter: self.filter,
        }
    }

    fn event_tx(&self) -> DbCursorEventTx { self.event_tx.clone() }
}

async fn send_event_recv_response<Event, Result>(
    event_tx: &mpsc::UnboundedSender<Event>,
    event: Event,
    result_rx: oneshot::Receiver<CursorResult<Result>>,
) -> CursorResult<Result> {
    if let Err(e) = event_tx.unbounded_send(event) {
        let error = format!("Error sending event: {}", e);
        return MmError::err(CursorError::UnexpectedState(error));
    }
    match result_rx.await {
        Ok(result) => result,
        Err(e) => {
            let error = format!("Error receiving result: {}", e);
            MmError::err(CursorError::UnexpectedState(error))
        },
    }
}

mod tests {
    use super::*;
    use crate::for_tests::register_wasm_log;
    use crate::indexed_db::{DbIdentifier, DbTable, DbUpgrader, IndexedDbBuilder, OnUpgradeResult};
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    macro_rules! swap_item {
        ($uuid:literal, $base_coin:literal, $rel_coin:literal, $base_coin_value:expr, $rel_coin_value:expr, $started_at:expr) => {
            SwapTable {
                uuid: $uuid.to_owned(),
                base_coin: $base_coin.to_owned(),
                rel_coin: $rel_coin.to_owned(),
                base_coin_value: $base_coin_value,
                rel_coin_value: $rel_coin_value,
                started_at: $started_at,
            }
        };
    }

    #[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
    #[serde(deny_unknown_fields)]
    struct SwapTable {
        uuid: String,
        base_coin: String,
        rel_coin: String,
        base_coin_value: u32,
        rel_coin_value: u32,
        started_at: i32,
    }

    impl TableSignature for SwapTable {
        fn table_name() -> &'static str { "swap_test_table" }

        fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, _new_version: u32) -> OnUpgradeResult<()> {
            if old_version > 0 {
                // the table is initialized already
                return Ok(());
            }
            let table_upgrader = upgrader.create_table("swap_test_table")?;
            table_upgrader.create_index("base_coin", false)?;
            table_upgrader.create_index("rel_coin_value", false)?;
            table_upgrader.create_multi_index(
                "all_fields_index",
                &[
                    "base_coin",
                    "rel_coin",
                    "base_coin_value",
                    "rel_coin_value",
                    "started_at",
                ],
                false,
            )?;
            table_upgrader.create_multi_index(
                "basecoin_basecoinvalue_startedat_index",
                &["base_coin", "base_coin_value", "started_at"],
                false,
            )
        }
    }

    async fn fill_table(table: &DbTable<'_, SwapTable>, items: Vec<SwapTable>) {
        for item in items {
            table
                .add_item(&item)
                .await
                .expect(&format!("Error adding {:?} item", item));
        }
    }

    #[wasm_bindgen_test]
    async fn test_collect_single_key_cursor() {
        const DB_NAME: &str = "TEST_COLLECT_SINGLE_KEY_CURSOR";
        const DB_VERSION: u32 = 1;

        register_wasm_log();

        let items = vec![
            swap_item!("uuid1", "RICK", "MORTY", 10, 1, 700), // +
            swap_item!("uuid2", "MORTY", "KMD", 95000, 1, 721),
            swap_item!("uuid3", "RICK", "XYZ", 7, 6, 721),   // +
            swap_item!("uuid4", "RICK", "MORTY", 8, 6, 721), // +
            swap_item!("uuid5", "KMD", "MORTY", 12, 3, 721),
            swap_item!("uuid6", "QRC20", "RICK", 2, 2, 721),
        ];

        let db = IndexedDbBuilder::new(DbIdentifier::for_test(DB_NAME))
            .with_version(DB_VERSION)
            .with_table::<SwapTable>()
            .build()
            .await
            .expect("!IndexedDb::init");
        let transaction = db.transaction().await.expect("!IndexedDb::transaction");
        let table = transaction
            .table::<SwapTable>()
            .await
            .expect("!DbTransaction::open_table");
        fill_table(&table, items).await;

        let mut actual_items = table
            .open_cursor("base_coin")
            .await
            .expect("!DbTable::open_cursor")
            .only("base_coin", "RICK")
            .expect("!DbEmptyCursor::only")
            .collect()
            .await
            .expect("!DbSingleKeyCursor::collect")
            .into_iter()
            .map(|(_item_id, item)| item)
            .collect::<Vec<_>>();
        actual_items.sort();

        let mut expected_items = vec![
            swap_item!("uuid1", "RICK", "MORTY", 10, 1, 700),
            swap_item!("uuid3", "RICK", "XYZ", 7, 6, 721),
            swap_item!("uuid4", "RICK", "MORTY", 8, 6, 721),
        ];
        expected_items.sort();

        assert_eq!(actual_items, expected_items);
    }

    #[wasm_bindgen_test]
    async fn test_collect_single_key_bound_cursor() {
        const DB_NAME: &str = "TEST_COLLECT_SINGLE_KEY_BOUND_CURSOR";
        const DB_VERSION: u32 = 1;

        register_wasm_log();

        let items = vec![
            swap_item!("uuid1", "RICK", "MORTY", 10, 3, 700),
            swap_item!("uuid2", "MORTY", "KMD", 95000, 1, 721),
            swap_item!("uuid3", "RICK", "XYZ", 7, u32::MAX, 1281), // +
            swap_item!("uuid4", "RICK", "MORTY", 8, 6, 92),        // +
            swap_item!("uuid5", "QRC20", "RICK", 2, 4, 721),
            swap_item!("uuid6", "KMD", "MORTY", 12, 3124, 214), // +
        ];

        let db = IndexedDbBuilder::new(DbIdentifier::for_test(DB_NAME))
            .with_version(DB_VERSION)
            .with_table::<SwapTable>()
            .build()
            .await
            .expect("!IndexedDb::init");
        let transaction = db.transaction().await.expect("!IndexedDb::transaction");
        let table = transaction
            .table::<SwapTable>()
            .await
            .expect("!DbTransaction::open_table");
        fill_table(&table, items).await;

        let mut actual_items = table
            .open_cursor("rel_coin_value")
            .await
            .expect("!DbTable::open_cursor")
            .bound("rel_coin_value", 5u32, u32::MAX)
            .collect()
            .await
            .expect("!DbSingleKeyCursor::collect")
            .into_iter()
            .map(|(_item_id, item)| item)
            .collect::<Vec<_>>();
        actual_items.sort();

        let mut expected_items = vec![
            swap_item!("uuid3", "RICK", "XYZ", 7, u32::MAX, 1281),
            swap_item!("uuid4", "RICK", "MORTY", 8, 6, 92),
            swap_item!("uuid6", "KMD", "MORTY", 12, 3124, 214),
        ];
        expected_items.sort();

        assert_eq!(actual_items, expected_items);
    }

    #[wasm_bindgen_test]
    async fn test_collect_multi_key_cursor() {
        const DB_NAME: &str = "TEST_COLLECT_MULTI_KEY_CURSOR";
        const DB_VERSION: u32 = 1;

        register_wasm_log();

        let items = vec![
            swap_item!("uuid1", "RICK", "MORTY", 12, 1, 700),
            swap_item!("uuid2", "RICK", "KMD", 95000, 6, 721),
            swap_item!("uuid3", "RICK", "MORTY", 12, 5, 720),
            swap_item!("uuid4", "RICK", "MORTY", 12, 3, 721), // +
            swap_item!("uuid5", "QRC20", "MORTY", 51, 221, 182),
            swap_item!("uuid6", "QRC20", "RICK", 12, 6, 121),
            swap_item!("uuid7", "RICK", "QRC20", 12, 6, 721), // +
            swap_item!("uuid8", "FIRO", "DOGE", 12, 8, 721),
            swap_item!("uuid9", "RICK", "DOGE", 115, 1221, 721),
            swap_item!("uuid10", "RICK", "tQTUM", 12, 6, 721), // +
            swap_item!("uuid11", "MORTY", "RICK", 12, 7, 677),
            swap_item!("uuid12", "tBTC", "RICK", 92, 6, 721),
        ];

        let db = IndexedDbBuilder::new(DbIdentifier::for_test(DB_NAME))
            .with_version(DB_VERSION)
            .with_table::<SwapTable>()
            .build()
            .await
            .expect("!IndexedDb::init");
        let transaction = db.transaction().await.expect("!IndexedDb::transaction");
        let table = transaction
            .table::<SwapTable>()
            .await
            .expect("!DbTransaction::open_table");
        fill_table(&table, items).await;

        let mut actual_items = table
            .open_cursor("basecoin_basecoinvalue_startedat_index")
            .await
            .expect("!DbTable::open_cursor")
            .only("base_coin", "RICK")
            .expect("!DbEmptyCursor::only")
            .only("base_coin_value", 12)
            .expect("!DbSingleKeyCursor::only")
            .only("started_at", 721)
            .expect("!DbMultiKeyCursor::only")
            .collect()
            .await
            .expect("!DbMultiKeyCursor::collect")
            .into_iter()
            .map(|(_item_id, item)| item)
            .collect::<Vec<_>>();
        actual_items.sort();

        let mut expected_items = vec![
            swap_item!("uuid4", "RICK", "MORTY", 12, 3, 721),
            swap_item!("uuid7", "RICK", "QRC20", 12, 6, 721),
            swap_item!("uuid10", "RICK", "tQTUM", 12, 6, 721),
        ];
        expected_items.sort();

        assert_eq!(actual_items, expected_items);
    }

    #[wasm_bindgen_test]
    async fn test_collect_multi_key_bound_cursor() {
        const DB_NAME: &str = "TEST_COLLECT_MULTI_KEY_BOUND_CURSOR";
        const DB_VERSION: u32 = 1;

        register_wasm_log();

        let items = vec![
            swap_item!("uuid1", "MORTY", "RICK", 12, 10, 999),
            swap_item!("uuid2", "RICK", "QRC20", 4, 12, 557),
            swap_item!("uuid3", "RICK", "QRC20", 8, 11, 795), // +
            swap_item!("uuid4", "MORTY", "QRC20", 2, 10, 596),
            swap_item!("uuid5", "tQTUM", "MORTY", 1, 8, 709),
            swap_item!("uuid6", "tQTUM", "RICK", 5, 90, 555),
            swap_item!("uuid7", "RICK", "QRC20", 66, 88, 744),
            swap_item!("uuid8", "DOGE", "DOGE", 5, 12, 714),
            swap_item!("uuid9", "RICK", "QRC20", 7, 10, 743), // +
            swap_item!("uuid10", "FIRO", "tQTUM", 7, 11, 777),
            swap_item!("uuid11", "RICK", "MORTY", 91, 11, 1061),
            swap_item!("uuid12", "tBTC", "tQTUM", 4, 771, 745),
            swap_item!("uuid13", "RICK", "QRC20", 3, 11, 759), // +
            swap_item!("uuid14", "DOGE", "tBTC", 4, 6, 895),
            swap_item!("uuid15", "RICK", "QRC20", 723, 19, 558),
            swap_item!("uuid16", "FIRO", "tBTC", 5, 10, 724),
            swap_item!("uuid17", "RICK", "tBTC", 5, 13, 636),
            swap_item!("uuid18", "RICK", "QRC20", 7, 33, 864),
            swap_item!("uuid19", "DOGE", "tBTC", 55, 12, 723),
            swap_item!("uuid20", "RICK", "QRC20", 5, 11, 785), // +
            swap_item!("uuid21", "FIRO", "tBTC", 24, 1, 605),
            swap_item!("uuid22", "RICK", "QRC20", 9, 10, 734),
            swap_item!("uuid23", "tBTC", "tBTC", 7, 99, 834),
            swap_item!("uuid24", "RICK", "QRC20", 8, 12, 849),
            swap_item!("uuid25", "DOGE", "tBTC", 9, 10, 711),
        ];

        let db = IndexedDbBuilder::new(DbIdentifier::for_test(DB_NAME))
            .with_version(DB_VERSION)
            .with_table::<SwapTable>()
            .build()
            .await
            .expect("!IndexedDb::init");
        let transaction = db.transaction().await.expect("!IndexedDb::transaction");
        let table = transaction
            .table::<SwapTable>()
            .await
            .expect("!DbTransaction::open_table");
        fill_table(&table, items).await;

        let actual_items = table
            .open_cursor("all_fields_index")
            .await
            .expect("!DbTable::open_cursor")
            .only("base_coin", "RICK")
            .expect("!DbEmptyCursor::only")
            .only("rel_coin", "QRC20")
            .expect("!DbEmptyCursor::only")
            .bound("base_coin_value", 3u32, 8u32)
            .bound("rel_coin_value", 10u32, 12u32)
            .bound("started_at", 600i32, 800i32)
            .collect()
            .await
            .expect("!DbMultiKeyCursor::collect")
            .into_iter()
            .map(|(_item_id, item)| item)
            .collect::<Vec<_>>();

        // Items are expected to be sorted in the following order.
        let expected_items = vec![
            swap_item!("uuid13", "RICK", "QRC20", 3, 11, 759),
            swap_item!("uuid20", "RICK", "QRC20", 5, 11, 785),
            swap_item!("uuid9", "RICK", "QRC20", 7, 10, 743),
            swap_item!("uuid3", "RICK", "QRC20", 8, 11, 795),
        ];

        assert_eq!(actual_items, expected_items);
    }
}

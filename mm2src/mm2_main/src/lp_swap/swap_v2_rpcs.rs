use super::maker_swap::MakerSavedSwap;
use super::maker_swap_v2::MakerSwapEvent;
use super::my_swaps_storage::{MySwapsError, MySwapsOps, MySwapsStorage};
use super::taker_swap::TakerSavedSwap;
use super::taker_swap_v2::TakerSwapEvent;
use super::{MySwapsFilter, SavedSwap, SavedSwapError, SavedSwapIo, LEGACY_SWAP_TYPE, MAKER_SWAP_V2_TYPE,
            TAKER_SWAP_V2_TYPE};
use common::log::{error, warn};
use common::{calc_total_pages, HttpStatusCode, PagingOptions};
use derive_more::Display;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{MmNumber, MmNumberMultiRepr};
use serde::de::DeserializeOwned;
use std::num::NonZeroUsize;
use uuid::Uuid;

cfg_native!(
    use crate::mm2::database::my_swaps::SELECT_MY_SWAP_V2_FOR_RPC_BY_UUID;
    use common::async_blocking;
    use db_common::sqlite::query_single_row;
    use db_common::sqlite::rusqlite::{Result as SqlResult, Row, Error as SqlError};
    use db_common::sqlite::rusqlite::types::Type as SqlType;
);

cfg_wasm32!(
    use super::SwapsContext;
    use super::maker_swap_v2::MakerSwapDbRepr;
    use super::taker_swap_v2::TakerSwapDbRepr;
    use crate::mm2::lp_swap::swap_wasm_db::{MySwapsFiltersTable, SavedSwapTable};
    use mm2_db::indexed_db::{DbTransactionError, DbTransactionResult, InitDbError};
);

#[cfg(not(target_arch = "wasm32"))]
pub(super) async fn get_swap_type(ctx: &MmArc, uuid: &Uuid) -> MmResult<Option<u8>, SqlError> {
    let ctx = ctx.clone();
    let uuid = uuid.to_string();

    async_blocking(move || {
        const SELECT_SWAP_TYPE_BY_UUID: &str = "SELECT swap_type FROM my_swaps WHERE uuid = :uuid;";
        let maybe_swap_type = query_single_row(
            &ctx.sqlite_connection(),
            SELECT_SWAP_TYPE_BY_UUID,
            &[(":uuid", uuid.as_str())],
            |row| row.get(0),
        )?;
        Ok(maybe_swap_type)
    })
    .await
}

#[cfg(target_arch = "wasm32")]
#[derive(Display)]
pub enum SwapV2DbError {
    DbTransaction(DbTransactionError),
    InitDb(InitDbError),
    Serde(serde_json::Error),
    UnsupportedSwapType(u8),
}

#[cfg(target_arch = "wasm32")]
impl From<DbTransactionError> for SwapV2DbError {
    fn from(e: DbTransactionError) -> Self { SwapV2DbError::DbTransaction(e) }
}

#[cfg(target_arch = "wasm32")]
impl From<InitDbError> for SwapV2DbError {
    fn from(e: InitDbError) -> Self { SwapV2DbError::InitDb(e) }
}

#[cfg(target_arch = "wasm32")]
impl From<serde_json::Error> for SwapV2DbError {
    fn from(e: serde_json::Error) -> Self { SwapV2DbError::Serde(e) }
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn get_swap_type(ctx: &MmArc, uuid: &Uuid) -> MmResult<Option<u8>, SwapV2DbError> {
    use crate::mm2::lp_swap::swap_wasm_db::MySwapsFiltersTable;

    let swaps_ctx = SwapsContext::from_ctx(ctx).unwrap();
    let db = swaps_ctx.swap_db().await?;
    let transaction = db.transaction().await?;
    let table = transaction.table::<MySwapsFiltersTable>().await?;
    let item = match table.get_item_by_unique_index("uuid", uuid).await? {
        Some((_item_id, item)) => item,
        None => return Ok(None),
    };
    Ok(Some(item.swap_type))
}

/// Represents data of the swap used for RPC, omits fields that should be kept in secret
#[derive(Debug, Serialize)]
pub(crate) struct MySwapForRpc<T> {
    my_coin: String,
    other_coin: String,
    uuid: Uuid,
    started_at: i64,
    is_finished: bool,
    events: Vec<T>,
    maker_volume: MmNumberMultiRepr,
    taker_volume: MmNumberMultiRepr,
    premium: MmNumberMultiRepr,
    dex_fee: MmNumberMultiRepr,
    lock_duration: i64,
    maker_coin_confs: i64,
    maker_coin_nota: bool,
    taker_coin_confs: i64,
    taker_coin_nota: bool,
}

impl<T: DeserializeOwned> MySwapForRpc<T> {
    #[cfg(not(target_arch = "wasm32"))]
    fn from_row(row: &Row) -> SqlResult<Self> {
        Ok(Self {
            my_coin: row.get(0)?,
            other_coin: row.get(1)?,
            uuid: row
                .get::<_, String>(2)?
                .parse()
                .map_err(|e| SqlError::FromSqlConversionFailure(2, SqlType::Text, Box::new(e)))?,
            started_at: row.get(3)?,
            is_finished: row.get(4)?,
            events: serde_json::from_str(&row.get::<_, String>(5)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(5, SqlType::Text, Box::new(e)))?,
            maker_volume: MmNumber::from_fraction_string(&row.get::<_, String>(6)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(6, SqlType::Text, Box::new(e)))?
                .into(),
            taker_volume: MmNumber::from_fraction_string(&row.get::<_, String>(7)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(7, SqlType::Text, Box::new(e)))?
                .into(),
            premium: MmNumber::from_fraction_string(&row.get::<_, String>(8)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(8, SqlType::Text, Box::new(e)))?
                .into(),
            dex_fee: MmNumber::from_fraction_string(&row.get::<_, String>(9)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(9, SqlType::Text, Box::new(e)))?
                .into(),
            lock_duration: row.get(10)?,
            maker_coin_confs: row.get(11)?,
            maker_coin_nota: row.get(12)?,
            taker_coin_confs: row.get(13)?,
            taker_coin_nota: row.get(14)?,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) async fn get_maker_swap_data_for_rpc(
    ctx: &MmArc,
    uuid: &Uuid,
) -> MmResult<Option<MySwapForRpc<MakerSwapEvent>>, SqlError> {
    get_swap_data_for_rpc_impl(ctx, uuid).await
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) async fn get_taker_swap_data_for_rpc(
    ctx: &MmArc,
    uuid: &Uuid,
) -> MmResult<Option<MySwapForRpc<TakerSwapEvent>>, SqlError> {
    get_swap_data_for_rpc_impl(ctx, uuid).await
}

#[cfg(not(target_arch = "wasm32"))]
async fn get_swap_data_for_rpc_impl<T: DeserializeOwned + Send + 'static>(
    ctx: &MmArc,
    uuid: &Uuid,
) -> MmResult<Option<MySwapForRpc<T>>, SqlError> {
    let ctx = ctx.clone();
    let uuid = uuid.to_string();

    async_blocking(move || {
        let swap_data = query_single_row(
            &ctx.sqlite_connection(),
            SELECT_MY_SWAP_V2_FOR_RPC_BY_UUID,
            &[(":uuid", uuid.as_str())],
            MySwapForRpc::from_row,
        )?;
        Ok(swap_data)
    })
    .await
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn get_maker_swap_data_for_rpc(
    ctx: &MmArc,
    uuid: &Uuid,
) -> MmResult<Option<MySwapForRpc<MakerSwapEvent>>, SwapV2DbError> {
    let swaps_ctx = SwapsContext::from_ctx(ctx).unwrap();
    let db = swaps_ctx.swap_db().await?;
    let transaction = db.transaction().await?;
    let table = transaction.table::<SavedSwapTable>().await?;
    let item = match table.get_item_by_unique_index("uuid", uuid).await? {
        Some((_item_id, item)) => item,
        None => return Ok(None),
    };

    let filters_table = transaction.table::<MySwapsFiltersTable>().await?;
    let filter_item = match filters_table.get_item_by_unique_index("uuid", uuid).await? {
        Some((_item_id, item)) => item,
        None => return Ok(None),
    };

    let json_repr: MakerSwapDbRepr = serde_json::from_value(item.saved_swap)?;
    Ok(Some(MySwapForRpc {
        my_coin: json_repr.maker_coin,
        other_coin: json_repr.taker_coin,
        uuid: json_repr.uuid,
        started_at: json_repr.started_at as i64,
        is_finished: filter_item.is_finished.as_bool(),
        events: json_repr.events,
        maker_volume: json_repr.maker_volume.into(),
        taker_volume: json_repr.taker_volume.into(),
        premium: json_repr.taker_premium.into(),
        dex_fee: json_repr.dex_fee_amount.into(),
        lock_duration: json_repr.lock_duration as i64,
        maker_coin_confs: json_repr.conf_settings.maker_coin_confs as i64,
        maker_coin_nota: json_repr.conf_settings.maker_coin_nota,
        taker_coin_confs: json_repr.conf_settings.taker_coin_confs as i64,
        taker_coin_nota: json_repr.conf_settings.taker_coin_nota,
    }))
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn get_taker_swap_data_for_rpc(
    ctx: &MmArc,
    uuid: &Uuid,
) -> MmResult<Option<MySwapForRpc<TakerSwapEvent>>, SwapV2DbError> {
    let swaps_ctx = SwapsContext::from_ctx(ctx).unwrap();
    let db = swaps_ctx.swap_db().await?;
    let transaction = db.transaction().await?;
    let table = transaction.table::<SavedSwapTable>().await?;
    let item = match table.get_item_by_unique_index("uuid", uuid).await? {
        Some((_item_id, item)) => item,
        None => return Ok(None),
    };

    let filters_table = transaction.table::<MySwapsFiltersTable>().await?;
    let filter_item = match filters_table.get_item_by_unique_index("uuid", uuid).await? {
        Some((_item_id, item)) => item,
        None => return Ok(None),
    };

    let json_repr: TakerSwapDbRepr = serde_json::from_value(item.saved_swap)?;
    Ok(Some(MySwapForRpc {
        my_coin: json_repr.taker_coin,
        other_coin: json_repr.maker_coin,
        uuid: json_repr.uuid,
        started_at: json_repr.started_at as i64,
        is_finished: filter_item.is_finished.as_bool(),
        events: json_repr.events,
        maker_volume: json_repr.maker_volume.into(),
        taker_volume: json_repr.taker_volume.into(),
        premium: json_repr.taker_premium.into(),
        dex_fee: json_repr.dex_fee.into(),
        lock_duration: json_repr.lock_duration as i64,
        maker_coin_confs: json_repr.conf_settings.maker_coin_confs as i64,
        maker_coin_nota: json_repr.conf_settings.maker_coin_nota,
        taker_coin_confs: json_repr.conf_settings.taker_coin_confs as i64,
        taker_coin_nota: json_repr.conf_settings.taker_coin_nota,
    }))
}

#[derive(Serialize)]
#[serde(tag = "swap_type", content = "swap_data")]
pub(crate) enum SwapRpcData {
    MakerV1(MakerSavedSwap),
    TakerV1(TakerSavedSwap),
    MakerV2(MySwapForRpc<MakerSwapEvent>),
    TakerV2(MySwapForRpc<TakerSwapEvent>),
}

#[derive(Deserialize)]
pub(crate) struct MySwapStatusRequest {
    uuid: Uuid,
}

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub(crate) enum MySwapStatusError {
    NoSwapWithUuid(Uuid),
    UnsupportedSwapType(u8),
    DbError(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqlError> for MySwapStatusError {
    fn from(e: SqlError) -> Self { MySwapStatusError::DbError(e.to_string()) }
}

#[cfg(target_arch = "wasm32")]
impl From<SwapV2DbError> for MySwapStatusError {
    fn from(e: SwapV2DbError) -> Self { MySwapStatusError::DbError(e.to_string()) }
}

impl From<SavedSwapError> for MySwapStatusError {
    fn from(e: SavedSwapError) -> Self { MySwapStatusError::DbError(e.to_string()) }
}

impl HttpStatusCode for MySwapStatusError {
    fn status_code(&self) -> StatusCode {
        match self {
            MySwapStatusError::NoSwapWithUuid(_) => StatusCode::BAD_REQUEST,
            MySwapStatusError::DbError(_) | MySwapStatusError::UnsupportedSwapType(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

pub(crate) async fn my_swap_status_rpc(
    ctx: MmArc,
    req: MySwapStatusRequest,
) -> MmResult<SwapRpcData, MySwapStatusError> {
    let swap_type = get_swap_type(&ctx, &req.uuid)
        .await?
        .or_mm_err(|| MySwapStatusError::NoSwapWithUuid(req.uuid))?;
    match swap_type {
        LEGACY_SWAP_TYPE => {
            let saved_swap = SavedSwap::load_my_swap_from_db(&ctx, req.uuid)
                .await?
                .or_mm_err(|| MySwapStatusError::NoSwapWithUuid(req.uuid))?;
            match saved_swap {
                SavedSwap::Maker(m) => Ok(SwapRpcData::MakerV1(m)),
                SavedSwap::Taker(t) => Ok(SwapRpcData::TakerV1(t)),
            }
        },
        MAKER_SWAP_V2_TYPE => {
            let data = get_maker_swap_data_for_rpc(&ctx, &req.uuid)
                .await?
                .or_mm_err(|| MySwapStatusError::NoSwapWithUuid(req.uuid))?;
            Ok(SwapRpcData::MakerV2(data))
        },
        TAKER_SWAP_V2_TYPE => {
            let data = get_taker_swap_data_for_rpc(&ctx, &req.uuid)
                .await?
                .or_mm_err(|| MySwapStatusError::NoSwapWithUuid(req.uuid))?;
            Ok(SwapRpcData::TakerV2(data))
        },
        unsupported => MmError::err(MySwapStatusError::UnsupportedSwapType(unsupported)),
    }
}

#[derive(Deserialize)]
pub(crate) struct MyRecentSwapsRequest {
    #[serde(flatten)]
    pub paging_options: PagingOptions,
    #[serde(flatten)]
    pub filter: MySwapsFilter,
}

#[derive(Serialize)]
pub(crate) struct MyRecentSwapsResponse {
    swaps: Vec<SwapRpcData>,
    from_uuid: Option<Uuid>,
    skipped: usize,
    limit: usize,
    total: usize,
    page_number: NonZeroUsize,
    total_pages: usize,
    found_records: usize,
}

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub(crate) enum MyRecentSwapsErr {
    FromUuidSwapNotFound(Uuid),
    InvalidTimeStampRange,
    DbError(String),
}

impl From<MySwapsError> for MyRecentSwapsErr {
    fn from(e: MySwapsError) -> Self {
        match e {
            MySwapsError::InvalidTimestampRange => MyRecentSwapsErr::InvalidTimeStampRange,
            MySwapsError::FromUuidNotFound(uuid) => MyRecentSwapsErr::FromUuidSwapNotFound(uuid),
            other => MyRecentSwapsErr::DbError(other.to_string()),
        }
    }
}

impl HttpStatusCode for MyRecentSwapsErr {
    fn status_code(&self) -> StatusCode {
        match self {
            MyRecentSwapsErr::FromUuidSwapNotFound(_) | MyRecentSwapsErr::InvalidTimeStampRange => {
                StatusCode::BAD_REQUEST
            },
            MyRecentSwapsErr::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub(crate) async fn my_recent_swaps_rpc(
    ctx: MmArc,
    req: MyRecentSwapsRequest,
) -> MmResult<MyRecentSwapsResponse, MyRecentSwapsErr> {
    let db_result = MySwapsStorage::new(ctx.clone())
        .my_recent_swaps_with_filters(&req.filter, Some(&req.paging_options))
        .await?;
    let mut swaps = Vec::with_capacity(db_result.uuids_and_types.len());
    for (uuid, swap_type) in db_result.uuids_and_types.iter() {
        match *swap_type {
            LEGACY_SWAP_TYPE => match SavedSwap::load_my_swap_from_db(&ctx, *uuid).await {
                Ok(Some(SavedSwap::Maker(m))) => {
                    swaps.push(SwapRpcData::MakerV1(m));
                },
                Ok(Some(SavedSwap::Taker(t))) => {
                    swaps.push(SwapRpcData::TakerV1(t));
                },
                Ok(None) => warn!("No such swap with the uuid '{}'", uuid),
                Err(e) => error!("Error loading a swap with the uuid '{}': {}", uuid, e),
            },
            MAKER_SWAP_V2_TYPE => match get_maker_swap_data_for_rpc(&ctx, uuid).await {
                Ok(Some(m)) => {
                    swaps.push(SwapRpcData::MakerV2(m));
                },
                Ok(None) => warn!("No such swap with the uuid '{}'", uuid),
                Err(e) => error!("Error loading a swap with the uuid '{}': {}", uuid, e),
            },
            TAKER_SWAP_V2_TYPE => match get_taker_swap_data_for_rpc(&ctx, uuid).await {
                Ok(Some(t)) => {
                    swaps.push(SwapRpcData::TakerV2(t));
                },
                Ok(None) => warn!("No such swap with the uuid '{}'", uuid),
                Err(e) => error!("Error loading a swap with the uuid '{}': {}", uuid, e),
            },
            unknown_type => error!("Swap with the uuid '{}' has unknown type {}", uuid, unknown_type),
        }
    }

    Ok(MyRecentSwapsResponse {
        swaps,
        from_uuid: req.paging_options.from_uuid,
        skipped: db_result.skipped,
        limit: req.paging_options.limit,
        total: db_result.total_count,
        page_number: req.paging_options.page_number,
        total_pages: calc_total_pages(db_result.total_count, req.paging_options.limit),
        found_records: db_result.uuids_and_types.len(),
    })
}

use super::{CollectCursorAction, CollectItemAction, CursorBoundValue, CursorError, CursorOps, CursorResult, DbFilter};
use async_trait::async_trait;
use common::{log::warn, stringify_js_error};
use mm2_err_handle::prelude::*;
use serde_json::Value as Json;
use wasm_bindgen::prelude::*;
use web_sys::{IdbIndex, IdbKeyRange};

/// The representation of a range that includes records
/// whose value of the [`IdbSingleBoundCursor::field_name`] field is lower than [`IdbSingleBoundCursor::lower_bound_value`]
/// and greater than [`IdbSingleBoundCursor::upper_bound_value`].
/// https://developer.mozilla.org/en-US/docs/Web/API/IDBKeyRange/bound
pub struct IdbSingleKeyBoundCursor {
    db_index: IdbIndex,
    #[allow(dead_code)]
    field_name: String,
    lower_bound: CursorBoundValue,
    upper_bound: CursorBoundValue,
    /// An additional predicate that may be used to filter records.
    collect_filter: Option<DbFilter>,
}

impl IdbSingleKeyBoundCursor {
    pub(super) fn new(
        db_index: IdbIndex,
        field_name: String,
        lower_bound: CursorBoundValue,
        upper_bound: CursorBoundValue,
        collect_filter: Option<DbFilter>,
    ) -> CursorResult<IdbSingleKeyBoundCursor> {
        Self::check_bounds(&lower_bound, &upper_bound)?;
        Ok(IdbSingleKeyBoundCursor {
            db_index,
            field_name,
            lower_bound,
            upper_bound,
            collect_filter,
        })
    }
}

impl IdbSingleKeyBoundCursor {
    fn check_bounds(lower_bound: &CursorBoundValue, upper_bound: &CursorBoundValue) -> CursorResult<()> {
        if lower_bound > upper_bound {
            let description = format!(
                "Incorrect usage of 'IdbSingleKeyBoundCursor': lower_bound '{:?}' is expected to be less or equal to upper_bound '{:?}'",
                lower_bound,
                upper_bound
            );
            return MmError::err(CursorError::InvalidKeyRange { description });
        }
        if lower_bound == upper_bound {
            warn!("lower_bound '{:?}' equals to upper_bound '{:?}'. Consider using 'IdbObjectStoreImpl::get_items' instead", lower_bound, upper_bound);
        }
        Ok(())
    }
}

#[async_trait(?Send)]
impl CursorOps for IdbSingleKeyBoundCursor {
    fn db_index(&self) -> &IdbIndex { &self.db_index }

    fn key_range(&self) -> CursorResult<Option<IdbKeyRange>> {
        let key_range = IdbKeyRange::bound(&self.lower_bound.to_js_value()?, &self.upper_bound.to_js_value()?)
            .map_to_mm(|e| CursorError::InvalidKeyRange {
                description: stringify_js_error(&e),
            })?;
        Ok(Some(key_range))
    }

    fn on_collect_iter(
        &mut self,
        _key: JsValue,
        value: &Json,
    ) -> CursorResult<(CollectItemAction, CollectCursorAction)> {
        if let Some(ref mut filter) = self.collect_filter {
            return Ok(filter(value));
        }
        Ok((CollectItemAction::Include, CollectCursorAction::Continue))
    }
}

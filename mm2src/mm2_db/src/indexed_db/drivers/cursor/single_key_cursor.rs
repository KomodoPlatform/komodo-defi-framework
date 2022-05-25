use super::{CollectCursorAction, CollectItemAction, CursorError, CursorOps, CursorResult, DbFilter};
use async_trait::async_trait;
use common::{log::warn, stringify_js_error};
use mm2_err_handle::prelude::*;
use serde::Serialize;
use serde_json::Value as Json;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::prelude::*;
use web_sys::{IdbIndex, IdbKeyRange};

/// The representation of a range that includes records
/// whose value of the [`IdbSingleKeyCursor::field_name`] field equals to the [`IdbSingleKeyCursor::field_value`] value.
/// https://developer.mozilla.org/en-US/docs/Web/API/IDBKeyRange/only
pub struct IdbSingleKeyCursor {
    db_index: IdbIndex,
    #[allow(dead_code)]
    field_name: String,
    field_value: Json,
    /// An additional predicate that may be used to filter records.
    collect_filter: Option<DbFilter>,
}

impl IdbSingleKeyCursor {
    pub(super) fn new(
        db_index: IdbIndex,
        field_name: String,
        field_value: Json,
        filter: Option<DbFilter>,
    ) -> IdbSingleKeyCursor {
        if filter.is_none() {
            warn!("Consider using 'IdbObjectStoreImpl::get_items' instead of 'IdbSingleKeyCursor'");
        }
        IdbSingleKeyCursor {
            db_index,
            field_name,
            field_value,
            collect_filter: None,
        }
    }
}

#[async_trait(?Send)]
impl CursorOps for IdbSingleKeyCursor {
    fn db_index(&self) -> &IdbIndex { &self.db_index }

    fn key_range(&self) -> CursorResult<Option<IdbKeyRange>> {
        let serializer = Serializer::json_compatible();
        let js_value =
            self.field_value
                .serialize(&serializer)
                .map_to_mm(|e| CursorError::ErrorSerializingIndexFieldValue {
                    field: self.field_name.clone(),
                    value: format!("{:?}", self.field_value),
                    description: e.to_string(),
                })?;

        let key_range = IdbKeyRange::only(&js_value).map_to_mm(|e| CursorError::InvalidKeyRange {
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

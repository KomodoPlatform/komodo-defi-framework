use super::{CollectCursorAction, CollectItemAction, CursorError, CursorOps, CursorResult, DbFilter};
use async_trait::async_trait;
use common::stringify_js_error;
use js_sys::Array;
use mm2_err_handle::prelude::*;
use serde::Serialize;
use serde_json::Value as Json;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::prelude::*;
use web_sys::{IdbIndex, IdbKeyRange};

/// The representation of a range that includes records
/// whose fields have only the specified [`IdbSingleCursor::only_values`] values.
/// https://developer.mozilla.org/en-US/docs/Web/API/IDBKeyRange/only
pub struct IdbMultiKeyCursor {
    db_index: IdbIndex,
    only_values: Vec<(String, Json)>,
    /// An additional predicate that can be used to filter records.
    collect_filter: Option<DbFilter>,
}

impl IdbMultiKeyCursor {
    pub(super) fn new(
        db_index: IdbIndex,
        only_values: Vec<(String, Json)>,
        collect_filter: Option<DbFilter>,
    ) -> CursorResult<IdbMultiKeyCursor> {
        Self::check_only_values(&only_values)?;
        Ok(IdbMultiKeyCursor {
            db_index,
            only_values,
            collect_filter,
        })
    }

    fn check_only_values(only_values: &Vec<(String, Json)>) -> CursorResult<()> {
        if only_values.len() < 2 {
            let description = format!(
                "Incorrect usage of 'IdbMultiKeyCursor': expected more than one cursor bound, found '{}'",
                only_values.len(),
            );
            return MmError::err(CursorError::IncorrectUsage { description });
        }
        Ok(())
    }
}

#[async_trait(?Send)]
impl CursorOps for IdbMultiKeyCursor {
    fn db_index(&self) -> &IdbIndex { &self.db_index }

    fn key_range(&self) -> CursorResult<Option<IdbKeyRange>> {
        let only = Array::new();

        let serializer = Serializer::json_compatible();
        for (field, value) in self.only_values.iter() {
            let js_value =
                value
                    .serialize(&serializer)
                    .map_to_mm(|e| CursorError::ErrorSerializingIndexFieldValue {
                        field: field.to_owned(),
                        value: format!("{:?}", value),
                        description: e.to_string(),
                    })?;
            only.push(&js_value);
        }

        let key_range = IdbKeyRange::only(&only).map_to_mm(|e| CursorError::InvalidKeyRange {
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

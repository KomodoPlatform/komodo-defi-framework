use common::{serialize_to_js, stringify_js_error};
use mm2_err_handle::prelude::*;
use serde_json::Value as Json;
use wasm_bindgen::prelude::*;
use web_sys::IdbKeyRange;

use super::{CursorAction, CursorDriverImpl, CursorError, CursorItemAction, CursorResult};

/// The representation of a range that includes records
/// whose value of the [`IdbSingleKeyCursor::field_name`] field equals to the [`IdbSingleKeyCursor::field_value`] value.
/// https://developer.mozilla.org/en-US/docs/Web/API/IDBKeyRange/only
pub struct IdbSingleKeyCursor {
    #[allow(dead_code)]
    field_name: String,
    field_value: Json,
}

impl IdbSingleKeyCursor {
    pub(super) fn new(field_name: String, field_value: Json) -> IdbSingleKeyCursor {
        IdbSingleKeyCursor {
            field_name,
            field_value,
        }
    }
}

impl CursorDriverImpl for IdbSingleKeyCursor {
    fn key_range(&self) -> CursorResult<Option<IdbKeyRange>> {
        let js_value =
            serialize_to_js(&self.field_value).map_to_mm(|e| CursorError::ErrorSerializingIndexFieldValue {
                field: self.field_name.clone(),
                value: format!("{:?}", self.field_value),
                description: e.to_string(),
            })?;

        let key_range = IdbKeyRange::only(&js_value).map_to_mm(|e| CursorError::InvalidKeyRange {
            description: stringify_js_error(&e),
        })?;
        Ok(Some(key_range))
    }

    fn on_iteration(&mut self, _key: JsValue) -> CursorResult<(CursorItemAction, CursorAction)> {
        Ok((CursorItemAction::Include, CursorAction::Continue))
    }
}

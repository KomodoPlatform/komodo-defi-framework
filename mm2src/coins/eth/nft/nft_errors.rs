use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Display, Serialize, SerializeErrorType, Deserialize)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetNftInfoError {
    // todo
}

impl HttpStatusCode for GetNftInfoError {
    fn status_code(&self) -> StatusCode { todo!() }
}

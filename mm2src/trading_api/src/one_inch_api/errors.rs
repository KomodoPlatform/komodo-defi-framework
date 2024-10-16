use common::StatusCode;
use derive_more::Display;
use enum_derives::EnumFromStringify;
use ethereum_types::U256;
use mm2_net::transport::SlurpError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct GeneralApiError {
    pub error: String,
    pub description: Option<String>,
    pub status_code: u16,
}

impl std::fmt::Display for GeneralApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "error description: {}",
            self.description.as_ref().unwrap_or(&"".to_owned())
        )
    }
}

#[derive(Debug, Serialize)]
pub struct AllowanceNotEnoughError {
    pub error: String,
    pub description: Option<String>,
    pub status_code: u16,
    /// Amount to approve for the API contract
    pub amount: U256,
    /// Existing allowance for the API contract
    pub allowance: U256,
}

impl std::fmt::Display for AllowanceNotEnoughError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "error description: {}",
            self.description.as_ref().unwrap_or(&"".to_owned())
        )
    }
}

#[derive(Debug, Display, Serialize, EnumFromStringify)]
pub enum ApiClientError {
    #[from_stringify("url::ParseError")]
    InvalidParam(String),
    #[display(fmt = "Parameter {param} out of bounds, value: {value}, min: {min} max: {max}")]
    OutOfBounds { param: String, value: String, min: String, max: String },
    HttpClientError(SlurpError),
    ParseBodyError(String),
    GeneralApiError(GeneralApiError),
    AllowanceNotEnough(AllowanceNotEnoughError),
}

// API error meta 'type' field known values
const META_TYPE_ALLOWANCE: &str = "allowance";
const META_TYPE_AMOUNT: &str = "amount";

#[derive(Debug, Deserialize)]
pub(crate) struct Error400 {
    pub error: String,
    pub description: Option<String>,
    #[serde(rename = "statusCode")]
    pub status_code: u16,
    pub meta: Option<Vec<Meta>>,
    #[allow(dead_code)]
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Meta {
    #[serde(rename = "type")]
    pub meta_type: String,
    #[serde(rename = "value")]
    pub meta_value: String,
}

#[derive(Debug)]
pub(crate) struct OtherError {
    pub error: String,
    pub status_code: u16,
}

#[derive(Debug)]
pub(crate) enum NativeError {
    Error400(Error400),
    OtherError(OtherError),
    ParseError(String),
}

impl NativeError {
    pub(crate) fn new(status_code: StatusCode, body: Value) -> Self {
        if status_code == StatusCode::BAD_REQUEST {
            match serde_json::from_value(body) {
                Ok(err) => Self::Error400(err),
                Err(err) => Self::ParseError(err.to_string()),
            }
        } else {
            Self::OtherError(OtherError {
                error: body["error"].as_str().unwrap_or_default().to_owned(),
                status_code: status_code.into(),
            })
        }
    }
}

impl ApiClientError {
    /// Convert from native API errors to lib errors
    /// Look for known API errors. If none found return as general API error
    pub(crate) fn from_native_error(api_error: NativeError) -> ApiClientError {
        match api_error {
            NativeError::Error400(error_400) => {
                if let Some(meta) = error_400.meta {
                    // Try if it's "Not enough allowance" error 'meta' data:
                    if let Some(meta_allowance) = meta.iter().find(|m| m.meta_type == META_TYPE_ALLOWANCE) {
                        // try find 'amount' value
                        let amount = if let Some(meta_amount) = meta.iter().find(|m| m.meta_type == META_TYPE_AMOUNT) {
                            U256::from_dec_str(&meta_amount.meta_value).unwrap_or_default()
                        } else {
                            Default::default()
                        };
                        let allowance = U256::from_dec_str(&meta_allowance.meta_value).unwrap_or_default();
                        return ApiClientError::AllowanceNotEnough(AllowanceNotEnoughError {
                            error: error_400.error,
                            status_code: error_400.status_code,
                            description: error_400.description,
                            amount,
                            allowance,
                        });
                    }
                }
                ApiClientError::GeneralApiError(GeneralApiError {
                    error: error_400.error,
                    status_code: error_400.status_code,
                    description: error_400.description,
                })
            },
            NativeError::OtherError(other_error) => ApiClientError::GeneralApiError(GeneralApiError {
                error: other_error.error,
                status_code: other_error.status_code,
                description: None,
            }),
            NativeError::ParseError(err_str) => ApiClientError::ParseBodyError(err_str),
        }
    }
}

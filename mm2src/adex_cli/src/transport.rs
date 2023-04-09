use async_trait::async_trait;
use http::{HeaderMap, StatusCode};
use log::{error, warn};
use mm2_net::native_http::slurp_post_json;
use serde::{Deserialize, Serialize};
use std::fmt::Display;

#[async_trait]
pub trait Transport {
    async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<Result<OkT, ErrT>, ()>
    where
        ReqT: Serialize + Send,
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a> + Display;
}

pub struct SlurpTransport {
    pub uri: String,
}

#[async_trait]
impl Transport for SlurpTransport {
    async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<Result<OkT, ErrT>, ()>
    where
        ReqT: Serialize + Send,
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a> + Display,
    {
        let data = serde_json::to_string(&req).expect("Failed to serialize enable request");
        match slurp_post_json(&self.uri, data).await {
            Err(error) => {
                error!("Failed to send json: {error}");
                return Err(());
            },
            Ok(resp) => resp.process::<OkT, ErrT>(),
        }
    }
}

pub(crate) trait Response {
    fn process<OkT, ErrT>(self) -> Result<Result<OkT, ErrT>, ()>
    where
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a> + Display;
}

impl Response for (StatusCode, HeaderMap, Vec<u8>) {
    fn process<OkT, ErrT>(self) -> Result<Result<OkT, ErrT>, ()>
    where
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a> + Display,
    {
        let (status, _headers, data) = self;

        match status {
            StatusCode::OK => match serde_json::from_slice::<OkT>(&data) {
                Ok(resp_data) => Ok(Ok(resp_data)),
                Err(error) => {
                    error!("Failed to deserialize adex_response from data: {data:?}, error: {error}");
                    Err(())
                },
            },
            StatusCode::INTERNAL_SERVER_ERROR => match serde_json::from_slice::<ErrT>(&data) {
                Ok(resp_data) => Ok(Err(resp_data)),
                Err(error) => {
                    error!("Failed to deserialize adex_response from data: {data:?}, error: {error}");
                    Err(())
                },
            },
            _ => {
                warn!("Bad http status: {status}, data: {data:?}");
                Err(())
            },
        }
    }
}

use http::{HeaderMap, StatusCode};
use log::{error, info, warn};
use serde::Deserialize;
use std::fmt::Display;

pub(crate) trait Response {
    fn process<T, E, OkF, ErrF>(self, if_ok: OkF, if_err: Option<ErrF>)
    where
        T: for<'a> Deserialize<'a>,
        OkF: Fn(T) -> Result<(), ()>,
        E: for<'a> Deserialize<'a> + Display,
        ErrF: Fn(E) -> Result<(), ()>;
}

impl Response for (StatusCode, HeaderMap, Vec<u8>) {
    fn process<T, E, OkF, ErrF>(self, if_ok: OkF, if_err: Option<ErrF>)
    where
        T: for<'a> Deserialize<'a>,
        OkF: Fn(T) -> Result<(), ()>,
        E: for<'a> Deserialize<'a> + Display,
        ErrF: Fn(E) -> Result<(), ()>,
    {
        let (status, _headers, data) = self;

        match status {
            StatusCode::OK => match serde_json::from_slice::<T>(&data) {
                Ok(resp_data) => {
                    let _ = if_ok(resp_data);
                },
                Err(error) => error!("Failed to deserialize adex_response from data: {data:?}, error: {error}"),
            },
            StatusCode::INTERNAL_SERVER_ERROR => match serde_json::from_slice::<E>(&data) {
                Ok(resp_data) => match if_err {
                    Some(if_err) => {
                        let _ = if_err(resp_data);
                    },
                    None => info!("{}", resp_data),
                },
                Err(error) => error!("Failed to deserialize adex_response from data: {data:?}, error: {error}"),
            },
            _ => {
                warn!("Bad http status: {status}, data: {data:?}");
            },
        };
    }
}

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait Transport {
    async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<OkT, Result<ErrT, ()>>
    where
        ReqT: Serialize,
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a>;
}

pub struct SlurpTransport {}

impl Transport for SlurpTransport {
    async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<OkT, Result<ErrT, ()>>
    where
        ReqT: Serialize,
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a>,
    {
        Ok(33)
    }
}

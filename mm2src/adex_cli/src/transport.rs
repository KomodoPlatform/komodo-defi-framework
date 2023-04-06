use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait Transport {
    async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<OkT, Result<ErrT, ()>>
    where
        ReqT: Serialize + Send,
        OkT: for<'a> Deserialize<'a> + Default,
        ErrT: for<'a> Deserialize<'a>,
        Self: Sized + Sync + Send + 'static;
}

pub struct SlurpTransport {}

#[async_trait]
impl Transport for SlurpTransport {
    async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<OkT, Result<ErrT, ()>>
    where
        ReqT: Serialize + Send,
        OkT: for<'a> Deserialize<'a> + Default,
        ErrT: for<'a> Deserialize<'a>,
        Self: Sized + Sync + Send + 'static,
    {
        Ok(OkT::default())
    }
}

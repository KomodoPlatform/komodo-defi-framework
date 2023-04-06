use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
trait Transport {
    async fn send<ReqT, OkT, ErrT>(req: ReqT) -> Result<OkT, ErrT>
    where
        ReqT: Serialize,
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a>;
}

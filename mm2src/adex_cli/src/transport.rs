use serde::{Deserialize, Serialize};

trait Transport {
    fn send<ReqT, OkT, ErrT>(req: ReqT) -> Result<OkT, ErrT>
    where
        ReqT: Serialize,
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a>;
}

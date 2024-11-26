use crate::rpc::lp_commands::pubkey::GetPublicKeyError;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::mm_error::MmError;
use rpc::v1::types::H160 as H160Json;
use serde_json::Value as Json;

pub type GetSharedDbIdResult<T> = Result<T, MmError<GetSharedDbIdError>>;
pub type GetSharedDbIdError = GetPublicKeyError;

#[derive(Serialize)]
pub struct GetSharedDbIdResponse {
    shared_db_id: H160Json,
}

pub async fn get_shared_db_id(ctx: MmArc, _req: Json) -> GetSharedDbIdResult<GetSharedDbIdResponse> {
    let shared_db_id = ctx.shared_db_id().to_owned().into();
    Ok(GetSharedDbIdResponse { shared_db_id })
}

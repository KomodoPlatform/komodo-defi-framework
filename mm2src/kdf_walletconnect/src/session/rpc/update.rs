use crate::storage::WalletConnectStorageOps;
use crate::{error::WalletConnectError, WalletConnectCtx};

use common::log::info;
use mm2_err_handle::prelude::*;
use relay_rpc::domain::{MessageId, Topic};
use relay_rpc::rpc::params::{session_update::SessionUpdateRequest, ResponseParamsSuccess};

// TODO: Handle properly when multi chain is supported.
// Hanlding for only cosmos support.
pub(crate) async fn reply_session_update_request(
    ctx: &WalletConnectCtx,
    topic: &Topic,
    message_id: &MessageId,
    update: SessionUpdateRequest,
) -> MmResult<(), WalletConnectError> {
    {
        if let Some(mut session) = ctx.session.get_session_mut(topic).await {
            session.namespaces = update.namespaces.0;
            //  Update storage session.
            ctx.storage
                .update_session(&session)
                .await
                .mm_err(|err| WalletConnectError::StorageError(err.to_string()))?;

            info!("Updated extended, info: {:?}", session);
        };
    }

    let param = ResponseParamsSuccess::SessionUpdate(true);
    ctx.publish_response_ok(topic, param, message_id).await?;

    Ok(())
}

use crate::storage::WalletConnectStorageOps;
use crate::{error::WalletConnectError, WalletConnectCtxImpl};

use common::log::info;
use mm2_err_handle::prelude::*;
use relay_rpc::domain::{MessageId, Topic};
use relay_rpc::rpc::params::{session_update::SessionUpdateRequest, ResponseParamsSuccess};

// TODO: Handle properly when multi chain is supported.
// Hanlding for only cosmos support.
pub(crate) async fn reply_session_update_request(
    ctx: &WalletConnectCtxImpl,
    topic: &Topic,
    message_id: &MessageId,
    update: SessionUpdateRequest,
) -> MmResult<(), WalletConnectError> {
    {
        if let Some(mut session) = ctx.session_manager.get_session_mut(topic) {
            update
                .namespaces
                .caip2_validate()
                .map_to_mm(|err| WalletConnectError::InternalError(err.to_string()))?;
            //TODO: session.namespaces.supported(update.namespaces.0)
            session.namespaces = update.namespaces.0;
            //  Update storage session.
            ctx.session_manager
                .storage()
                .update_session(&session)
                .await
                .mm_err(|err| WalletConnectError::StorageError(err.to_string()))?;

            info!("Updated extended, info: {:?}", session.topic);
        };
    }

    let param = ResponseParamsSuccess::SessionUpdate(true);
    ctx.publish_response_ok(topic, param, message_id).await?;

    Ok(())
}

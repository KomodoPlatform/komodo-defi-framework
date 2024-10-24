use crate::{error::{WalletConnectError, USER_REQUESTED},
            storage::WalletConnectStorageOps,
            WalletConnectCtx};

use common::log::debug;
use mm2_err_handle::prelude::{MapMmError, MmResult};
use relay_rpc::domain::{MessageId, Topic};
use relay_rpc::rpc::params::{session_delete::SessionDeleteRequest, RequestParams, ResponseParamsSuccess};

pub(crate) async fn reply_session_delete_request(
    ctx: &WalletConnectCtx,
    topic: &Topic,
    message_id: &MessageId,
    _delete_params: SessionDeleteRequest,
) -> MmResult<(), WalletConnectError> {
    let param = ResponseParamsSuccess::SessionDelete(true);
    ctx.publish_response_ok(topic, param, message_id).await?;

    session_delete_cleanup(ctx, topic).await
}

pub async fn send_session_delete_request(
    ctx: &WalletConnectCtx,
    session_topic: &Topic,
) -> MmResult<(), WalletConnectError> {
    let delete_request = SessionDeleteRequest {
        code: USER_REQUESTED,
        message: "User Disconnected".to_owned(),
    };
    let param = RequestParams::SessionDelete(delete_request);

    ctx.publish_request(session_topic, param).await?;

    session_delete_cleanup(ctx, session_topic).await
}

async fn session_delete_cleanup(ctx: &WalletConnectCtx, topic: &Topic) -> MmResult<(), WalletConnectError> {
    {
        ctx.client.unsubscribe(topic.clone()).await?;
    };

    if let Some(session) = ctx.session.delete_session(topic).await {
        debug!(
            "No active sessions left for pairing {}, disconnecting",
            session.pairing_topic
        );
        //Attempt to unsubscribe from topic
        ctx.client.unsubscribe(session.pairing_topic.clone()).await?;
        // Attempt to disconnect the pairing
        ctx.pairing.delete(session.pairing_topic.as_ref()).await;
        // Remove subscriptions
        let mut subs = ctx.subscriptions.lock().await;
        subs.retain(|s| s != &session.topic);
        subs.retain(|s| s != &session.pairing_topic);

        // delete session from storage as well.
        ctx.storage
            .delete_session(topic)
            .await
            .mm_err(|err| WalletConnectError::StorageError(err.to_string()))?;
    };

    Ok(())
}

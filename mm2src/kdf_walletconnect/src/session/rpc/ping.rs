use crate::{error::WalletConnectCtxError, WalletConnectCtx};

use mm2_err_handle::prelude::MmResult;
use relay_rpc::{domain::{MessageId, Topic},
                rpc::params::{RequestParams, ResponseParamsSuccess}};

pub(crate) async fn reply_session_ping_request(
    ctx: &WalletConnectCtx,
    topic: &Topic,
    message_id: &MessageId,
) -> MmResult<(), WalletConnectCtxError> {
    let param = ResponseParamsSuccess::SessionPing(true);
    ctx.publish_response_ok(topic, param, message_id).await?;

    Ok(())
}

pub async fn send_session_ping_request(ctx: &WalletConnectCtx, topic: &Topic) -> MmResult<(), WalletConnectCtxError> {
    let param = RequestParams::SessionPing(());
    ctx.publish_request(topic, param).await?;

    Ok(())
}

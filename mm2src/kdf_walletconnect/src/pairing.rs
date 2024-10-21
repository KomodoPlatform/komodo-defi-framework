use crate::session::{WcRequestResponseResult, THIRTY_DAYS};
use crate::{error::WalletConnectError, WalletConnectCtx};

use chrono::Utc;
use mm2_err_handle::prelude::MmResult;
use relay_rpc::domain::MessageId;
use relay_rpc::rpc::params::pairing_ping::PairingPingRequest;
use relay_rpc::rpc::params::{RelayProtocolMetadata, RequestParams};
use relay_rpc::{domain::Topic,
                rpc::params::{pairing_delete::PairingDeleteRequest, pairing_extend::PairingExtendRequest,
                              ResponseParamsSuccess}};

pub(crate) async fn reply_pairing_ping_response(
    ctx: &WalletConnectCtx,
    topic: &Topic,
    message_id: &MessageId,
) -> MmResult<(), WalletConnectError> {
    let param = ResponseParamsSuccess::PairingPing(true);
    ctx.publish_response_ok(topic, param, message_id).await?;

    Ok(())
}

pub(crate) async fn reply_pairing_extend_response(
    ctx: &WalletConnectCtx,
    topic: &Topic,
    message_id: &MessageId,
    extend: PairingExtendRequest,
) -> MmResult<(), WalletConnectError> {
    {
        let mut pairings = ctx.pairing.pairings.lock().await;
        if let Some(pairing) = pairings.get_mut(topic.as_ref()) {
            pairing.pairing.expiry = extend.expiry;
            pairing.pairing.active = true;
        };
    }

    let param = ResponseParamsSuccess::PairingPing(true);
    ctx.publish_response_ok(topic, param, message_id).await?;

    Ok(())
}

pub(crate) async fn reply_pairing_delete_response(
    ctx: &WalletConnectCtx,
    topic: &Topic,
    message_id: &MessageId,
    _delete: PairingDeleteRequest,
) -> MmResult<(), WalletConnectError> {
    {
        ctx.pairing.disconnect(topic.as_ref(), &ctx.client).await?;
    }

    let param = ResponseParamsSuccess::PairingDelete(true);
    ctx.publish_response_ok(topic, param, message_id).await?;

    Ok(())
}

use crate::{error::WalletConnectError,
            pairing::{reply_pairing_delete_response, reply_pairing_extend_response, reply_pairing_ping_response},
            session::rpc::{delete::reply_session_delete_request,
                           event::handle_session_event,
                           extend::reply_session_extend_request,
                           ping::reply_session_ping_request,
                           propose::{process_session_propose_response, reply_session_proposal_request},
                           settle::reply_session_settle_request,
                           update::reply_session_update_request},
            WalletConnectCtx};

use common::log::{info, LogOnError};
use futures::sink::SinkExt;
use mm2_err_handle::prelude::{MmError, MmResult};
use relay_rpc::domain::{MessageId, Topic};
use relay_rpc::rpc::{params::ResponseParamsSuccess, Params, Request, Response};

pub(crate) type SessionMessageType = MmResult<SessionMessage, String>;
#[derive(Debug)]
pub struct SessionMessage {
    pub message_id: MessageId,
    pub topic: Topic,
    pub data: ResponseParamsSuccess,
}

pub(crate) async fn process_inbound_request(
    ctx: &WalletConnectCtx,
    request: Request,
    topic: &Topic,
) -> MmResult<(), WalletConnectError> {
    let message_id = request.id;
    match request.params {
        Params::SessionPropose(proposal) => reply_session_proposal_request(ctx, proposal, topic, &message_id).await?,
        Params::SessionExtend(param) => reply_session_extend_request(ctx, topic, &message_id, param).await?,
        Params::SessionDelete(param) => reply_session_delete_request(ctx, topic, &message_id, param).await?,
        Params::SessionPing(()) => reply_session_ping_request(ctx, topic, &message_id).await?,
        Params::SessionSettle(param) => reply_session_settle_request(ctx, topic, &message_id, param).await?,
        Params::SessionUpdate(param) => reply_session_update_request(ctx, topic, &message_id, param).await?,
        Params::SessionEvent(param) => handle_session_event(ctx, topic, &message_id, param).await?,
        Params::SessionRequest(_param) => {
            // TODO: Implement when integrating KDF as a Dapp.
            return MmError::err(WalletConnectError::NotImplemented);
        },

        Params::PairingPing(_param) => reply_pairing_ping_response(ctx, topic, &message_id).await?,
        Params::PairingDelete(param) => reply_pairing_delete_response(ctx, topic, &message_id, param).await?,
        Params::PairingExtend(param) => reply_pairing_extend_response(ctx, topic, &message_id, param).await?,
        _ => {
            info!("Unknown request params received.");
            return MmError::err(WalletConnectError::InvalidRequest);
        },
    };

    Ok(())
}

pub(crate) async fn process_inbound_response(ctx: &WalletConnectCtx, response: Response, topic: &Topic) {
    let message_id = response.id();
    let result = match response {
        Response::Success(value) => match serde_json::from_value::<ResponseParamsSuccess>(value.result) {
            Ok(data) => {
                // TODO: move to session::proposal mod and spawn in a different thread to avoid
                // blocking
                if let ResponseParamsSuccess::SessionPropose(propose) = &data {
                    process_session_propose_response(ctx, topic, propose).await.error_log();
                }

                Ok(SessionMessage {
                    message_id,
                    topic: topic.clone(),
                    data,
                })
            },
            Err(e) => MmError::err(e.to_string()),
        },
        Response::Error(err) => MmError::err(format!("{err:?}")),
    };

    let mut sender = ctx.message_tx.clone();
    sender.send(result).await.error_log();
}

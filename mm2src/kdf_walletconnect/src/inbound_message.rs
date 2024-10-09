use common::log::info;
use futures::SinkExt;
use mm2_err_handle::prelude::{MmError, MmResult};
use relay_rpc::{domain::Topic,
                rpc::{params::ResponseParamsSuccess, Params, Request, Response}};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{error::WalletConnectCtxError,
            pairing::{reply_pairing_delete_response, reply_pairing_extend_response, reply_pairing_ping_response},
            session::rpc::{delete::reply_session_delete_request,
                           event::SessionEvents,
                           extend::reply_session_extend_request,
                           ping::reply_session_ping_request,
                           propose::{process_session_propose_response, reply_session_proposal_request},
                           settle::reply_session_settle_request,
                           update::reply_session_update_request},
            WalletConnectCtx};

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SuccessResponses {
    ResponseParamsSuccess(ResponseParamsSuccess),
    Other(Value),
}

pub(crate) async fn process_inbound_request(
    ctx: &WalletConnectCtx,
    request: Request,
    topic: &Topic,
) -> MmResult<(), WalletConnectCtxError> {
    let message_id = request.id;
    match request.params {
        Params::SessionPropose(proposal) => reply_session_proposal_request(ctx, proposal, topic, &message_id).await?,
        Params::SessionExtend(param) => reply_session_extend_request(ctx, topic, &message_id, param).await?,
        Params::SessionDelete(param) => reply_session_delete_request(ctx, topic, &message_id, param).await?,
        Params::SessionPing(()) => reply_session_ping_request(ctx, topic, &message_id).await?,
        Params::SessionSettle(param) => reply_session_settle_request(ctx, topic, &message_id, param).await?,
        Params::SessionUpdate(param) => reply_session_update_request(ctx, topic, &message_id, param).await?,
        Params::SessionEvent(param) => {
            SessionEvents::from_events(param)?
                .handle_session_event(ctx, topic, &message_id)
                .await?
        },
        Params::SessionRequest(_param) => {
            // TODO: Implement when integrating KDF as a Dapp.
            return MmError::err(WalletConnectCtxError::NotImplemented);
        },

        Params::PairingPing(_param) => reply_pairing_ping_response(ctx, topic, &message_id).await?,
        Params::PairingDelete(param) => reply_pairing_delete_response(ctx, topic, &message_id, param).await?,
        Params::PairingExtend(param) => reply_pairing_extend_response(ctx, topic, &message_id, param).await?,
        _ => {
            info!("Unknown request params received.");
            return MmError::err(WalletConnectCtxError::InvalidRequest);
        },
    };

    Ok(())
}

pub(crate) async fn process_inbound_response(
    ctx: &WalletConnectCtx,
    response: Response,
    topic: &Topic,
) -> MmResult<(), WalletConnectCtxError> {
    let message_id = response.id();

    match response {
        Response::Success(value) => {
            let success_response = serde_json::from_value::<SuccessResponses>(value.result)?;
            match success_response {
                SuccessResponses::ResponseParamsSuccess(params) => match params {
                    ResponseParamsSuccess::SessionPropose(param) => {
                        process_session_propose_response(ctx, topic, param).await
                    },
                    ResponseParamsSuccess::SessionSettle(success)
                    | ResponseParamsSuccess::SessionUpdate(success)
                    | ResponseParamsSuccess::SessionExtend(success)
                    | ResponseParamsSuccess::SessionRequest(success)
                    | ResponseParamsSuccess::SessionEvent(success)
                    | ResponseParamsSuccess::SessionDelete(success)
                    | ResponseParamsSuccess::SessionPing(success)
                    | ResponseParamsSuccess::PairingExtend(success)
                    | ResponseParamsSuccess::PairingDelete(success)
                    | ResponseParamsSuccess::PairingPing(success) => {
                        if !success {
                            return MmError::err(WalletConnectCtxError::UnSuccessfulResponse(format!(
                                "Unsuccessful response={params:?}"
                            )));
                        };

                        Ok(())
                    },
                },
                SuccessResponses::Other(value) => {
                    println!("Received: {value:?}");
                    ctx.session_request_sender
                        .lock()
                        .await
                        .send((message_id, value))
                        .await
                        .ok();
                    Ok(())
                },
            }
        },
        Response::Error(err) => {
            // TODO: handle error properly
            println!("Error: {err:?}");
            Ok(())
        },
    }
}

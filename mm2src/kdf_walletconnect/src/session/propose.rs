use super::{settle::send_session_settle_request, SessionInfo};
use crate::{error::WalletConnectCtxError,
            session::{SessionKey, SessionType, THIRTY_DAYS},
            WalletConnectCtx};

use chrono::Utc;
use mm2_err_handle::map_to_mm::MapToMmResult;
use mm2_err_handle::prelude::MmResult;
use relay_rpc::{domain::{MessageId, Topic},
                rpc::params::{session::ProposeNamespaces,
                              session_propose::{Proposer, SessionProposeRequest, SessionProposeResponse},
                              Metadata, RequestParams, ResponseParamsSuccess}};
use std::ops::Deref;

/// Creates a new session proposal form topic and metadata.
pub(crate) async fn create_proposal_session(
    ctx: &WalletConnectCtx,
    topic: Topic,
    required_namespaces: Option<ProposeNamespaces>,
) -> MmResult<(), WalletConnectCtxError> {
    let proposer = Proposer {
        metadata: ctx.metadata.clone(),
        public_key: hex::encode(ctx.sessions.public_key.as_bytes()),
    };
    let session_proposal = RequestParams::SessionPropose(SessionProposeRequest {
        relays: vec![ctx.relay.clone()],
        proposer,
        required_namespaces: required_namespaces.unwrap_or(ctx.namespaces.clone()),
    });

    ctx.publish_request(&topic, session_proposal).await?;

    Ok(())
}

async fn send_proposal_request_response(
    ctx: &WalletConnectCtx,
    topic: &Topic,
    message_id: &MessageId,
    responder_public_key: String,
) -> MmResult<(), WalletConnectCtxError> {
    let param = ResponseParamsSuccess::SessionPropose(SessionProposeResponse {
        relay: ctx.relay.clone(),
        responder_public_key,
    });

    ctx.publish_response_ok(topic, param, message_id).await?;

    Ok(())
}

/// Process session proposal request
/// https://specs.walletconnect.com/2.0/specs/clients/sign/session-proposal
pub async fn process_proposal_request(
    ctx: &WalletConnectCtx,
    proposal: SessionProposeRequest,
    topic: &Topic,
    message_id: &MessageId,
) -> MmResult<(), WalletConnectCtxError> {
    let sender_public_key = hex::decode(&proposal.proposer.public_key)?
        .as_slice()
        .try_into()
        .unwrap();

    let session_key = SessionKey::from_osrng(&sender_public_key)?;
    let session_topic: Topic = session_key.generate_topic().into();
    let subscription_id = ctx
        .client
        .subscribe(session_topic.clone())
        .await
        .map_to_mm(|err| WalletConnectCtxError::SubscriptionError(err.to_string()))?;

    let session = SessionInfo::new(
        ctx,
        subscription_id,
        session_key,
        topic.clone(),
        proposal.proposer.metadata,
        SessionType::Controller,
    );
    session
        .propose_namespaces
        .supported(&proposal.required_namespaces)
        .map_to_mm(|err| WalletConnectCtxError::InternalError(err.to_string()))?;

    {
        let mut sessions = ctx.sessions.deref().lock().await;
        _ = sessions.insert(session_topic.clone(), session.clone());
    }

    {
        send_session_settle_request(ctx, session, session_topic).await?;
    };

    send_proposal_request_response(ctx, topic, message_id, proposal.proposer.public_key).await
}

/// Process session propose reponse.
pub(crate) async fn process_session_propose_response(
    ctx: &WalletConnectCtx,
    pairing_topic: &Topic,
    response: SessionProposeResponse,
) -> MmResult<(), WalletConnectCtxError> {
    let other_public_key = hex::decode(&response.responder_public_key)?
        .as_slice()
        .try_into()
        .unwrap();

    let mut session_key = SessionKey::new(ctx.sessions.public_key);
    session_key.generate_symmetric_key(&ctx.sessions.keypair, &other_public_key)?;

    let session_topic: Topic = session_key.generate_topic().into();
    let subscription_id = ctx
        .client
        .subscribe(session_topic.clone())
        .await
        .map_to_mm(|err| WalletConnectCtxError::SubscriptionError(err.to_string()))?;

    let mut session = SessionInfo::new(
        ctx,
        subscription_id,
        session_key,
        pairing_topic.clone(),
        Metadata::default(),
        SessionType::Controller,
    );
    session.relay = response.relay;
    session.expiry = Utc::now().timestamp() as u64 + THIRTY_DAYS;
    session.controller.public_key = response.responder_public_key;

    let mut sessions = ctx.sessions.lock().await;
    sessions.insert(session_topic.clone(), session.clone());

    // Activate pairing_topic
    ctx.pairing.activate(pairing_topic.as_ref()).await?;

    Ok(())
}

use crate::session::{Session, SessionProperties};
use crate::storage::WalletConnectStorageOps;
use crate::{error::WalletConnectError, WalletConnectCtxImpl};

use common::log::{debug, info};
use mm2_err_handle::prelude::{MapMmError, MmResult};
use relay_rpc::domain::Topic;
use relay_rpc::rpc::params::session_settle::SessionSettleRequest;

pub(crate) async fn send_session_settle_request(
    _ctx: &WalletConnectCtxImpl,
    _session_info: &Session,
) -> MmResult<(), WalletConnectError> {
    // let mut settled_namespaces = BTreeMap::<String, Namespace>::new();
    // let nam
    // settled_namespaces.insert("eip155".to_string(), Namespace {
    //     chains: Some(SUPPORTED_CHAINS.iter().map(|c| c.to_string()).collect()),
    //     methods: SUPPORTED_METHODS.iter().map(|m| m.to_string()).collect(),
    //     events: SUPPORTED_EVENTS.iter().map(|e| e.to_string()).collect(),
    //     accounts: None,
    // });
    //
    // let request = RequestParams::SessionSettle(SessionSettleRequest {
    //     relay: session_info.relay.clone(),
    //     controller: session_info.controller.clone(),
    //     namespaces: SettleNamespaces(settled_namespaces),
    //     expiry: Utc::now().timestamp() as u64 + THIRTY_DAYS,
    //     session_properties: None,
    // });
    //
    // ctx.publish_request(&session_info.topic, request).await?;

    Ok(())
}

/// Process session settle request.
pub(crate) async fn reply_session_settle_request(
    ctx: &WalletConnectCtxImpl,
    topic: &Topic,
    settle: SessionSettleRequest,
) -> MmResult<(), WalletConnectError> {
    {
        let session = ctx.session_manager.get_session_mut(topic);
        if let Some(mut session) = session {
            session.namespaces = settle.namespaces.0;
            session.controller = settle.controller.clone();
            session.relay = settle.relay;
            session.expiry = settle.expiry;

            if let Some(value) = settle.session_properties {
                let session_properties = serde_json::from_str::<SessionProperties>(&value.to_string())?;
                session.session_properties = Some(session_properties);
            }

            // Update storage session.
            ctx.session_manager
                .storage()
                .update_session(&session)
                .await
                .mm_err(|err| WalletConnectError::StorageError(err.to_string()))?;
        };
    }
    info!("Session successfully settled for topic: {:?}", topic);

    // Delete other sessions with same controller
    // TODO: we might not want to do this!
    let all_sessions = ctx.session_manager.get_sessions_full();
    for session in all_sessions {
        if session.controller == settle.controller && session.topic.as_ref() != topic.as_ref() {
            ctx.client.unsubscribe(session.topic.clone()).await?;
            ctx.client.unsubscribe(session.pairing_topic.clone()).await?;
            ctx.session_manager
                .storage()
                .delete_session(&session.topic.clone())
                .await
                .mm_err(|err| WalletConnectError::StorageError(err.to_string()))?;

            // Optionally: Remove from active sessions in memory too
            ctx.session_manager.delete_session(&session.topic);
            ctx.drop_session(&session.topic).await?;
            debug!("Deleted previous session with topic: {:?}", session.topic);
        }
    }

    Ok(())
}

pub(crate) mod key;
pub mod rpc;

use crate::{error::WalletConnectCtxError, WalletConnectCtx};

use chrono::Utc;
use common::log::debug;
use dashmap::mapref::one::{Ref, RefMut};
use dashmap::DashMap;
use futures::lock::Mutex;
use key::SessionKey;
use mm2_err_handle::prelude::MmResult;
use relay_client::websocket::Client;
use relay_rpc::domain::Topic;
use relay_rpc::rpc::params::session::Namespace;
use relay_rpc::rpc::params::session_propose::Proposer;
use relay_rpc::rpc::params::IrnMetadata;
use relay_rpc::{domain::SubscriptionId,
                rpc::params::{session::ProposeNamespaces, session_settle::Controller, Metadata, Relay}};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

pub(crate) const FIVE_MINUTES: u64 = 300;
pub(crate) const THIRTY_DAYS: u64 = 60 * 60 * 30;

pub(crate) type WcRequestResponseResult = MmResult<(Value, IrnMetadata), WalletConnectCtxError>;

/// In the WalletConnect protocol, a session involves two parties: a controller
/// (typically a wallet) and a proposer (typically a dApp). This enum is used
/// to distinguish between these two roles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionType {
    /// Represents the controlling party in a session, typically a wallet.
    Controller,
    /// Represents the proposing party in a session, typically a dApp.
    Proposer,
}

impl ToString for SessionType {
    fn to_string(&self) -> String {
        match self {
            Self::Controller => "Controller".to_string(),
            Self::Proposer => "Proposer".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionRpcInfo {
    pub topic: String,
    pub metadata: Metadata,
    pub peer_pubkey: String,
    pub pairing_topic: String,
    pub namespaces: BTreeMap<String, Namespace>,
    pub subscription_id: SubscriptionId,
    pub properties: Option<SessionProperties>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KeyInfo {
    pub chain_id: String,
    pub name: String,
    pub algo: String,
    pub pub_key: String,
    pub address: String,
    pub bech32_address: String,
    pub ethereum_hex_address: String,
    pub is_nano_ledger: bool,
    pub is_keystone: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionProperties {
    #[serde(default, deserialize_with = "deserialize_keys_from_string")]
    pub keys: Option<Vec<KeyInfo>>,
}

fn deserialize_keys_from_string<'de, D>(deserializer: D) -> Result<Option<Vec<KeyInfo>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum KeysField {
        String(String),
        Vec(Vec<KeyInfo>),
        None,
    }

    match KeysField::deserialize(deserializer)? {
        KeysField::String(key_string) => serde_json::from_str(&key_string)
            .map(Some)
            .map_err(serde::de::Error::custom),
        KeysField::Vec(keys) => Ok(Some(keys)),
        KeysField::None => Ok(None),
    }
}

/// This struct is typically used in the core session management logic of a WalletConnect
/// implementation. It's used to store, retrieve, and update session information throughout
/// the lifecycle of a WalletConnect connection.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Session {
    /// Session topic
    pub topic: Topic,
    /// Pairing subscription id.
    pub subscription_id: SubscriptionId,
    /// Session symmetric key
    pub session_key: SessionKey,
    /// Information about the controlling party (typically a wallet).
    pub controller: Controller,
    /// Information about the proposing party (typically a dApp).
    pub proposer: Proposer,
    /// Details about the relay used for communication.
    pub relay: Relay,
    /// Agreed-upon namespaces for the session, mapping namespace strings to their definitions.
    pub namespaces: BTreeMap<String, Namespace>,
    /// Namespaces proposed for the session, may differ from agreed namespaces.
    pub propose_namespaces: ProposeNamespaces,
    /// Unix timestamp (in seconds) when the session expires.
    pub expiry: u64,
    /// Topic used for the initial pairing process.
    pub pairing_topic: Topic,
    /// Indicates whether this session info represents a Controller or Proposer perspective.
    pub session_type: SessionType,
    pub session_properties: Option<SessionProperties>,
}

impl Session {
    pub fn new(
        ctx: &WalletConnectCtx,
        session_topic: Topic,
        subscription_id: SubscriptionId,
        session_key: SessionKey,
        pairing_topic: Topic,
        metadata: Metadata,
        session_type: SessionType,
    ) -> Self {
        // handle proposer or controller
        let (proposer, controller) = match session_type {
            SessionType::Proposer => (
                Proposer {
                    public_key: hex::encode(session_key.diffie_public_key()),
                    metadata,
                },
                Controller::default(),
            ),
            SessionType::Controller => (Proposer::default(), Controller {
                public_key: hex::encode(session_key.diffie_public_key()),
                metadata,
            }),
        };

        Self {
            subscription_id,
            session_key,
            controller,
            namespaces: BTreeMap::new(),
            proposer,
            propose_namespaces: ctx.required_namespaces.clone(),
            relay: ctx.relay.clone(),
            expiry: Utc::now().timestamp() as u64 + FIVE_MINUTES,
            pairing_topic,
            session_type,
            topic: session_topic,
            session_properties: None,
        }
    }

    pub(crate) fn extend(&mut self, till: u64) { self.expiry = till; }
}

/// Internal implementation of session management.
#[derive(Default, Debug)]
struct SessionManagementImpl {
    /// The currently active session topic.
    active_topic: Mutex<Option<Topic>>,
    /// A thread-safe map of sessions indexed by topic.
    sessions: DashMap<Topic, Session>,
}

pub struct SessionManagement(Arc<SessionManagementImpl>);

impl Default for SessionManagement {
    fn default() -> Self { Self::new() }
}

impl From<Session> for SessionRpcInfo {
    fn from(value: Session) -> Self {
        Self {
            topic: value.topic.to_string(),
            metadata: value.controller.metadata,
            peer_pubkey: value.controller.public_key,
            pairing_topic: value.pairing_topic.to_string(),
            namespaces: value.namespaces,
            subscription_id: value.subscription_id,
            properties: value.session_properties,
        }
    }
}

#[allow(unused)]
impl SessionManagement {
    pub fn new() -> Self { Self(Default::default()) }

    /// Inserts the provided `Session` into the session store, associated with the specified topic.
    /// If a session with the same topic already exists, it will be overwritten.
    pub(crate) async fn add_session(&self, session: Session) {
        // set active session topic.
        *self.0.active_topic.lock().await = Some(session.topic.clone());

        // insert session
        self.0.sessions.insert(session.topic.clone(), session);
    }

    /// Removes the session corresponding to the specified topic from the session store.
    /// If the session does not exist, this method does nothing.
    pub(crate) async fn delete_session(&self, topic: &Topic) -> Option<Session> {
        debug!("Deleting session with topic: {topic}");
        let mut active_topic = self.0.active_topic.lock().await;

        // Remove the session and get the removed session (if any)
        let removed_session = self.0.sessions.remove(topic).map(|(_, session)| session);

        // Update active topic if necessary
        if active_topic.as_ref() == Some(topic) {
            // If the deleted session was the active one, find a new active session
            *active_topic = self.0.sessions.iter().next().map(|session| session.topic.clone());

            if let Some(new_active_topic) = active_topic.as_ref() {
                debug!("New session with topic: {} activated!", new_active_topic);
            } else {
                debug!("No active sessions remaining");
            }
        }

        removed_session
    }

    /// Retrieves a cloned session associated with a given topic.
    pub fn get_session(&self, topic: &Topic) -> Option<Ref<'_, Topic, Session>> { self.0.sessions.get(topic) }

    /// Retrieves a mutable reference to the session associated with a given topic.
    pub(crate) async fn get_session_mut(&self, topic: &Topic) -> Option<RefMut<'_, Topic, Session>> {
        self.0.sessions.get_mut(topic)
    }

    /// Returns an `Option<Session>` containing the active session if it exists; otherwise, returns `None`.
    pub async fn get_session_active(&self) -> Option<Ref<'_, Topic, Session>> {
        let active_topic = self.0.active_topic.lock().await;
        if let Some(ref topic) = *active_topic {
            self.get_session(topic)
        } else {
            None
        }
    }

    /// Retrieves all sessions(active and inactive)
    pub fn get_sessions(&self) -> Vec<SessionRpcInfo> {
        self.0
            .sessions
            .clone()
            .into_iter()
            .map(|(topic, session)| SessionRpcInfo {
                topic: topic.to_string(),
                metadata: session.controller.metadata.clone(),
                peer_pubkey: session.controller.public_key.clone(),
                pairing_topic: session.pairing_topic.to_string(),
                namespaces: session.namespaces.clone(),
                subscription_id: session.subscription_id,
                properties: session.session_properties,
            })
            .collect()
    }

    /// Updates the expiry time of the session associated with the given topic to the specified timestamp.
    /// If the session does not exist, this method does nothing.
    pub(crate) fn extend_session(&self, topic: &Topic, till: u64) {
        debug!("Extending session with topic: {topic}");
        if let Some(mut session) = self.0.sessions.get_mut(topic) {
            session.extend(till);
        }
    }

    /// This method checks all sessions for expiration based on the current time.
    /// Expired sessions are removed from the session store and returned.
    /// If the active session is expired, it is also removed, and the active session is set to `None`.
    pub(crate) async fn delete_expired_sessions(&self) -> Vec<Session> {
        let now = Utc::now().timestamp() as u64;
        let mut expired_sessions = Vec::new();

        // Collect session arcs for processing
        for session in self.0.sessions.iter() {
            if session.expiry <= now {
                // Remove the session from the map
                if let Some(session) = self.delete_session(&session.topic).await {
                    expired_sessions.push(session);
                }
            }
        }

        expired_sessions
    }

    /// Retrieves the symmetric key associated with a given topic.
    pub(crate) fn sym_key(&self, topic: &Topic) -> Option<Vec<u8>> {
        self.0
            .sessions
            .get(topic)
            .map(|k| k.session_key.symmetric_key().to_vec())
    }

    pub async fn disconnect_session(&self, topic: &Topic, client: &Client) -> MmResult<(), WalletConnectCtxError> {
        client.unsubscribe(topic.clone()).await?;
        self.delete_session(topic).await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_sample_key_info() -> KeyInfo {
        KeyInfo {
            chain_id: "test-chain".to_string(),
            name: "Test Key".to_string(),
            algo: "secp256k1".to_string(),
            pub_key: "0123456789ABCDEF".to_string(),
            address: "test_address".to_string(),
            bech32_address: "bech32_test_address".to_string(),
            ethereum_hex_address: "0xtest_eth_address".to_string(),
            is_nano_ledger: false,
            is_keystone: false,
        }
    }

    #[test]
    fn test_deserialize_keys_from_string() {
        let key_info = create_sample_key_info();
        let key_json = serde_json::to_string(&vec![key_info.clone()]).unwrap();
        let json = format!(r#"{{"keys": "{}"}}"#, key_json.replace('\"', "\\\""));
        let session: SessionProperties = serde_json::from_str(&json).unwrap();
        assert!(session.keys.is_some());
        assert_eq!(session.keys.unwrap(), vec![key_info]);
    }

    #[test]
    fn test_deserialize_keys_from_vec() {
        let key_info = create_sample_key_info();
        let json = format!(r#"{{"keys": [{}]}}"#, serde_json::to_string(&key_info).unwrap());
        let session: SessionProperties = serde_json::from_str(&json).unwrap();
        assert!(session.keys.is_some());
        assert_eq!(session.keys.unwrap(), vec![key_info]);
    }

    #[test]
    fn test_deserialize_empty_keys() {
        let json = r#"{"keys": []}"#;
        let session: SessionProperties = serde_json::from_str(json).unwrap();
        assert_eq!(session.keys, Some(vec![]));
    }

    #[test]
    fn test_deserialize_no_keys() {
        let json = r#"{}"#;
        let session: SessionProperties = serde_json::from_str(json).unwrap();
        assert_eq!(session.keys, None);
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let key_info = create_sample_key_info();
        let original = SessionProperties {
            keys: Some(vec![key_info]),
        };
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: SessionProperties = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }
}

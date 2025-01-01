pub(crate) mod key;
pub mod rpc;

use crate::chain::WcChainId;
use crate::storage::SessionStorageDb;
use crate::{error::WalletConnectError, WalletConnectCtxImpl};

use chrono::Utc;
use common::log::info;
use derive_more::Display;
use key::SessionKey;
use mm2_err_handle::prelude::{MmError, MmResult};
use relay_rpc::domain::Topic;
use relay_rpc::rpc::params::session::Namespace;
use relay_rpc::rpc::params::session_propose::Proposer;
use relay_rpc::rpc::params::IrnMetadata;
use relay_rpc::{domain::SubscriptionId,
                rpc::params::{session::ProposeNamespaces, session_settle::Controller, Metadata, Relay}};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use wc_common::SymKey;

pub(crate) const FIVE_MINUTES: u64 = 5 * 60;
pub(crate) const THIRTY_DAYS: u64 = 30 * 24 * 60 * 60;

pub(crate) type WcRequestResponseResult = MmResult<(Value, IrnMetadata), WalletConnectError>;

/// In the WalletConnect protocol, a session involves two parties: a controller
/// (typically a wallet) and a proposer (typically a dApp). This enum is used
/// to distinguish between these two roles.
#[derive(Debug, Display, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionType {
    /// Represents the controlling party in a session, typically a wallet.
    Controller,
    /// Represents the proposing party in a session, typically a dApp.
    Proposer,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionRpcInfo {
    pub topic: Topic,
    pub metadata: Metadata,
    pub peer_pubkey: String,
    pub pairing_topic: Topic,
    pub namespaces: BTreeMap<String, Namespace>,
    pub subscription_id: SubscriptionId,
    pub properties: Option<SessionProperties>,
    pub expiry: u64,
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
    /// Session active chain_id
    pub active_chain_id: Option<WcChainId>,
}

impl Session {
    pub fn new(
        ctx: &WalletConnectCtxImpl,
        session_topic: Topic,
        subscription_id: SubscriptionId,
        session_key: SessionKey,
        pairing_topic: Topic,
        metadata: Metadata,
        session_type: SessionType,
    ) -> Self {
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
            propose_namespaces: ProposeNamespaces::default(),
            relay: ctx.relay.clone(),
            expiry: Utc::now().timestamp() as u64 + FIVE_MINUTES,
            pairing_topic,
            session_type,
            topic: session_topic,
            session_properties: None,
            active_chain_id: Default::default(),
        }
    }

    pub(crate) fn extend(&mut self, till: u64) { self.expiry = till; }

    /// Get the active chain ID for the current session.
    pub fn get_active_chain_id(&self) -> &Option<WcChainId> { &self.active_chain_id }

    /// Sets the active chain ID for the current session.
    pub fn set_active_chain_id(&mut self, chain_id: WcChainId) { self.active_chain_id = Some(chain_id); }
}

/// Internal implementation of session management.
struct SessionManagerImpl {
    /// The currently active session topic.
    active_topic: Mutex<Option<Topic>>,
    /// A thread-safe map of sessions indexed by topic.
    sessions: Arc<RwLock<HashMap<Topic, Session>>>,
    pub(crate) storage: SessionStorageDb,
}

pub struct SessionManager(Arc<SessionManagerImpl>);

impl From<Session> for SessionRpcInfo {
    fn from(value: Session) -> Self {
        Self {
            topic: value.topic,
            metadata: value.controller.metadata,
            peer_pubkey: value.controller.public_key,
            pairing_topic: value.pairing_topic,
            namespaces: value.namespaces,
            subscription_id: value.subscription_id,
            properties: value.session_properties,
            expiry: value.expiry,
        }
    }
}

#[allow(unused)]
impl SessionManager {
    pub(crate) fn new(storage: SessionStorageDb) -> Self {
        Self(
            SessionManagerImpl {
                active_topic: Default::default(),
                sessions: Default::default(),
                storage,
            }
            .into(),
        )
    }

    pub(crate) fn read(&self) -> RwLockReadGuard<HashMap<Topic, Session>> {
        self.0.sessions.read().expect("read shouldn't fail")
    }

    pub(crate) fn write(&self) -> RwLockWriteGuard<HashMap<Topic, Session>> {
        self.0.sessions.write().expect("read shouldn't fail")
    }

    pub(crate) fn storage(&self) -> &SessionStorageDb { &self.0.storage }

    /// Get active session topic or return error if no session has been activated.
    pub fn get_active_topic_or_err(&self) -> MmResult<Topic, WalletConnectError> {
        self.0
            .active_topic
            .lock()
            .unwrap()
            .clone()
            .ok_or(MmError::new(WalletConnectError::SessionError(
                "No active session".to_owned(),
            )))
    }

    /// Inserts `Session` into the session store, associated with the specified topic.
    /// If a session with the same topic already exists, it will be overwritten.
    pub(crate) fn add_session(&self, session: Session) {
        // set active session topic.
        *self.0.active_topic.lock().unwrap() = Some(session.topic.clone());
        // insert session
        self.write().insert(session.topic.clone(), session);
    }

    /// Removes session corresponding to the specified topic from the session store.
    /// If the session does not exist, this method does nothing.
    pub(crate) fn delete_session(&self, topic: &Topic) -> Option<Session> {
        info!("[{topic}] Deleting session with topic");
        let mut active_topic = self.0.active_topic.lock().unwrap();

        // Remove the session and get the removed session (if any)
        let removed_session = self.write().remove(topic);

        // Update active topic if necessary
        if active_topic.as_ref() == Some(topic) {
            // If the deleted session was the active one, find a new active session
            *active_topic = self.read().iter().next().map(|(topic, session)| topic.clone());

            if let Some(new_active_topic) = active_topic.as_ref() {
                info!("[{new_active_topic}] New session with topic activated!");
            }
        }

        removed_session
    }

    pub fn set_active_session(&self, topic: &Topic) -> MmResult<(), WalletConnectError> {
        let mut active_topic = self.0.active_topic.lock().unwrap();
        let session = self
            .get_session(topic)
            .ok_or(MmError::new(WalletConnectError::SessionError(
                "Session not found".to_owned(),
            )))?;

        *active_topic = Some(session.topic);

        Ok(())
    }

    /// Retrieves a cloned session associated with a given topic.
    pub fn get_session(&self, topic: &Topic) -> Option<Session> { self.read().get(topic).cloned() }

    /// returns an `option<session>` containing the active session if it exists; otherwise, returns `none`.
    pub fn get_session_active(&self) -> Option<Session> {
        self.0
            .active_topic
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|topic| self.get_session(topic))
    }

    /// Retrieves all sessions(active and inactive)
    pub fn get_sessions(&self) -> impl Iterator<Item = SessionRpcInfo> {
        self.read().clone().into_values().map(|session| session.into())
    }

    pub(crate) fn get_sessions_full(&self) -> impl Iterator<Item = Session> { self.read().clone().into_values() }

    /// Updates the expiry time of the session associated with the given topic to the specified timestamp.
    /// If the session does not exist, this method does nothing.
    pub(crate) fn extend_session(&self, topic: &Topic, till: u64) {
        info!("[{topic}] Extending session with topic");
        if let Some(mut session) = self.write().get_mut(topic) {
            session.extend(till);
        }
    }

    /// Retrieves the symmetric key associated with a given topic.
    pub(crate) fn sym_key(&self, topic: &Topic) -> Option<SymKey> {
        self.get_session(topic).map(|sess| sess.session_key.symmetric_key())
    }

    /// Check if a session exists.
    pub(crate) fn validate_session_exists(&self, topic: &Topic) -> Result<(), MmError<WalletConnectError>> {
        if self.read().contains_key(topic) {
            return Ok(());
        };

        MmError::err(WalletConnectError::SessionError(
            "No active WalletConnect session found".to_string(),
        ))
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

pub mod chain;
mod connection_handler;
#[allow(unused)] pub mod error;
mod inbound_message;
mod metadata;
#[allow(unused)] mod pairing;
pub mod session;
mod storage;

use chain::{build_required_namespaces,
            tendermint::{cosmos_get_accounts_impl, cosmos_sign_direct_impl, CosmosAccount, CosmosTxSignedData},
            SUPPORTED_CHAINS};
use common::log::info;
use common::{executor::SpawnFuture, log::error};
use connection_handler::{maintain_client_connection, Handler};
use error::WalletConnectCtxError;
use futures::{channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
              lock::Mutex,
              StreamExt};
use inbound_message::{process_inbound_request, process_inbound_response};
use metadata::{generate_metadata, AUTH_TOKEN_SUB, PROJECT_ID, RELAY_ADDRESS};
use mm2_core::mm_ctx::{from_ctx, MmArc};
use mm2_err_handle::prelude::*;
use pairing_api::PairingClient;
use relay_client::{websocket::{Client, PublishedMessage},
                   ConnectionOptions, MessageIdGenerator};
use relay_rpc::rpc::params::{session::{Namespace, SettleNamespaces},
                             RelayProtocolMetadata, RequestParams};
use relay_rpc::{auth::{ed25519_dalek::SigningKey, AuthToken},
                domain::{MessageId, Topic},
                rpc::{params::{session::ProposeNamespaces, IrnMetadata, Metadata, Relay, ResponseParamsError,
                               ResponseParamsSuccess},
                      ErrorResponse, Payload, Request, Response, SuccessfulResponse}};
use serde_json::Value;
use session::{key::SymKeyPair, SessionManagement};
use std::{sync::Arc, time::Duration};
use storage::SessionStorageDb;
use storage::WalletConnectStorageOps;
use wc_common::{decode_and_decrypt_type0, encrypt_and_encode, EnvelopeType};

use crate::session::rpc::propose::send_proposal_request;

pub(crate) const SUPPORTED_PROTOCOL: &str = "irn";
const DEFAULT_CHAIN_ID: &str = "cosmoshub-4"; // tendermint e.g ATOM

type SessionEventMessage = (MessageId, Value);

pub struct WalletConnectCtx {
    pub client: Client,
    pub pairing: PairingClient,
    pub session: SessionManagement,
    pub active_chain_id: Arc<Mutex<String>>,

    pub(crate) key_pair: SymKeyPair,
    pub(crate) storage: SessionStorageDb,

    relay: Relay,
    metadata: Metadata,
    namespaces: Arc<Mutex<SettleNamespaces>>,
    required_namespaces: ProposeNamespaces,
    subscriptions: Arc<Mutex<Vec<Topic>>>,
    inbound_message_handler: Arc<Mutex<UnboundedReceiver<PublishedMessage>>>,
    connection_live_handler: Arc<Mutex<UnboundedReceiver<()>>>,
    session_request_sender: Arc<Mutex<UnboundedSender<SessionEventMessage>>>,
    session_request_handler: Arc<Mutex<UnboundedReceiver<SessionEventMessage>>>,
}

impl Drop for WalletConnectCtx {
    fn drop(&mut self) {
        println!("drop");
    }
}

impl WalletConnectCtx {
    pub fn try_init(ctx: &MmArc) -> MmResult<Self, WalletConnectCtxError> {
        let (msg_sender, msg_receiver) = unbounded();
        let (conn_live_sender, conn_live_receiver) = unbounded();
        let (session_request_sender, session_request_receiver) = unbounded();

        let pairing = PairingClient::new();
        let client = Client::new(Handler::new("Komodefi", msg_sender, conn_live_sender));

        let required = build_required_namespaces();

        let relay = Relay {
            protocol: SUPPORTED_PROTOCOL.to_string(),
            data: None,
        };

        let storage = SessionStorageDb::init(ctx)?;

        Ok(Self {
            client,
            pairing,
            session: SessionManagement::new(),
            active_chain_id: Arc::new(Mutex::new(DEFAULT_CHAIN_ID.to_string())),
            relay,
            namespaces: Default::default(),
            required_namespaces: required,
            metadata: generate_metadata(),
            key_pair: SymKeyPair::new(),
            storage,
            inbound_message_handler: Arc::new(Mutex::new(msg_receiver)),
            connection_live_handler: Arc::new(Mutex::new(conn_live_receiver)),
            session_request_handler: Arc::new(Mutex::new(session_request_receiver)),
            session_request_sender: Arc::new(Mutex::new(session_request_sender)),
            subscriptions: Default::default(),
        })
    }

    pub fn from_ctx(ctx: &MmArc) -> MmResult<Arc<WalletConnectCtx>, WalletConnectCtxError> {
        from_ctx(&ctx.wallet_connect, move || {
            Self::try_init(ctx).map_err(|err| err.to_string())
        })
        .map_to_mm(WalletConnectCtxError::InternalError)
    }

    pub async fn connect_client(&self) -> MmResult<(), WalletConnectCtxError> {
        let auth = {
            let key = SigningKey::generate(&mut rand::thread_rng());
            AuthToken::new(AUTH_TOKEN_SUB)
                .aud(RELAY_ADDRESS)
                .ttl(Duration::from_secs(8 * 60 * 60))
                .as_jwt(&key)
                .unwrap()
        };
        let opts = ConnectionOptions::new(PROJECT_ID, auth).with_address(RELAY_ADDRESS);

        self.client.connect(&opts).await?;

        Ok(())
    }

    /// Create a WalletConnect pairing connection url.
    pub async fn new_connection(
        &self,
        required_namespaces: Option<ProposeNamespaces>,
    ) -> MmResult<String, WalletConnectCtxError> {
        let (topic, url) = self.pairing.create(self.metadata.clone(), None).await?;

        info!("Subscribing to topic: {topic:?}");

        self.client.subscribe(topic.clone()).await?;

        info!("Subscribed to topic: {topic:?}");

        send_proposal_request(self, topic.clone(), required_namespaces).await?;

        {
            let mut subs = self.subscriptions.lock().await;
            subs.push(topic);
        };

        Ok(url)
    }

    pub async fn is_ledger_connection(&self) -> bool {
        self.session
            .get_session_active()
            .await
            .and_then(|session| session.session_properties.clone())
            .and_then(|props| props.keys.as_ref().cloned())
            .and_then(|keys| keys.first().cloned())
            .map(|key| key.is_nano_ledger)
            .unwrap_or(false)
    }

    pub fn is_chain_supported(&self, chain_id: &str) -> bool { SUPPORTED_CHAINS.iter().any(|chain| chain == &chain_id) }

    pub async fn set_active_chain(&self, chain_id: &str) {
        let mut active_chain = self.active_chain_id.lock().await;
        *active_chain = chain_id.to_owned();
    }

    pub async fn get_active_chain_id(&self) -> String { self.active_chain_id.lock().await.clone() }

    /// Retrieves the available account for a given chain ID.
    pub async fn get_account_for_chain_id(&self, chain_id: &str) -> MmResult<String, WalletConnectCtxError> {
        let active_chain_id = self.get_active_chain_id().await;
        if active_chain_id != chain_id {
            return MmError::err(WalletConnectCtxError::ChainIdMismatch);
        }

        let namespaces = &self
            .session
            .get_session_active()
            .await
            .ok_or(MmError::new(WalletConnectCtxError::NoAccountFound(
                chain_id.to_string(),
            )))?
            .namespaces;
        namespaces
            .iter()
            .find_map(|(_key, namespace)| self.find_account_in_namespace(namespace, chain_id))
            .ok_or(MmError::new(WalletConnectCtxError::NoAccountFound(
                chain_id.to_string(),
            )))
    }

    fn find_account_in_namespace(&self, namespace: &Namespace, chain_id: &str) -> Option<String> {
        let accounts = namespace.accounts.as_ref()?;

        accounts.iter().find_map(|account_name| {
            let parts: Vec<&str> = account_name.split(':').collect();
            if parts.len() >= 3 && parts[1] == chain_id {
                Some(parts[2].to_string())
            } else {
                None
            }
        })
    }

    pub async fn cosmos_get_account(
        &self,
        account_index: usize,
        chain_id: &str,
    ) -> MmResult<CosmosAccount, WalletConnectCtxError> {
        let accounts = cosmos_get_accounts_impl(self, chain_id).await?;

        if accounts.is_empty() {
            return MmError::err(WalletConnectCtxError::EmptyAccount(chain_id.to_string()));
        };

        if accounts.len() < account_index + 1 {
            return MmError::err(WalletConnectCtxError::NoAccountFoundForIndex(account_index));
        };

        Ok(accounts[account_index].clone())
    }

    pub async fn cosmos_send_sign_tx_request(
        &self,
        sign_doc: Value,
        chain_id: &str,
    ) -> MmResult<CosmosTxSignedData, WalletConnectCtxError> {
        cosmos_sign_direct_impl(self, sign_doc, chain_id).await
    }

    async fn sym_key(&self, topic: &Topic) -> MmResult<Vec<u8>, WalletConnectCtxError> {
        {
            if let Some(key) = self.session.sym_key(topic) {
                return Ok(key);
            }
        }

        {
            let pairings = self.pairing.pairings.lock().await;
            if let Some(pairing) = pairings.get(topic.as_ref()) {
                let key = hex::decode(pairing.sym_key.clone())?;
                return Ok(key);
            }
        }

        MmError::err(WalletConnectCtxError::PairingError(format!("Topic not found:{topic}")))
    }

    /// Private function to publish a request.
    async fn publish_request(&self, topic: &Topic, param: RequestParams) -> MmResult<(), WalletConnectCtxError> {
        let irn_metadata = param.irn_metadata();
        let message_id = MessageIdGenerator::new().next();
        let request = Request::new(message_id, param.into());
        self.publish_payload(topic, irn_metadata, Payload::Request(request))
            .await?;

        info!("Outbound request sent!\n");

        Ok(())
    }

    /// Private function to publish a success request response.
    async fn publish_response_ok(
        &self,
        topic: &Topic,
        result: ResponseParamsSuccess,
        message_id: &MessageId,
    ) -> MmResult<(), WalletConnectCtxError> {
        let irn_metadata = result.irn_metadata();
        let value = serde_json::to_value(result)?;
        let response = Response::Success(SuccessfulResponse::new(*message_id, value));
        self.publish_payload(topic, irn_metadata, Payload::Response(response))
            .await?;

        Ok(())
    }

    /// Private function to publish an error request response.
    async fn publish_response_err(
        &self,
        topic: &Topic,
        error_data: ResponseParamsError,
        message_id: &MessageId,
    ) -> MmResult<(), WalletConnectCtxError> {
        let error = error_data.error();
        let irn_metadata = error_data.irn_metadata();
        let response = Response::Error(ErrorResponse::new(*message_id, error));
        self.publish_payload(topic, irn_metadata, Payload::Response(response))
            .await?;

        Ok(())
    }

    /// Private function to publish a payload.
    async fn publish_payload(
        &self,
        topic: &Topic,
        irn_metadata: IrnMetadata,
        payload: Payload,
    ) -> MmResult<(), WalletConnectCtxError> {
        let sym_key = self.sym_key(topic).await?;
        let payload = serde_json::to_string(&payload)?;

        info!("\n Sending Outbound request: {payload}!");

        let message = encrypt_and_encode(EnvelopeType::Type0, payload, &sym_key)?;
        {
            self.client
                .publish(
                    topic.clone(),
                    message,
                    None,
                    irn_metadata.tag,
                    Duration::from_secs(irn_metadata.ttl),
                    irn_metadata.prompt,
                )
                .await?;
        };

        Ok(())
    }

    pub async fn message_handler_event_loop(self: Arc<Self>) {
        let selfi = self.clone();
        let mut recv = self.inbound_message_handler.lock().await;
        while let Some(msg) = recv.next().await {
            if let Err(e) = selfi.handle_published_message(msg).await {
                info!("Error processing message: {:?}", e);
            }
        }
    }

    async fn handle_published_message(&self, msg: PublishedMessage) -> MmResult<(), WalletConnectCtxError> {
        let message = {
            let key = self.sym_key(&msg.topic).await?;
            decode_and_decrypt_type0(msg.message.as_bytes(), &key).unwrap()
        };

        info!("Inbound message payload={message}");

        let payload: Payload = serde_json::from_str(&message)?;

        match payload {
            Payload::Request(request) => process_inbound_request(self, request, &msg.topic).await?,
            Payload::Response(response) => process_inbound_response(self, response, &msg.topic).await?,
        }

        info!("Inbound message was handled successfully");
        Ok(())
    }

    async fn load_session_from_storage(&self) -> MmResult<(), WalletConnectCtxError> {
        let now = chrono::Utc::now().timestamp() as u64;
        let mut sessions = self
            .storage
            .get_all_sessions()
            .await
            .mm_err(|err| WalletConnectCtxError::StorageError(err.to_string()))?;

        sessions.sort_by(|a, b| a.expiry.cmp(&b.expiry));

        for session in sessions {
            // delete expired session
            if now > session.expiry {
                info!("Session {} expired, trying to delete from storage", session.topic);
                if let Err(err) = self.storage.delete_session(&session.topic).await {
                    error!("Unable to delete session: {:?} from storage", err);
                }
                continue;
            };

            let topic = session.topic.clone();
            let pairing_topic = session.pairing_topic.clone();

            info!("Session found! activating :{}", topic);
            self.session.add_session(session).await;

            // subcribe to session topics
            self.client.subscribe(topic).await?;
            self.client.subscribe(pairing_topic).await?;
        }

        Ok(())
    }
}

/// This function spwans related WalletConnect related tasks and needed initialization before
/// WalletConnect can be usable in KDF.
pub async fn initialize_walletconnect(ctx: &MmArc) -> MmResult<(), WalletConnectCtxError> {
    info!("Initializing WalletConnect");

    // Initialized WalletConnectCtx
    let wallet_connect = WalletConnectCtx::from_ctx(ctx)?;

    // Intialize storage.
    wallet_connect.storage.init().await.unwrap();

    // WalletConnectCtx is initialized, now we can connect to relayer client and spawn a watcher
    // loop for disconnection.
    ctx.spawner().spawn(maintain_client_connection(wallet_connect.clone()));

    // spawn message handler event loop
    ctx.spawner().spawn(wallet_connect.message_handler_event_loop());

    info!("WalletConnect initialization completed successfully");

    Ok(())
}

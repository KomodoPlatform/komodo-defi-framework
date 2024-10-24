pub mod chain;
mod connection_handler;
#[allow(unused)] pub mod error;
pub mod inbound_message;
mod metadata;
#[allow(unused)] mod pairing;
pub mod session;
mod storage;

use crate::session::rpc::propose::send_proposal_request;

use chain::{DEFAULT_CHAIN_ID, SUPPORTED_CHAINS, SUPPORTED_PROTOCOL};
use common::log::info;
use common::{executor::SpawnFuture, log::error};
use connection_handler::Handler;
use error::WalletConnectError;
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::lock::Mutex;
use futures::StreamExt;
use inbound_message::{process_inbound_request, process_inbound_response, SessionMessageType};
use metadata::{generate_metadata, AUTH_TOKEN_SUB, PROJECT_ID, RELAY_ADDRESS};
use mm2_core::mm_ctx::{from_ctx, MmArc};
use mm2_err_handle::prelude::*;
use pairing_api::PairingClient;
use relay_client::websocket::{Client, PublishedMessage};
use relay_client::{ConnectionOptions, MessageIdGenerator};
use relay_rpc::auth::{ed25519_dalek::SigningKey, AuthToken};
use relay_rpc::domain::{MessageId, Topic};
use relay_rpc::rpc::params::session::Namespace;
use relay_rpc::rpc::params::{IrnMetadata, Metadata, Relay, RelayProtocolMetadata, RequestParams, ResponseParamsError,
                             ResponseParamsSuccess};
use relay_rpc::rpc::{ErrorResponse, Payload, Request, Response, SuccessfulResponse};
use serde::de::DeserializeOwned;
use serde_json::Value;
use session::{key::SymKeyPair, SessionManager};
use std::{sync::Arc, time::Duration};
use storage::SessionStorageDb;
use storage::WalletConnectStorageOps;
use tokio::time::timeout;
use wc_common::{decode_and_decrypt_type0, encrypt_and_encode, EnvelopeType};

pub struct WalletConnectCtx {
    pub client: Client,
    pub pairing: PairingClient,
    pub session: SessionManager,
    pub active_chain_id: Arc<Mutex<String>>,

    pub(crate) key_pair: SymKeyPair,
    pub(crate) storage: SessionStorageDb,

    relay: Relay,
    metadata: Metadata,
    subscriptions: Arc<Mutex<Vec<Topic>>>,
    inbound_message_handler: Arc<Mutex<UnboundedReceiver<PublishedMessage>>>,
    connection_live_handler: Arc<Mutex<UnboundedReceiver<()>>>,

    message_tx: Arc<Mutex<UnboundedSender<SessionMessageType>>>,
    pub message_rx: Arc<Mutex<UnboundedReceiver<SessionMessageType>>>,
}

impl WalletConnectCtx {
    pub fn try_init(ctx: &MmArc) -> MmResult<Self, WalletConnectError> {
        let (msg_sender, msg_receiver) = unbounded();
        let (conn_live_sender, conn_live_receiver) = unbounded();
        let (message_tx, session_request_receiver) = unbounded();

        let pairing = PairingClient::new();
        let client = Client::new(Handler::new("Komodefi", msg_sender, conn_live_sender));

        let relay = Relay {
            protocol: SUPPORTED_PROTOCOL.to_string(),
            data: None,
        };

        let storage = SessionStorageDb::init(ctx)?;

        Ok(Self {
            client,
            pairing,
            session: SessionManager::new(),
            active_chain_id: Arc::new(Mutex::new(DEFAULT_CHAIN_ID.to_string())),
            relay,
            metadata: generate_metadata(),
            key_pair: SymKeyPair::new(),
            storage,
            subscriptions: Default::default(),

            inbound_message_handler: Arc::new(Mutex::new(msg_receiver)),
            connection_live_handler: Arc::new(Mutex::new(conn_live_receiver)),

            message_rx: Arc::new(Mutex::new(session_request_receiver)),
            message_tx: Arc::new(Mutex::new(message_tx)),
        })
    }

    pub fn from_ctx(ctx: &MmArc) -> MmResult<Arc<WalletConnectCtx>, WalletConnectError> {
        from_ctx(&ctx.wallet_connect, move || {
            Self::try_init(ctx).map_err(|err| err.to_string())
        })
        .map_to_mm(WalletConnectError::InternalError)
    }

    pub async fn connect_client(&self) -> MmResult<(), WalletConnectError> {
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
    pub async fn new_connection(&self) -> MmResult<String, WalletConnectError> {
        let (topic, url) = self.pairing.create(self.metadata.clone(), None).await?;

        info!("Subscribing to topic: {topic:?}");

        self.client.subscribe(topic.clone()).await?;

        info!("Subscribed to topic: {topic:?}");

        send_proposal_request(self, topic.clone()).await?;

        {
            let mut subs = self.subscriptions.lock().await;
            subs.push(topic);
        };

        Ok(url)
    }

    /// Retrieves the symmetric key associated with a given `topic`.
    async fn sym_key(&self, topic: &Topic) -> MmResult<Vec<u8>, WalletConnectError> {
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

        MmError::err(WalletConnectError::InternalError(format!("Topic not found:{topic}")))
    }

    /// Handles an inbound published message by decrypting, decoding, and processing it.
    async fn handle_published_message(&self, msg: PublishedMessage) -> MmResult<(), WalletConnectError> {
        let message = {
            let key = self.sym_key(&msg.topic).await?;
            decode_and_decrypt_type0(msg.message.as_bytes(), &key).unwrap()
        };

        info!("Inbound message payload={message}");

        let payload: Payload = serde_json::from_str(&message)?;

        match payload {
            Payload::Request(request) => process_inbound_request(self, request, &msg.topic).await?,
            Payload::Response(response) => process_inbound_response(self, response, &msg.topic).await,
        }

        info!("Inbound message was handled successfully");
        Ok(())
    }

    /// Loads sessions from storage, activates valid ones, and deletes expired ones.
    async fn load_session_from_storage(&self) -> MmResult<(), WalletConnectError> {
        let now = chrono::Utc::now().timestamp() as u64;
        let mut sessions = self
            .storage
            .get_all_sessions()
            .await
            .mm_err(|err| WalletConnectError::StorageError(err.to_string()))?;

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

    /// function to publish a request.
    pub async fn publish_request(&self, topic: &Topic, param: RequestParams) -> MmResult<(), WalletConnectError> {
        let irn_metadata = param.irn_metadata();
        let message_id = MessageIdGenerator::new().next();
        let request = Request::new(message_id, param.into());
        self.publish_payload(topic, irn_metadata, Payload::Request(request))
            .await?;

        info!("Outbound request sent!\n");

        Ok(())
    }

    /// Private function to publish a success request response.
    pub(crate) async fn publish_response_ok(
        &self,
        topic: &Topic,
        result: ResponseParamsSuccess,
        message_id: &MessageId,
    ) -> MmResult<(), WalletConnectError> {
        let irn_metadata = result.irn_metadata();
        let value = serde_json::to_value(result)?;
        let response = Response::Success(SuccessfulResponse::new(*message_id, value));
        self.publish_payload(topic, irn_metadata, Payload::Response(response))
            .await?;

        Ok(())
    }

    /// Private function to publish an error request response.
    pub(crate) async fn publish_response_err(
        &self,
        topic: &Topic,
        error_data: ResponseParamsError,
        message_id: &MessageId,
    ) -> MmResult<(), WalletConnectError> {
        let error = error_data.error();
        let irn_metadata = error_data.irn_metadata();
        let response = Response::Error(ErrorResponse::new(*message_id, error));
        self.publish_payload(topic, irn_metadata, Payload::Response(response))
            .await?;

        Ok(())
    }

    /// Private function to publish a payload.
    pub(crate) async fn publish_payload(
        &self,
        topic: &Topic,
        irn_metadata: IrnMetadata,
        payload: Payload,
    ) -> MmResult<(), WalletConnectError> {
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

    /// Checks if the current session is connected to a Ledger device.
    /// NOTE: for COSMOS chains only.
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

    /// Checks if a given chain ID is supported.
    pub fn is_chain_supported(&self, chain_id: &str) -> bool { SUPPORTED_CHAINS.iter().any(|&c| c == chain_id) }

    /// Sets the active chain ID for the current session.
    pub async fn set_active_chain(&self, chain_id: &str) {
        let mut active_chain = self.active_chain_id.lock().await;
        *active_chain = chain_id.to_owned();
    }

    /// Retrieves the current active chain ID.
    pub async fn get_active_chain_id(&self) -> String { self.active_chain_id.lock().await.clone() }

    /// Retrieves the available account for a given chain ID.
    pub async fn get_account_for_chain_id(&self, chain_id: &str) -> MmResult<String, WalletConnectError> {
        let namespaces = &self
            .session
            .get_session_active()
            .await
            .ok_or(MmError::new(WalletConnectError::SessionError(
                "No active WalletConnect session found".to_string(),
            )))?
            .namespaces;
        namespaces
            .iter()
            .find_map(|(_key, namespace)| find_account_in_namespace(namespace, chain_id))
            .ok_or(MmError::new(WalletConnectError::NoAccountFound(chain_id.to_string())))
    }

    /// Waits for and handles a WalletConnect session response with arbitrary data.
    pub async fn on_wc_session_response<T, R, F>(&self, func: F) -> MmResult<R, WalletConnectError>
    where
        T: DeserializeOwned,
        F: Fn(T) -> MmResult<R, WalletConnectError>,
    {
        let wait_duration = Duration::from_secs(30);

        while let Ok(Some(resp)) = timeout(wait_duration, self.message_rx.lock().await.next()).await {
            let result = resp.mm_err(WalletConnectError::InternalError)?;
            if let ResponseParamsSuccess::Arbitrary(data) = result.data {
                let data = serde_json::from_value::<T>(data)?;
                let response = ResponseParamsSuccess::SessionEvent(true);
                self.publish_response_ok(&result.topic, response, &result.message_id)
                    .await?;

                return func(data);
            }
        }

        MmError::err(WalletConnectError::NoWalletFeedback)
    }
}

#[async_trait::async_trait]
pub trait WcRequestOps {
    type Error;
    type SignTxData;

    async fn wc_request_sign_tx(
        &self,
        ctx: &WalletConnectCtx,
        chain_id: &str,
        tx_json: Value,
    ) -> Result<Self::SignTxData, Self::Error>;
}

/// This function spwans related WalletConnect related tasks and needed initialization before
/// WalletConnect can be usable in KDF.
pub async fn initialize_walletconnect(ctx: &MmArc) -> MmResult<(), WalletConnectError> {
    info!("Initializing WalletConnect");

    // Initialized WalletConnectCtx
    let wallet_connect = WalletConnectCtx::from_ctx(ctx)?;

    // Intialize storage.
    wallet_connect.storage.init().await.unwrap();

    // WalletConnectCtx is initialized, now we can connect to relayer client and spawn a watcher
    // loop for disconnection.
    ctx.spawner().spawn({
        let this = wallet_connect.clone();
        async move {
            connection_handler::initial_connection(&this).await;
            connection_handler::handle_disconnections(&this).await;
        }
    });

    // spawn message handler event loop
    ctx.spawner().spawn(async move {
        let this = wallet_connect.clone();
        let mut recv = wallet_connect.inbound_message_handler.lock().await;
        while let Some(msg) = recv.next().await {
            if let Err(e) = this.handle_published_message(msg).await {
                info!("Error processing message: {:?}", e);
            }
        }
    });

    info!("WalletConnect initialization completed successfully");
    Ok(())
}

fn find_account_in_namespace(namespace: &Namespace, chain_id: &str) -> Option<String> {
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

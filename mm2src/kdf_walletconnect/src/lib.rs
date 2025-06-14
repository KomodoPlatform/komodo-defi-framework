pub mod chain;
mod connection_handler;
#[allow(unused)] pub mod error;
pub mod inbound_message;
mod metadata;
#[allow(unused)] mod pairing;
pub mod session;
mod storage;

use crate::connection_handler::{handle_disconnections, MAX_BACKOFF};
use crate::session::rpc::propose::send_proposal_request;
use chain::{WcChainId, WcRequestMethods, SUPPORTED_PROTOCOL};
use common::custom_futures::timeout::FutureTimerExt;
use common::executor::abortable_queue::AbortableQueue;
use common::executor::{AbortableSystem, Timer};
use common::log::{debug, info, LogOnError};
use common::{executor::SpawnFuture, log::error};
use connection_handler::Handler;
use error::WalletConnectError;
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use futures::StreamExt;
use inbound_message::{process_inbound_request, process_inbound_response, SessionMessageType};
use metadata::{generate_metadata, AUTH_TOKEN_DURATION, AUTH_TOKEN_SUB, PROJECT_ID, RELAY_ADDRESS};
use mm2_core::mm_ctx::{from_ctx, MmArc};
use mm2_err_handle::prelude::*;
use pairing_api::PairingClient;
use relay_client::websocket::{connection_event_loop as client_event_loop, Client, PublishedMessage};
use relay_client::{ConnectionOptions, MessageIdGenerator};
use relay_rpc::auth::{ed25519_dalek::SigningKey, AuthToken};
use relay_rpc::domain::{MessageId, Topic};
use relay_rpc::rpc::params::session::{Namespace, ProposeNamespaces};
use relay_rpc::rpc::params::session_request::SessionRequestRequest;
use relay_rpc::rpc::params::{session_request::Request as SessionRequest, IrnMetadata, Metadata, Relay,
                             RelayProtocolMetadata, RequestParams, ResponseParamsError, ResponseParamsSuccess};
use relay_rpc::rpc::{ErrorResponse, Payload, Request, Response, SuccessfulResponse};
use serde::de::DeserializeOwned;
use session::rpc::delete::send_session_delete_request;
use session::{key::SymKeyPair, SessionManager};
use session::{EncodingAlgo, Session, SessionProperties, FIVE_MINUTES};
use std::collections::BTreeSet;
use std::ops::Deref;
use std::{sync::{Arc, Mutex},
          time::Duration};
use storage::SessionStorageDb;
use storage::WalletConnectStorageOps;
use timed_map::TimedMap;
use tokio::sync::oneshot;
use wc_common::{decode_and_decrypt_type0, encrypt_and_encode, EnvelopeType, SymKey};

const PUBLISH_TIMEOUT_SECS: f64 = 6.;
const MAX_RETRIES: usize = 5;

#[async_trait::async_trait]
pub trait WalletConnectOps {
    type Error;
    type Params<'a>;
    type SignTxData;
    type SendTxData;

    /// Unique chain_id associated with an activated/supported coin.
    async fn wc_chain_id(&self, ctx: &WalletConnectCtx) -> Result<WcChainId, Self::Error>;

    /// Send sign transaction request to WalletConnect Wallet.
    async fn wc_sign_tx<'a>(
        &self,
        wc: &WalletConnectCtx,
        params: Self::Params<'a>,
    ) -> Result<Self::SignTxData, Self::Error>;

    /// Send sign and send/broadcast transaction request to WalletConnect Wallet.
    async fn wc_send_tx<'a>(
        &self,
        wc: &WalletConnectCtx,
        params: Self::Params<'a>,
    ) -> Result<Self::SendTxData, Self::Error>;

    /// Session topic used to activate this.
    fn session_topic(&self) -> Result<&str, Self::Error>;
}

/// Implements the WalletConnect context, providing functionality for
/// establishing and managing wallet connections.
/// This struct contains the necessary state and methods to handle
/// wallet connection sessions, signing requests, and connection events.
pub struct WalletConnectCtxImpl {
    pub(crate) client: Client,
    pub(crate) pairing: PairingClient,
    pub(crate) key_pair: SymKeyPair,
    pub session_manager: SessionManager,
    relay: Relay,
    metadata: Metadata,
    message_id_generator: MessageIdGenerator,
    pending_requests: Mutex<TimedMap<MessageId, oneshot::Sender<SessionMessageType>>>,
    abortable_system: AbortableQueue,
}

/// A newtype wrapper around a thread-safe reference to `WalletConnectCtxImpl`.
/// Provides shared access to wallet connection functionality through an Arc pointer.
pub struct WalletConnectCtx(pub Arc<WalletConnectCtxImpl>);
impl Deref for WalletConnectCtx {
    type Target = WalletConnectCtxImpl;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl WalletConnectCtx {
    /// Attempt to initialize a new WalletConnect context.
    pub fn try_init(ctx: &MmArc) -> MmResult<Self, WalletConnectError> {
        let abortable_system = ctx
            .abortable_system
            .create_subsystem::<AbortableQueue>()
            .map_to_mm(|err| WalletConnectError::InternalError(err.to_string()))?;
        let storage = SessionStorageDb::new(ctx)?;
        let pairing = PairingClient::new();
        let relay = Relay {
            protocol: SUPPORTED_PROTOCOL.to_string(),
            data: None,
        };
        let (inbound_message_tx, inbound_message_rx) = unbounded();
        let (conn_live_sender, conn_live_receiver) = unbounded();
        let (client, _) = Client::new_with_callback(
            Handler::new("KDF", inbound_message_tx, conn_live_sender),
            |receiver, handler| {
                abortable_system
                    .weak_spawner()
                    .spawn(client_event_loop(receiver, handler))
            },
        );

        let message_id_generator = MessageIdGenerator::new();
        let context = Arc::new(WalletConnectCtxImpl {
            client,
            pairing,
            relay,
            metadata: generate_metadata(),
            key_pair: SymKeyPair::new(),
            session_manager: SessionManager::new(storage),
            pending_requests: Default::default(),
            message_id_generator,
            abortable_system,
        });

        // Connect to relayer client and spawn a watcher loop for disconnection.
        context
            .abortable_system
            .weak_spawner()
            .spawn(context.clone().spawn_connection_initialization_fut(conn_live_receiver));

        // spawn message handler event loop
        context
            .abortable_system
            .weak_spawner()
            .spawn(context.clone().spawn_published_message_fut(inbound_message_rx));

        Ok(Self(context))
    }

    pub fn from_ctx(ctx: &MmArc) -> MmResult<Arc<WalletConnectCtx>, WalletConnectError> {
        from_ctx(&ctx.wallet_connect, move || {
            Self::try_init(ctx).map_err(|err| err.to_string())
        })
        .map_to_mm(WalletConnectError::InternalError)
    }
}

impl WalletConnectCtxImpl {
    /// Establishes initial connection to WalletConnect relay server with linear retry mechanism.
    /// Uses increasing delay between retry attempts starting from 1sec and increase exponentially.
    /// After successful connection, attempts to restore previous session state from storage.
    pub(crate) async fn spawn_connection_initialization_fut(
        self: Arc<Self>,
        connection_live_rx: UnboundedReceiver<Option<String>>,
    ) {
        info!("Initializing WalletConnect connection");
        let mut retry_count = 0;
        let mut retry_secs = 1;

        // Connect to WalletConnect relay client(retry until successful) before proceeeding with other initializations.
        while let Err(err) = self.connect_client().await {
            retry_count += 1;
            error!(
                "Error during initial connection attempt {}: {:?}. Retrying in {retry_secs} seconds...",
                retry_count, err
            );
            Timer::sleep(retry_secs as f64).await;
            retry_secs = std::cmp::min(retry_secs * 2, MAX_BACKOFF);
        }

        // Initialize storage
        if let Err(err) = self.session_manager.storage().init().await {
            error!("Failed to initialize WalletConnect storage, shutting down: {err:?}");
            self.abortable_system.abort_all().error_log();
        };

        // load session from storage
        if let Err(err) = self.load_session_from_storage().await {
            error!("Failed to load sessions from storage, shutting down: {err:?}");
            self.abortable_system.abort_all().error_log();
        };

        // Spawn session disconnection watcher.
        handle_disconnections(&self, connection_live_rx).await;
    }

    /// Attempt to connect to a wallet connection relay server.
    pub async fn connect_client(&self) -> MmResult<(), WalletConnectError> {
        let auth = {
            let key = SigningKey::generate(&mut rand::thread_rng());
            AuthToken::new(AUTH_TOKEN_SUB)
                .aud(RELAY_ADDRESS)
                .ttl(AUTH_TOKEN_DURATION)
                .as_jwt(&key)
                .map_to_mm(|err| WalletConnectError::InternalError(err.to_string()))?
        };
        let opts = ConnectionOptions::new(PROJECT_ID, auth).with_address(RELAY_ADDRESS);
        self.client.connect(&opts).await?;

        Ok(())
    }

    /// Re-connect to WalletConnect relayer and re-subscribes to previously active session topics after reconnection.
    pub(crate) async fn reconnect_and_subscribe(&self) -> MmResult<(), WalletConnectError> {
        self.connect_client().await?;
        let sessions = self
            .session_manager
            .get_sessions()
            .flat_map(|s| vec![s.topic, s.pairing_topic])
            .collect::<Vec<_>>();

        if !sessions.is_empty() {
            self.client.batch_subscribe(sessions).await?;
        }

        Ok(())
    }

    /// Create a WalletConnect pairing connection url.
    pub async fn new_connection(
        &self,
        required_namespaces: serde_json::Value,
        optional_namespaces: Option<serde_json::Value>,
    ) -> MmResult<String, WalletConnectError> {
        let required_namespaces = serde_json::from_value(required_namespaces)?;
        let optional_namespaces = match optional_namespaces {
            Some(value) => serde_json::from_value(value)?,
            None => ProposeNamespaces::default(),
        };
        let (topic, url) = self.pairing.create(self.metadata.clone(), None)?;

        info!("[{topic}] Subscribing to topic");

        for attempt in 0..MAX_RETRIES {
            match self
                .client
                .subscribe(topic.clone())
                .timeout_secs(PUBLISH_TIMEOUT_SECS)
                .await
            {
                Ok(res) => {
                    res.map_to_mm(|err| err.into())?;
                    info!("[{topic}] Subscribed to topic");
                    send_proposal_request(self, &topic, required_namespaces, optional_namespaces).await?;
                    return Ok(url);
                },
                Err(_) => self.wait_until_client_is_online_loop(attempt).await,
            }
        }

        MmError::err(WalletConnectError::InternalError(
            "client connection timeout".to_string(),
        ))
    }

    /// Get symmetric key associated with a for `topic`.
    fn sym_key(&self, topic: &Topic) -> MmResult<SymKey, WalletConnectError> {
        self.session_manager
            .sym_key(topic)
            .or_else(|| self.pairing.sym_key(topic).ok())
            .ok_or_else(|| {
                error!("Failed to find sym_key for topic: {topic}");
                MmError::new(WalletConnectError::InternalError(format!(
                    "topic sym_key not found: {topic}"
                )))
            })
    }

    /// Handles an inbound published message by decrypting, decoding, and processing it.
    async fn handle_published_message(&self, msg: PublishedMessage) -> MmResult<(), WalletConnectError> {
        let message = {
            let key = self.sym_key(&msg.topic)?;
            decode_and_decrypt_type0(msg.message.as_bytes(), &key)?
        };

        info!("[{}] Inbound message payload={message}", msg.topic);

        match serde_json::from_str(&message)? {
            Payload::Request(request) => process_inbound_request(self, request, &msg.topic).await?,
            Payload::Response(response) => process_inbound_response(self, response, &msg.topic).await,
        }

        debug!("[{}] Inbound message was handled successfully", msg.topic);

        Ok(())
    }

    /// Spawns a task that continuously processes published messages from inbound message channel.
    async fn spawn_published_message_fut(self: Arc<Self>, mut recv: UnboundedReceiver<PublishedMessage>) {
        while let Some(msg) = recv.next().await {
            self.handle_published_message(msg)
                .await
                .error_log_with_msg("Error processing message");
        }
    }

    /// Loads sessions from storage, activates valid ones, and deletes expired.
    async fn load_session_from_storage(&self) -> MmResult<(), WalletConnectError> {
        info!("Loading WalletConnect session from storage");
        let now = chrono::Utc::now().timestamp() as u64;
        let sessions = self
            .session_manager
            .storage()
            .get_all_sessions()
            .await
            .mm_err(|err| WalletConnectError::StorageError(err.to_string()))?;
        let mut valid_topics = Vec::with_capacity(sessions.len());
        let mut pairing_topics = Vec::with_capacity(sessions.len());

        // bring most recent active session to the back.
        for session in sessions.into_iter().rev() {
            // delete expired session
            if now > session.expiry {
                debug!("Session {} expired, trying to delete from storage", session.topic);
                self.session_manager
                    .storage()
                    .delete_session(&session.topic)
                    .await
                    .error_log_with_msg(&format!("[{}] Unable to delete session from storage", session.topic));
                continue;
            };

            let topic = session.topic.clone();
            let pairing_topic = session.pairing_topic.clone();
            debug!("[{topic}] Session found! activating");
            self.session_manager.add_session(session);

            valid_topics.push(topic);
            pairing_topics.push(pairing_topic);
        }

        let all_topics = valid_topics.into_iter().chain(pairing_topics).collect::<Vec<_>>();

        if !all_topics.is_empty() {
            self.client.batch_subscribe(all_topics).await?;
        }

        info!("Loaded WalletConnect session from storage");

        Ok(())
    }

    pub fn encode<T: AsRef<[u8]>>(&self, session_topic: &str, data: T) -> String {
        let session_topic = session_topic.into();
        let algo = self
            .session_manager
            .get_session(&session_topic)
            .map(|session| session.encoding_algo.unwrap_or(EncodingAlgo::Hex))
            .unwrap_or(EncodingAlgo::Hex);

        algo.encode(data)
    }

    /// Private function to publish a WC request.
    pub(crate) async fn publish_request(
        &self,
        topic: &Topic,
        param: RequestParams,
    ) -> MmResult<(oneshot::Receiver<SessionMessageType>, Duration), WalletConnectError> {
        let irn_metadata = param.irn_metadata();
        let ttl = irn_metadata.ttl;
        let message_id = self.message_id_generator.next();
        let request = Request::new(message_id, param.into());

        self.publish_payload(topic, irn_metadata, Payload::Request(request))
            .await?;

        let (tx, rx) = oneshot::channel();
        // insert request to map with a reasonable expiration time of 5 minutes
        self.pending_requests
            .lock()
            .unwrap()
            .insert_expirable(message_id, tx, Duration::from_secs(FIVE_MINUTES));

        Ok((rx, Duration::from_secs(ttl)))
    }

    /// Private function to publish a success WC request response.
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
            .await
    }

    /// Private function to publish an error WC request response.
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
            .await
    }

    /// Private function to publish a WC payload.
    pub(crate) async fn publish_payload(
        &self,
        topic: &Topic,
        irn_metadata: IrnMetadata,
        payload: Payload,
    ) -> MmResult<(), WalletConnectError> {
        info!("[{topic}] Publishing message={payload:?}");
        let message = {
            let sym_key = self.sym_key(topic)?;
            let payload = serde_json::to_string(&payload)?;
            encrypt_and_encode(EnvelopeType::Type0, payload, &sym_key)?
        };

        for attempt in 0..MAX_RETRIES {
            match self
                .client
                .publish(
                    topic.clone(),
                    &*message,
                    None,
                    irn_metadata.tag,
                    Duration::from_secs(irn_metadata.ttl),
                    irn_metadata.prompt,
                )
                .timeout_secs(PUBLISH_TIMEOUT_SECS)
                .await
            {
                Ok(Ok(_)) => {
                    info!("[{topic}] Message published successfully");
                    return Ok(());
                },
                Ok(Err(err)) => return MmError::err(err.into()),
                Err(_) => self.wait_until_client_is_online_loop(attempt).await,
            }
        }

        MmError::err(WalletConnectError::InternalError(
            "[{topic}] client connection timeout".to_string(),
        ))
    }

    /// Persistent reconnection and retry strategy keeps the WebSocket connection active,
    /// allowing the client to automatically resume operations after network interruptions or disconnections.
    /// Since TCP handles connection timeouts (which can be lengthy and it's determined by the OS), we're using a shorter timeout here
    /// to detect issues quickly and reconnect as needed.
    async fn wait_until_client_is_online_loop(&self, attempt: usize) {
        debug!("Attempt {} failed due to timeout. Reconnecting...", attempt + 1);
        loop {
            match self.reconnect_and_subscribe().await {
                Ok(_) => {
                    info!("Reconnected and subscribed successfully.");
                    break;
                },
                Err(reconnect_err) => {
                    error!("Reconnection attempt failed: {reconnect_err:?}. Retrying...");
                    Timer::sleep(1.5).await;
                },
            }
        }
    }

    /// Checks if the current session is connected to a Ledger device.
    /// NOTE: for COSMOS chains only.
    pub fn is_ledger_connection(&self, session_topic: &str) -> bool {
        let session_topic = session_topic.into();
        self.session_manager
            .get_session(&session_topic)
            .and_then(|session| session.session_properties)
            .and_then(|props| props.keys.as_ref().cloned())
            // TODO: Please re-check if this is correct. Why do we pick the first KeyInfo? shouldn't we pick the one with matching chain_id?
            .and_then(|keys| keys.first().cloned())
            .map(|key| key.is_nano_ledger)
            .unwrap_or(false)
    }

    /// Checks if the current session is connected via Keplr wallet.
    /// NOTE: for COSMOS chains only.
    pub fn is_keplr_connection(&self, session_topic: &str) -> bool {
        let session_topic = session_topic.into();
        self.session_manager
            .get_session(&session_topic)
            .map(|session| session.controller.metadata.name == "Keplr")
            .unwrap_or_default()
    }

    /// Checks if a given chain ID is supported.
    pub(crate) fn validate_chain_id(
        &self,
        session: &Session,
        chain_id: &WcChainId,
    ) -> MmResult<(), WalletConnectError> {
        if let Some(Namespace { chains, .. }) = session.namespaces.get(chain_id.chain.as_ref()) {
            match chains {
                Some(chains) => {
                    if chains.contains(&chain_id.to_string()) {
                        return Ok(());
                    }
                },
                None => {
                    // https://specs.walletconnect.com/2.0/specs/clients/sign/namespaces#13-chains-might-be-omitted-if-the-caip-2-is-defined-in-the-index
                    if let Some(SessionProperties { keys: Some(keys) }) = &session.session_properties {
                        if keys.iter().any(|k| k.chain_id == chain_id.id) {
                            return Ok(());
                        }
                    }
                },
            };
        }

        // https://specs.walletconnect.com/2.0/specs/clients/sign/namespaces#13-chains-might-be-omitted-if-the-caip-2-is-defined-in-the-index
        if session.namespaces.contains_key(&chain_id.to_string()) {
            return Ok(());
        }

        MmError::err(WalletConnectError::ChainIdNotSupported(chain_id.to_string()))
    }

    /// Validate and send update active chain to WC if needed.
    pub async fn validate_update_active_chain_id(
        &self,
        session_topic: &str,
        chain_id: &WcChainId,
    ) -> MmResult<(), WalletConnectError> {
        let session_topic = session_topic.into();
        let session =
            self.session_manager
                .get_session(&session_topic)
                .ok_or(MmError::new(WalletConnectError::SessionError(
                    "No active WalletConnect session found".to_string(),
                )))?;

        self.validate_chain_id(&session, chain_id)?;

        // TODO: uncomment when WalletConnect wallets start listening to chainChanged event
        // if WcChain::Eip155 != chain_id.chain {
        //     return Ok(());
        // };
        //
        // if let Some(active_chain_id) = session.get_active_chain_id().await {
        //     if chain_id == active_chain_id {
        //         return Ok(());
        //     }
        // };
        //
        // let event = SessionEventRequest {
        //     event: Event {
        //         name: "chainChanged".to_string(),
        //         data: serde_json::to_value(&chain_id.id)?,
        //     },
        //     chain_id: chain_id.to_string(),
        // };
        // self.publish_request(&session.topic, RequestParams::SessionEvent(event))
        //     .await?;
        //
        // let wait_duration = Duration::from_secs(60);
        // if let Ok(Some(resp)) = self.message_rx.lock().await.next().timeout(wait_duration).await {
        //     let result = resp.mm_err(WalletConnectError::InternalError)?;
        //     if let ResponseParamsSuccess::SessionEvent(data) = result.data {
        //         if !data {
        //             return MmError::err(WalletConnectError::PayloadError(
        //                 "Please approve chain id change".to_owned(),
        //             ));
        //         }
        //
        //         self.session
        //             .get_session_mut(&session.topic)
        //             .ok_or(MmError::new(WalletConnectError::SessionError(
        //                 "No active WalletConnect session found".to_string(),
        //             )))?
        //             .set_active_chain_id(chain_id.clone())
        //             .await;
        //     }
        // }

        Ok(())
    }

    /// Get available account for a given chain ID.
    pub fn get_account_and_properties_for_chain_id(
        &self,
        session_topic: &str,
        chain_id: &WcChainId,
    ) -> MmResult<(String, Option<SessionProperties>), WalletConnectError> {
        let session_topic = session_topic.into();
        let session =
            self.session_manager
                .get_session(&session_topic)
                .ok_or(MmError::new(WalletConnectError::SessionError(
                    "No active WalletConnect session found".to_string(),
                )))?;

        if let Some(Namespace {
            accounts: Some(accounts),
            ..
        }) = &session.namespaces.get(chain_id.chain.as_ref())
        {
            if let Some(account) = find_account_in_namespace(accounts, &chain_id.id) {
                return Ok((account, session.session_properties));
            }
        };

        MmError::err(WalletConnectError::NoAccountFound(chain_id.to_string()))
    }

    /// Waits for and handles a WalletConnect session response with arbitrary data.
    /// https://specs.walletconnect.com/2.0/specs/clients/sign/session-events#session_request
    pub async fn send_session_request_and_wait<R>(
        &self,
        session_topic: &str,
        chain_id: &WcChainId,
        method: WcRequestMethods,
        params: serde_json::Value,
    ) -> MmResult<R, WalletConnectError>
    where
        R: DeserializeOwned,
    {
        let session_topic = session_topic.into();
        self.session_manager.validate_session_exists(&session_topic)?;

        let request = SessionRequestRequest {
            chain_id: chain_id.to_string(),
            request: SessionRequest {
                method: method.as_ref().to_string(),
                expiry: None,
                params,
            },
        };
        let (rx, ttl) = self
            .publish_request(&session_topic, RequestParams::SessionRequest(request))
            .await?;

        let response = rx
            .timeout(ttl)
            .await
            .map_to_mm(|_| WalletConnectError::TimeoutError)?
            .map_to_mm(|err| WalletConnectError::InternalError(err.to_string()))??;
        match response.data {
            ResponseParamsSuccess::Arbitrary(data) => Ok(serde_json::from_value::<R>(data)?),
            _ => MmError::err(WalletConnectError::PayloadError("Unexpected response type".to_string())),
        }
    }

    // Destroy WC session.
    pub async fn drop_session(&self, topic: &Topic) -> MmResult<(), WalletConnectError> {
        send_session_delete_request(self, topic).await
    }
}

fn find_account_in_namespace<'a>(accounts: &'a BTreeSet<String>, chain_id: &'a str) -> Option<String> {
    accounts.iter().find_map(move |account_name| {
        let parts: Vec<&str> = account_name.split(':').collect();
        if parts.len() >= 3 && parts[1] == chain_id {
            Some(parts[2].to_string())
        } else {
            None
        }
    })
}

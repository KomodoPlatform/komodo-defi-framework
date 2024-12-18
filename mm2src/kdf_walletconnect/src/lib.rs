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
use relay_rpc::rpc::params::session::Namespace;
use relay_rpc::rpc::params::session_request::SessionRequestRequest;
use relay_rpc::rpc::params::{session_request::Request as SessionRequest, IrnMetadata, Metadata, Relay,
                             RelayProtocolMetadata, RequestParams, ResponseParamsError, ResponseParamsSuccess};
use relay_rpc::rpc::{ErrorResponse, Payload, Request, Response, SuccessfulResponse};
use serde::de::DeserializeOwned;
use session::rpc::delete::send_session_delete_request;
use session::Session;
use session::{key::SymKeyPair, SessionManager};
use std::collections::{BTreeSet, HashMap};
use std::ops::Deref;
use std::{sync::{Arc, Mutex as SyncMutex},
          time::Duration};
use storage::SessionStorageDb;
use storage::WalletConnectStorageOps;
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

    async fn wc_chain_id(&self, ctx: &WalletConnectCtx) -> Result<WcChainId, Self::Error>;

    async fn wc_sign_tx<'a>(
        &self,
        ctx: &WalletConnectCtx,
        params: Self::Params<'a>,
    ) -> Result<Self::SignTxData, Self::Error>;

    async fn wc_send_tx<'a>(
        &self,
        ctx: &WalletConnectCtx,
        params: Self::Params<'a>,
    ) -> Result<Self::SendTxData, Self::Error>;
}

pub struct WalletConnectCtxImpl {
    pub(crate) client: Client,
    pub(crate) pairing: PairingClient,
    pub(crate) key_pair: SymKeyPair,
    pub session_manager: SessionManager,
    relay: Relay,
    metadata: Metadata,
    message_id_generator: MessageIdGenerator,
    pending_requests: SyncMutex<HashMap<MessageId, oneshot::Sender<SessionMessageType>>>,
    abortable_system: AbortableQueue,
}

pub struct WalletConnectCtx(pub Arc<WalletConnectCtxImpl>);
impl Deref for WalletConnectCtx {
    type Target = WalletConnectCtxImpl;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl WalletConnectCtx {
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
        let (inbound_message_tx, mut inbound_message_rx) = unbounded();
        let (conn_live_sender, conn_live_receiver) = unbounded();
        let (client, _) = Client::new_with_callback(
            Handler::new("Komodefi", inbound_message_tx, conn_live_sender),
            |r, h| abortable_system.weak_spawner().spawn(client_event_loop(r, h)),
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
            .spawn(context.clone().spawn_connection_initialization(conn_live_receiver));
        // spawn message handler event loop
        context.abortable_system.weak_spawner().spawn({
            let context = context.clone();
            async move {
                while let Some(msg) = inbound_message_rx.next().await {
                    if let Err(e) = context.handle_published_message(msg).await {
                        error!("Error processing message: {:?}", e);
                    }
                }
            }
        });

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
    pub(crate) async fn spawn_connection_initialization(
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

    pub(crate) async fn reconnect_and_subscribe(&self) -> MmResult<(), WalletConnectError> {
        self.connect_client().await?;
        // Resubscribes to previously active session topics after reconnection.
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
            Some(value) => Some(serde_json::from_value(value)?),
            None => None,
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
                Ok(Ok(_)) => {
                    info!("[{topic}] Subscribed to topic");
                    send_proposal_request(self, &topic, required_namespaces, optional_namespaces).await?;
                    return Ok(url);
                },
                Ok(Err(err)) => return MmError::err(err.into()),
                Err(_) => self.wait_until_client_is_online_loop(attempt).await,
            }
        }

        MmError::err(WalletConnectError::InternalError(
            "client connection timeout".to_string(),
        ))
    }

    /// Retrieves the symmetric key associated with a given `topic`.
    fn sym_key(&self, topic: &Topic) -> MmResult<SymKey, WalletConnectError> {
        if let Some(key) = self.session_manager.sym_key(topic) {
            return Ok(key);
        }

        if let Ok(key) = self.pairing.sym_key(topic) {
            return Ok(key);
        }

        MmError::err(WalletConnectError::InternalError(format!(
            "topic sym_key not found:{topic}"
        )))
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

        info!("[{}] Inbound message was handled successfully", msg.topic);

        Ok(())
    }

    /// Loads sessions from storage, activates valid ones, and deletes expired ones.
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
                if let Err(err) = self.session_manager.storage().delete_session(&session.topic).await {
                    error!("[{}] Unable to delete session from storage: {err:?}", session.topic);
                }
                continue;
            };

            let topic = session.topic.clone();
            let pairing_topic = session.pairing_topic.clone();
            debug!("[{topic}] Session found! activating");
            self.session_manager.add_session(session);

            valid_topics.push(topic);
            pairing_topics.push(pairing_topic);
        }

        let all_topics = valid_topics
            .into_iter()
            .chain(pairing_topics.into_iter())
            .collect::<Vec<_>>();

        if !all_topics.is_empty() {
            self.client.batch_subscribe(all_topics).await?;
        }

        Ok(())
    }

    /// function to publish a request.
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
        let mut pending_requests = self
            .pending_requests
            .lock()
            .expect("pending request lock shouldn't fail!");
        pending_requests.insert(message_id, tx);

        Ok((rx, Duration::from_secs(ttl)))
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
            .await
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
            .await
    }

    /// Private function to publish a payload.
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

    /// This persistent reconnection and retry strategy keeps the WebSocket connection active,
    /// allowing the client to automatically resume operations after network interruptions or disconnections.
    /// Since TCP handles connection timeouts (which can be lengthy), we're using a shorter timeout here
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
    pub fn is_ledger_connection(&self) -> bool {
        self.session_manager
            .get_session_active()
            .and_then(|session| session.session_properties)
            .and_then(|props| props.keys.as_ref().cloned())
            .and_then(|keys| keys.first().cloned())
            .map(|key| key.is_nano_ledger)
            .unwrap_or(false)
    }

    /// Checks if a given chain ID is supported.
    pub(crate) fn validate_chain_id(
        &self,
        session: &Session,
        chain_id: &WcChainId,
    ) -> MmResult<(), WalletConnectError> {
        println!("{:?}", session.namespaces.get(chain_id.chain.as_ref()));
        if let Some(Namespace {
            chains: Some(chains), ..
        }) = session.namespaces.get(chain_id.chain.as_ref())
        {
            if chains.contains(&chain_id.to_string()) {
                return Ok(());
            };
        }

        // https://specs.walletconnect.com/2.0/specs/clients/sign/namespaces#13-chains-might-be-omitted-if-the-caip-2-is-defined-in-the-index
        if session.namespaces.contains_key(&chain_id.to_string()) {
            return Ok(());
        }

        MmError::err(WalletConnectError::ChainIdNotSupported(chain_id.to_string()))
    }

    pub async fn validate_update_active_chain_id(&self, chain_id: &WcChainId) -> MmResult<(), WalletConnectError> {
        let session =
            self.session_manager
                .get_session_active()
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

    /// Retrieves the available account for a given chain ID.
    pub fn get_account_for_chain_id(&self, chain_id: &WcChainId) -> MmResult<String, WalletConnectError> {
        let namespaces = &self
            .session_manager
            .get_session_active()
            .ok_or(MmError::new(WalletConnectError::SessionError(
                "No active WalletConnect session found".to_string(),
            )))?
            .namespaces;

        if let Some(Namespace {
            accounts: Some(accounts),
            ..
        }) = namespaces.get(chain_id.chain.as_ref())
        {
            if let Some(account) = find_account_in_namespace(accounts, &chain_id.id) {
                return Ok(account);
            }
        };

        MmError::err(WalletConnectError::NoAccountFound(chain_id.to_string()))
    }

    /// Waits for and handles a WalletConnect session response with arbitrary data.
    /// https://specs.walletconnect.com/2.0/specs/clients/sign/session-events#session_request
    pub async fn send_session_request_and_wait<T, R, F>(
        &self,
        chain_id: &WcChainId,
        method: WcRequestMethods,
        params: serde_json::Value,
        callback: F,
    ) -> MmResult<R, WalletConnectError>
    where
        T: DeserializeOwned,
        F: Fn(T) -> MmResult<R, WalletConnectError>,
    {
        let active_topic = self.session_manager.get_active_topic_or_err()?;
        let request = SessionRequestRequest {
            chain_id: chain_id.to_string(),
            request: SessionRequest {
                method: method.as_ref().to_string(),
                expiry: None,
                params,
            },
        };
        let request = RequestParams::SessionRequest(request);
        let (rx, ttl) = self.publish_request(&active_topic, request).await?;

        let response = rx
            .timeout(ttl)
            .await
            .map_to_mm(|_| WalletConnectError::TimeoutError)?
            .map_to_mm(|err| WalletConnectError::InternalError(err.to_string()))??;
        match response.data {
            ResponseParamsSuccess::Arbitrary(data) => callback(serde_json::from_value::<T>(data)?),
            _ => MmError::err(WalletConnectError::PayloadError("Unexpected response type".to_string())),
        }
    }

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

mod error;
#[allow(unused)] mod websocket_client;
extern crate common;
extern crate serde;

use std::sync::{atomic::{AtomicU8, Ordering},
                Arc};

use error::RequestBuildError;
use http::Uri;
use relay_rpc::{auth::{SerializedAuthToken, RELAY_WEBSOCKET_ADDRESS},
                domain::{MessageId, ProjectId, SubscriptionId},
                rpc::{SubscriptionError, SubscriptionResult},
                user_agent::UserAgent};
use serde::Serialize;
use url::Url;

pub type HttpRequest<T> = ::http::Request<T>;

/// Relay authorization method. A wrapper around [`SerializedAuthToken`].
#[derive(Debug, Clone)]
pub enum Authorization {
    /// Uses query string to pass the auth token, e.g. `?auth=<token>`.
    Query(SerializedAuthToken),

    /// Uses the `Authorization: Bearer <token>` HTTP header.
    Header(SerializedAuthToken),
}

/// Relay connection options.
#[derive(Debug, Clone)]
pub struct ConnectionOptions {
    /// The Relay websocket address. The default address is
    /// `wss://relay.walletconnect.com`.
    pub address: String,

    /// The project-specific secret key. Can be generated in the Cloud Dashboard
    /// at the following URL: <https://cloud.walletconnect.com/app>
    pub project_id: ProjectId,

    /// The authorization method and auth token to use.
    pub auth: Authorization,

    /// Optional origin of the request. Subject to allow-list validation.
    pub origin: Option<String>,

    /// Optional user agent parameters.
    pub user_agent: Option<UserAgent>,
}

impl ConnectionOptions {
    pub fn new(project_id: impl Into<ProjectId>, auth: SerializedAuthToken) -> Self {
        Self {
            address: RELAY_WEBSOCKET_ADDRESS.into(),
            project_id: project_id.into(),
            auth: Authorization::Query(auth),
            origin: None,
            user_agent: None,
        }
    }

    pub fn set_address(mut self, address: impl Into<String>) -> Self {
        self.address = address.into();
        self
    }

    pub fn set_origin(mut self, origin: impl Into<Option<String>>) -> Self {
        self.origin = origin.into();
        self
    }

    pub fn set_user_agent(mut self, user_agent: impl Into<Option<UserAgent>>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn as_url(&self) -> Result<Url, RequestBuildError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct QueryParams<'a> {
            project_id: &'a ProjectId,
            auth: Option<&'a SerializedAuthToken>,
            ua: Option<&'a UserAgent>,
        }

        let query = serde_qs::to_string(&QueryParams {
            project_id: &self.project_id,
            auth: if let Authorization::Query(auth) = &self.auth {
                Some(auth)
            } else {
                None
            },
            ua: self.user_agent.as_ref(),
        })?;

        let url = {
            let mut url = Url::parse(&self.address).map_err(|err| RequestBuildError::Url(format!("{err:?}")))?;
            url.set_query(Some(&query));
            url
        };

        Ok(url)
    }

    #[allow(unused)]
    fn as_ws_request(&self) -> Result<HttpRequest<()>, RequestBuildError> {
        let url = self.as_url()?;
        let mut request = into_client_request(url.as_ref())?;
        {
            if let Authorization::Header(token) = &self.auth {
                let value = format!("Bearer {token}")
                    .parse()
                    .map_err(|_| RequestBuildError::Headers)?;

                request.headers_mut().append("Authorization", value);
            }

            if let Some(origin) = &self.origin {
                let value = origin.parse().map_err(|_| RequestBuildError::Headers)?;

                request.headers_mut().append("Origin", value);
            }
        };

        Ok(request)
    }
}

#[allow(unused)]
fn into_client_request(url: &str) -> Result<HttpRequest<()>, RequestBuildError> {
    let uri: Uri = url
        .parse()
        .map_err(|_| RequestBuildError::Url("Invalid url".to_owned()))?;
    let authority = uri
        .authority()
        .ok_or(RequestBuildError::Url("Url has not authority".to_owned()))?
        .as_str();
    let host = authority
        .find('@')
        .map(|idx| authority.split_at(idx + 1).1)
        .unwrap_or_else(|| authority);

    // Check if the host is empty (excluding the port)
    if host.split(':').next().unwrap_or("").is_empty() {
        return Err(RequestBuildError::Url("EmptyHostName".to_owned()));
    }

    let req = HttpRequest::builder()
        .method("GET")
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_websocket_key())
        .uri(uri)
        .body(())
        .map_err(|err| RequestBuildError::Url(err.to_string()))?;
    Ok(req)
}

/// Generate a random key for the `Sec-WebSocket-Key` header.
pub fn generate_websocket_key() -> String {
    // a base64-encoded (see Section 4 of [RFC4648]) value that,
    // when decoded, is 16 bytes in length (RFC 6455)
    let r: [u8; 16] = rand::random();
    data_encoding::BASE64.encode(&r)
}

/// Generates unique message IDs for use in RPC requests. Uses 56 bits for the
/// timestamp with millisecond precision, with the last 8 bits from a monotonic
/// counter. Capable of producing up to `256000` unique values per second.
#[derive(Debug, Clone)]
pub struct MessageIdGenerator {
    next: Arc<AtomicU8>,
}

impl MessageIdGenerator {
    pub fn new() -> Self { Self::default() }

    /// Generates a [`MessageId`].
    pub fn next(&self) -> MessageId {
        let next = self.next.fetch_add(1, Ordering::Relaxed) as u64;
        let timestamp = chrono::Utc::now().timestamp_millis() as u64;
        let id = timestamp << 8 | next;

        MessageId::new(id)
    }
}

impl Default for MessageIdGenerator {
    fn default() -> Self {
        Self {
            next: Arc::new(AtomicU8::new(0)),
        }
    }
}

#[allow(unused)]
#[inline]
fn convert_subscription_result(res: SubscriptionResult) -> Result<SubscriptionId, SubscriptionError> {
    match res {
        SubscriptionResult::Id(id) => Ok(id),
        SubscriptionResult::Error(_) => Err(SubscriptionError::SubscriberLimitExceeded),
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) mod wallet_connect_client_tests {
    use super::*;
    use common::log::info;
    use common::{self, executor::Timer};
    use error::ClientError;
    use http::header::{CONNECTION, HOST, SEC_WEBSOCKET_VERSION, UPGRADE};
    use mm2_core::mm_ctx::{MmArc, MmCtx};
    use relay_rpc::{auth::{ed25519_dalek::SigningKey, AuthToken},
                    domain::Topic};
    use std::time::Duration;
    use tokio_tungstenite_wasm::CloseFrame;
    use websocket_client::{Client, ConnectionHandler, PublishedMessage};

    pub(crate) fn test_as_ws_request_impl() {
        let key = SigningKey::generate(&mut rand::thread_rng());
        let auth = AuthToken::new("http://example.com")
            .aud("http://example.com")
            .ttl(Duration::from_secs(60 * 60))
            .as_jwt(&key)
            .unwrap();

        let project_id = ProjectId::from("test_project_id");
        let options = ConnectionOptions::new(project_id, auth.clone())
            .set_address("wss://test.example.com")
            .set_origin(Some("https://test.com".to_string()));

        // Call as_ws_request
        let request = options.as_ws_request().expect("Failed to create request");
        assert_eq!(
            request.uri().to_string(),
            format!(
                "wss://test.example.com/?projectId=test_project_id&auth={}",
                auth.to_string()
            )
        );

        // Check headers
        let headers = request.headers();
        assert_eq!(headers.get("Upgrade").unwrap(), "websocket");
        assert_eq!(headers.get("Connection").unwrap(), "Upgrade");
        assert_eq!(headers.get("Sec-WebSocket-Version").unwrap(), "13");
        assert!(headers.get("Sec-WebSocket-Key").is_some());
        assert_eq!(headers.get("Origin").unwrap(), "https://test.com");

        // Check that Authorization header is not present (using Query auth)
        assert!(headers.get("Authorization").is_none());

        // Test with Header authorization
        let options_with_header_auth = ConnectionOptions {
            auth: Authorization::Header(auth.clone()),
            ..options
        };
        let request_header_auth = options_with_header_auth
            .as_ws_request()
            .expect("Failed to create request");
        let headers_header_auth = request_header_auth.headers();
        // Check that Authorization header is present
        assert_eq!(
            headers_header_auth.get::<&str>("Authorization").unwrap().to_owned(),
            format!("Bearer {}", auth.to_string())
        );
        // Check that auth is not in URL for Header auth
        assert_eq!(
            request_header_auth.uri(),
            "wss://test.example.com/?projectId=test_project_id"
        );
    }

    pub(crate) fn test_into_client_request_impl() {
        // Test successful case
        let url = "wss://example.com/ws";
        let result = into_client_request(url);
        assert!(result.is_ok());
        let request = result.unwrap();
        assert_eq!(request.method(), "GET");
        assert_eq!(request.uri().to_string(), url);
        assert_eq!(request.headers().get(HOST).unwrap(), "example.com");
        assert_eq!(request.headers().get(CONNECTION).unwrap(), "Upgrade");
        assert_eq!(request.headers().get(UPGRADE).unwrap(), "websocket");
        assert_eq!(request.headers().get(SEC_WEBSOCKET_VERSION).unwrap(), "13");
        assert!(request.headers().contains_key("Sec-WebSocket-Key"));

        // Test URL with port
        let url_with_port = "wss://example.com:8080/ws";
        let result = into_client_request(url_with_port);
        assert!(result.is_ok());
        let request = result.unwrap();
        assert_eq!(request.headers().get(HOST).unwrap(), "example.com:8080");

        // Test URL with username and password
        let url_with_auth = "wss://user:pass@example.com/ws";
        let result = into_client_request(url_with_auth);
        assert!(result.is_ok());
        let request = result.unwrap();
        assert_eq!(request.headers().get(HOST).unwrap(), "example.com");

        // Test invalid URL
        let invalid_url = "not a valid url";
        let result = into_client_request(invalid_url);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RequestBuildError::Url(_)));

        // Test URL without authority
        let url_without_authority = "wss:///path";
        let result = into_client_request(url_without_authority);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RequestBuildError::Url(_)));

        // Test URL with empty host
        let url_empty_host = "wss://:8080/ws";
        let result = into_client_request(url_empty_host);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RequestBuildError::Url(_)));
    }

    struct Handler {
        name: &'static str,
    }

    impl Handler {
        fn new(name: &'static str) -> Self { Self { name } }
    }

    impl ConnectionHandler for Handler {
        fn connected(&mut self) {
            info!("[{}] connection open", self.name);
        }

        fn disconnected(&mut self, frame: Option<CloseFrame<'static>>) {
            info!("[{}] connection closed: frame={frame:?}", self.name);
        }

        fn message_received(&mut self, message: PublishedMessage) {
            info!(
                "[{}] inbound message: topic={} message={}",
                self.name, message.topic, message.message
            );
        }

        fn inbound_error(&mut self, error: ClientError) {
            info!("[{}] inbound error: {error}", self.name);
        }

        fn outbound_error(&mut self, error: ClientError) {
            info!("[{}] outbound error: {error}", self.name);
        }
    }

    pub(crate) async fn test_walletconnect_client_impl() {
        let ctx = MmArc::new(MmCtx::default());
        let key = SigningKey::generate(&mut rand::thread_rng());
        let auth = AuthToken::new("http://127.0.0.1:8000")
            .aud("wss://relay.walletconnect.com")
            .ttl(Duration::from_secs(60 * 60))
            .as_jwt(&key)
            .unwrap();
        let conn = ConnectionOptions::new("1979a8326eb123238e633655924f0a78", auth)
            .set_address("wss://relay.walletconnect.com");

        let client = Client::new(&ctx, Handler::new("client"));
        client.connect(&conn).await.unwrap();

        let topic = Topic::generate();
        let subscription_id = client.subscribe(topic.clone()).await.unwrap();
        info!("[client] subscribed: topic={topic} subscription_id={subscription_id}");

        let client2 = Client::new(&ctx, Handler::new("client2"));
        client2.connect(&conn).await.unwrap();

        Timer::sleep_ms(1000).await;
        client2
            .publish(
                topic.clone(),
                Arc::from("Hello WalletConnect!"),
                0,
                Duration::from_secs(60),
                false,
            )
            .await
            .unwrap();

        info!("[client2] published message with topic: {topic}",);

        Timer::sleep_ms(1000).await;
        drop(client);
        drop(client2);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_tests {
    use crate::wallet_connect_client_tests::{test_as_ws_request_impl, test_into_client_request_impl,
                                             test_walletconnect_client_impl};
    use common::block_on;

    #[test]
    fn test_as_ws_request() { test_as_ws_request_impl() }

    #[test]
    fn test_into_client_request() { test_into_client_request_impl() }

    #[test]
    fn test_walletconnect_client() { block_on(test_walletconnect_client_impl()) }
}

#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use super::*;
    use crate::wallet_connect_client_tests::{test_as_ws_request_impl, test_into_client_request_impl,
                                             test_walletconnect_client_impl};
    use common::log::wasm_log::register_wasm_log;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_as_ws_request() { test_as_ws_request_impl() }

    #[wasm_bindgen_test]
    fn test_into_client_request() {
        register_wasm_log();
        test_into_client_request_impl()
    }

    #[wasm_bindgen_test]
    async fn test_walletconnect_client() {
        register_wasm_log();
        test_walletconnect_client_impl().await
    }
}

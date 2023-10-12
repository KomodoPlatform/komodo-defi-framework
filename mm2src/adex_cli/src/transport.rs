use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use common::log::{error, warn};
use http::{HeaderMap, StatusCode};
use hyper_dangerous::get_hyper_client_dangerous;
use mm2_net::native_http::SlurpHttpClient;
use serde::{Deserialize, Serialize};

use crate::{error_anyhow, error_bail, warn_bail};

#[async_trait]
pub(super) trait Transport {
    async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<Result<OkT, ErrT>>
    where
        ReqT: Serialize + Send + Sync,
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a>;
}

pub(super) struct SlurpTransport {
    rpc_uri: String,
}

impl SlurpTransport {
    pub(super) fn new(rpc_uri: String) -> SlurpTransport { SlurpTransport { rpc_uri } }
}

#[async_trait]
impl Transport for SlurpTransport {
    async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<Result<OkT, ErrT>>
    where
        ReqT: Serialize + Send + Sync,
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a>,
    {
        let data = serde_json::to_string(&req)
            .map_err(|error| error_anyhow!("Failed to serialize data being sent: {error}"))?;
        let client = get_hyper_client_dangerous()?;
        match client.slurp_post_json(&self.rpc_uri, data).await {
            Err(error) => error_bail!("Failed to send json: {error}"),
            Ok(resp) => resp.process::<OkT, ErrT>(),
        }
    }
}

trait Response {
    fn process<OkT, ErrT>(self) -> Result<Result<OkT, ErrT>>
    where
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a>;
}

impl Response for (StatusCode, HeaderMap, Vec<u8>) {
    fn process<OkT, ErrT>(self) -> Result<Result<OkT, ErrT>>
    where
        OkT: for<'a> Deserialize<'a>,
        ErrT: for<'a> Deserialize<'a>,
    {
        let (status, _headers, data) = self;

        match status {
            StatusCode::OK => match serde_json::from_slice::<OkT>(&data) {
                Ok(resp_data) => Ok(Ok(resp_data)),
                Err(error) => {
                    let data = String::from_utf8(data)
                        .map_err(|error| error_anyhow!("Failed to get string from resp data: {error}"))?;
                    error_bail!("Failed to deserialize response from data: {data:?}, error: {error}")
                },
            },
            StatusCode::INTERNAL_SERVER_ERROR => match serde_json::from_slice::<ErrT>(&data) {
                Ok(resp_data) => Ok(Err(resp_data)),
                Err(error) => {
                    let data = String::from_utf8(data)
                        .map_err(|error| error_anyhow!("Failed to get string from resp data: {error}"))?;
                    error_bail!("Failed to deserialize response from data: {data:?}, error: {error}")
                },
            },
            _ => {
                warn_bail!("Bad http status: {status}, data: {data:?}")
            },
        }
    }
}

mod hyper_dangerous {

    use std::sync::Arc;
    use std::time::SystemTime;

    use hyper::{client::HttpConnector, Body, Client};
    use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
    use rustls::client::{ServerCertVerified, ServerCertVerifier};
    use rustls::{RootCertStore, DEFAULT_CIPHER_SUITES, DEFAULT_VERSIONS};

    use super::*;

    pub(super) fn get_hyper_client_dangerous() -> Result<Client<HttpsConnector<HttpConnector>>> {
        let mut config = rustls::ClientConfig::builder()
            .with_cipher_suites(DEFAULT_CIPHER_SUITES)
            .with_safe_default_kx_groups()
            .with_protocol_versions(DEFAULT_VERSIONS)
            .map_err(|error| error_anyhow!("Inconsistent cipher-suite/versions selected: {error}"))?
            .with_root_certificates(RootCertStore::empty())
            .with_no_client_auth();

        config
            .dangerous()
            .set_certificate_verifier(Arc::new(NoCertificateVerification {}));

        let https_connector = HttpsConnectorBuilder::default()
            .with_tls_config(config)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();

        Ok(Client::builder().build::<_, Body>(https_connector))
    }

    struct NoCertificateVerification {}

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _: &rustls::Certificate,
            _: &[rustls::Certificate],
            _: &rustls::ServerName,
            _: &mut dyn Iterator<Item = &[u8]>,
            _: &[u8],
            _: SystemTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
    }
}

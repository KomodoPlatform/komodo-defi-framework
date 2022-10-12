use common::APPLICATION_JSON;
pub use cosmrs::tendermint::abci::Path as AbciPath;
use cosmrs::tendermint::abci::Transaction;
use cosmrs::tendermint::block::Height;
use derive_more::Display;
use http::header::{ACCEPT, CONTENT_TYPE};
use http::uri::InvalidUri;
use http::{StatusCode, Uri};
use mm2_net::transport::SlurpError;
use mm2_net::wasm_http::FetchRequest;
use std::str::FromStr;
use tendermint_rpc::endpoint::{abci_info, broadcast};
pub use tendermint_rpc::endpoint::{abci_query::{AbciQuery, Request as AbciRequest},
                                   health::Request as HealthRequest,
                                   tx_search::Request as TxSearchRequest};
use tendermint_rpc::error::Error as TendermintRpcError;
pub use tendermint_rpc::query::Query as TendermintQuery;
use tendermint_rpc::request::SimpleRequest;
pub use tendermint_rpc::Order as TendermintResultOrder;
use tendermint_rpc::Response;

#[derive(Debug, Clone)]
pub(super) struct HttpClient {
    uri: String,
}

#[derive(Debug, Display)]
pub(super) enum HttpClientInitError {
    InvalidUri(InvalidUri),
}

impl From<InvalidUri> for HttpClientInitError {
    fn from(err: InvalidUri) -> Self { HttpClientInitError::InvalidUri(err) }
}

#[derive(Debug, Display)]
pub(super) enum PerformError {
    TendermintRpc(TendermintRpcError),
    Slurp(SlurpError),
    #[display(fmt = "Request failed with status code {}, response {}", status_code, response)]
    StatusCode {
        status_code: StatusCode,
        response: String,
    },
}

impl From<SlurpError> for PerformError {
    fn from(err: SlurpError) -> Self { PerformError::Slurp(err) }
}

impl From<TendermintRpcError> for PerformError {
    fn from(err: TendermintRpcError) -> Self { PerformError::TendermintRpc(err) }
}

impl HttpClient {
    pub(super) fn new(url: &str) -> Result<Self, HttpClientInitError> {
        Uri::from_str(url)?;
        Ok(HttpClient { uri: url.to_owned() })
    }

    pub(super) async fn perform<R>(&self, request: R) -> Result<R::Response, PerformError>
    where
        R: SimpleRequest,
    {
        let request_str = request.into_json();
        let (status_code, response_str) = FetchRequest::post(&self.uri)
            .cors()
            .body_utf8(request_str)
            .header(ACCEPT.as_str(), APPLICATION_JSON)
            .header(CONTENT_TYPE.as_str(), APPLICATION_JSON)
            .request_str()
            .await
            .map_err(|e| e.into_inner())?;
        if !status_code.is_success() {
            return Err(PerformError::StatusCode {
                status_code,
                response: response_str,
            });
        }
        Ok(R::Response::from_string(response_str)?)
    }

    /// `/abci_info`: get information about the ABCI application.
    pub async fn abci_info(&self) -> Result<abci_info::AbciInfo, PerformError> {
        Ok(self.perform(abci_info::Request).await?.response)
    }

    /// `/abci_query`: query the ABCI application
    pub async fn abci_query<V>(
        &self,
        path: Option<AbciPath>,
        data: V,
        height: Option<Height>,
        prove: bool,
    ) -> Result<AbciQuery, PerformError>
    where
        V: Into<Vec<u8>> + Send,
    {
        Ok(self
            .perform(AbciRequest::new(path, data, height, prove))
            .await?
            .response)
    }

    /// `/broadcast_tx_commit`: broadcast a transaction, returning the response
    /// from `DeliverTx`.
    pub async fn broadcast_tx_commit(&self, tx: Transaction) -> Result<broadcast::tx_commit::Response, PerformError> {
        self.perform(broadcast::tx_commit::Request::new(tx)).await
    }
}

mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_get_abci_info() {
        let client = HttpClient::new("https://cosmos-testnet-rpc.allthatnode.com:26657").unwrap();
        client.abci_info().await.unwrap();
    }
}

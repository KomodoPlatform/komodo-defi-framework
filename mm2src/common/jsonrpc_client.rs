use futures01::Future;
use itertools::Itertools;
use serde::de::DeserializeOwned;
use serde_json::{self as json, Value as Json};
use std::collections::{BTreeSet, HashMap};
use std::fmt;

/// Macro generating functions for RPC requests.
/// Args must implement/derive Serialize trait.
/// Generates params vector from input args, builds the request and sends it.
#[macro_export]
macro_rules! rpc_func {
    ($selff:ident, $method:expr $(, $arg_name:expr)*) => {{
        let request = $crate::rpc_req!($selff, $method $(, $arg_name)*);
        $selff.send_request(request)
    }}
}

/// Macro generating functions for RPC requests.
/// Sends the RPC request to specified remote endpoint using the passed address.
/// Args must implement/derive Serialize trait.
/// Generates params vector from input args, builds the request and sends it.
#[macro_export]
macro_rules! rpc_func_from {
    ($selff:ident, $address:expr, $method:expr $(, $arg_name:expr)*) => {{
        let request = $crate::rpc_req!($selff, $method $(, $arg_name)*);
        $selff.send_request_to($address, request)
    }}
}

/// Macro generating functions for RPC requests.
/// Args must implement/derive Serialize trait.
/// Generates params vector from input args, builds the `JsonRpcRequest` request.
#[macro_export]
macro_rules! rpc_req {
    ($selff:ident, $method:expr $(, $arg_name:expr)*) => {{
        let mut params = vec![];
        $(
            params.push(json::value::to_value($arg_name).unwrap());
        )*
        JsonRpcRequest {
            jsonrpc: $selff.version().into(),
            id: $selff.next_id(),
            method: $method.into(),
            params
        }
    }}
}

pub type JsonRpcResponseFut =
    Box<dyn Future<Item = (JsonRpcRemoteAddr, JsonRpcResponseEnum), Error = String> + Send + 'static>;
pub type RpcRes<T> = Box<dyn Future<Item = T, Error = JsonRpcError> + Send + 'static>;

/// Address of server from which an Rpc response was received
#[derive(Clone, Default)]
pub struct JsonRpcRemoteAddr(pub String);

impl fmt::Debug for JsonRpcRemoteAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}

impl From<JsonRpcRemoteAddr> for String {
    fn from(addr: JsonRpcRemoteAddr) -> Self { addr.0 }
}

impl From<String> for JsonRpcRemoteAddr {
    fn from(addr: String) -> Self { JsonRpcRemoteAddr(addr) }
}

/// The identifier is designed to uniquely match outgoing requests and incoming responses.
/// Even if the batch response is sorted in a different order, `BTreeSet<Id>` allows it to be matched to the request.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum JsonRpcId {
    Single(String),
    Batch(BTreeSet<String>),
}

/// Serializable RPC request that is either single or batch.
#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum JsonRpcRequestEnum {
    Single(JsonRpcRequest),
    Batch(JsonRpcBatchRequest),
}

impl JsonRpcRequestEnum {
    /// Creates [`JsonRpcRequestEnum::Batch`] from the given `requests`.
    pub fn new_batch(requests: Vec<JsonRpcRequest>) -> JsonRpcRequestEnum {
        JsonRpcRequestEnum::Batch(JsonRpcBatchRequest(requests))
    }

    /// Returns a `JsonRpcId` identifier of the request.
    pub fn rpc_id(&self) -> JsonRpcId {
        match self {
            JsonRpcRequestEnum::Single(single) => single.rpc_id(),
            JsonRpcRequestEnum::Batch(batch) => batch.rpc_id(),
        }
    }
}

impl fmt::Debug for JsonRpcRequestEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonRpcRequestEnum::Single(single) => single.fmt(f),
            JsonRpcRequestEnum::Batch(batch) => batch.fmt(f),
        }
    }
}

/// Serializable RPC single request.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: String,
    pub method: String,
    pub params: Vec<Json>,
}

impl JsonRpcRequest {
    // Returns [`JsonRpcRequest::id`].
    pub fn get_id(&self) -> &str { &self.id }

    /// Returns a `JsonRpcId` identifier of the request.
    pub fn rpc_id(&self) -> JsonRpcId { JsonRpcId::Single(self.id.clone()) }
}

impl From<JsonRpcRequest> for JsonRpcRequestEnum {
    fn from(single: JsonRpcRequest) -> Self { JsonRpcRequestEnum::Single(single) }
}

/// Serializable RPC batch request.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JsonRpcBatchRequest(Vec<JsonRpcRequest>);

impl JsonRpcBatchRequest {
    /// Returns a `JsonRpcId` identifier of the request.
    pub fn rpc_id(&self) -> JsonRpcId { JsonRpcId::Batch(self.orig_sequence_ids().collect()) }

    /// Returns the number of the requests in the batch.
    pub fn len(&self) -> usize { self.0.len() }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool { self.0.is_empty() }

    /// Returns original sequence of identifiers.
    /// The method is used to process batch responses in the same order in which the requests were sent.
    fn orig_sequence_ids(&self) -> impl Iterator<Item = String> + '_ { self.0.iter().map(|req| req.id.clone()) }
}

impl From<JsonRpcBatchRequest> for JsonRpcRequestEnum {
    fn from(batch: JsonRpcBatchRequest) -> Self { JsonRpcRequestEnum::Batch(batch) }
}

/// Deserializable RPC response that is either single or batch.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponseEnum {
    Single(JsonRpcResponse),
    Batch(JsonRpcBatchResponse),
}

impl JsonRpcResponseEnum {
    /// Returns a `JsonRpcId` identifier of the response.
    pub fn rpc_id(&self) -> JsonRpcId {
        match self {
            JsonRpcResponseEnum::Single(single) => single.rpc_id(),
            JsonRpcResponseEnum::Batch(batch) => batch.rpc_id(),
        }
    }
}

/// Deserializable RPC single response.
#[derive(Clone, Debug, Deserialize)]
pub struct JsonRpcResponse {
    #[serde(default)]
    pub jsonrpc: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub result: Json,
    #[serde(default)]
    pub error: Json,
}

impl JsonRpcResponse {
    /// Returns a `JsonRpcId` identifier of the response.
    pub fn rpc_id(&self) -> JsonRpcId { JsonRpcId::Single(self.id.clone()) }
}

/// Deserializable RPC batch response.
#[derive(Clone, Debug, Deserialize)]
pub struct JsonRpcBatchResponse(Vec<JsonRpcResponse>);

impl JsonRpcBatchResponse {
    /// Returns a `JsonRpcId` identifier of the response.
    pub fn rpc_id(&self) -> JsonRpcId { JsonRpcId::Batch(self.0.iter().map(|res| res.id.clone()).collect()) }

    /// Returns the number of the requests in the batch.
    pub fn len(&self) -> usize { self.0.len() }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
}

impl IntoIterator for JsonRpcBatchResponse {
    type Item = JsonRpcResponse;
    type IntoIter = std::vec::IntoIter<JsonRpcResponse>;

    fn into_iter(self) -> Self::IntoIter { self.0.into_iter() }
}

#[derive(Clone, Debug)]
pub struct JsonRpcError {
    /// Additional member contains an instance info that implements the JsonRpcClient trait.
    /// The info is used in particular to supplement the error info.
    pub client_info: String,
    /// Source Rpc request.
    pub request: JsonRpcRequestEnum,
    /// Error type.
    pub error: JsonRpcErrorType,
}

impl fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "{:?}", self) }
}

#[derive(Clone, Debug)]
pub enum JsonRpcErrorType {
    /// Invalid outgoing request error
    InvalidRequest(String),
    /// Error from transport layer
    Transport(String),
    /// Response parse error
    Parse(JsonRpcRemoteAddr, String),
    /// The JSON-RPC error returned from server
    Response(JsonRpcRemoteAddr, Json),
}

impl JsonRpcErrorType {
    /// Whether the error type is [`JsonRpcErrorType::Transport`].
    pub fn is_transport(&self) -> bool { matches!(self, JsonRpcErrorType::Transport(_)) }
}

pub trait JsonRpcClient {
    /// Returns a stringified version of the JSON-RPC protocol.
    fn version(&self) -> &'static str;

    /// Returns a stringified identifier of the next request.
    fn next_id(&self) -> String;

    /// Get info that is used in particular to supplement the error info
    fn client_info(&self) -> String;

    /// Sends the given `request` to the remote.
    /// Returns either an address `JsonRpcRemoteAddr` of the responder and the `JsonRpcResponseEnum` response,
    /// or a stringified error.
    fn transport(&self, request: JsonRpcRequestEnum) -> JsonRpcResponseFut;

    /// Sends the given single `request` to the remote and tries to decode the response into `T`.
    fn send_request<T: DeserializeOwned + Send + 'static>(&self, request: JsonRpcRequest) -> RpcRes<T> {
        let client_info = self.client_info();
        Box::new(
            self.transport(JsonRpcRequestEnum::Single(request.clone()))
                .then(move |result| process_transport_single_result(result, client_info, request)),
        )
    }
}

pub trait JsonRpcBatchClient: JsonRpcClient {
    /// Sends the RPC batch request.
    /// Responses are guaranteed to be in the same order in which they were requested.
    fn batch_rpc<I, T>(&self, batch_requests: I) -> RpcRes<Vec<T>>
    where
        I: IntoIterator<Item = JsonRpcRequest>,
        T: DeserializeOwned + Send + 'static,
    {
        let requests: Vec<_> = batch_requests.into_iter().collect();
        if requests.is_empty() {
            // Return an empty vector of results.
            return Box::new(futures01::future::ok(Vec::new()));
        }
        self.send_batch_request(JsonRpcBatchRequest(requests))
    }

    /// Sends the given batch `request` to the remote and tries to decode the batch response into `Vec<T>`.
    /// Responses are guaranteed to be in the same order in which they were requested.
    fn send_batch_request<T: DeserializeOwned + Send + 'static>(&self, request: JsonRpcBatchRequest) -> RpcRes<Vec<T>> {
        try_fu!(self.validate_batch_request(&request));
        let client_info = self.client_info();
        Box::new(
            self.transport(JsonRpcRequestEnum::Batch(request.clone()))
                .then(move |result| process_transport_batch_result(result, client_info, request)),
        )
    }

    /// Validates the given batch requests if they all have unique IDs.
    fn validate_batch_request(&self, request: &JsonRpcBatchRequest) -> Result<(), JsonRpcError> {
        if request.orig_sequence_ids().all_unique() {
            return Ok(());
        }
        Err(JsonRpcError {
            client_info: self.client_info(),
            request: request.clone().into(),
            error: JsonRpcErrorType::InvalidRequest(ERRL!("Each request in a batch must have a unique ID")),
        })
    }
}

/// The trait is used when the rpc client instance has more than one remote endpoints.
pub trait JsonRpcMultiClient: JsonRpcClient {
    /// Sends the given `request` to the specified `to_addr` remote.
    /// Returns either an address `JsonRpcRemoteAddr` of the responder and the `JsonRpcResponseEnum` response,
    /// or a stringified error.
    fn transport_exact(&self, to_addr: String, request: JsonRpcRequestEnum) -> JsonRpcResponseFut;

    /// Sends the given single `request` to the specified `to_addr` remote and tries to decode the response into `T`.
    fn send_request_to<T: DeserializeOwned + Send + 'static>(
        &self,
        to_addr: &str,
        request: JsonRpcRequest,
    ) -> RpcRes<T> {
        let client_info = self.client_info();
        Box::new(
            self.transport_exact(to_addr.to_owned(), JsonRpcRequestEnum::Single(request.clone()))
                .then(move |result| process_transport_single_result(result, client_info, request)),
        )
    }
}

/// Checks if the given `result` is success and contains `JsonRpcResponse`.
/// Tries to decode the batch response into `T`.
fn process_transport_single_result<T: DeserializeOwned + Send + 'static>(
    result: Result<(JsonRpcRemoteAddr, JsonRpcResponseEnum), String>,
    client_info: String,
    request: JsonRpcRequest,
) -> Result<T, JsonRpcError> {
    let request = JsonRpcRequestEnum::Single(request);

    match result {
        Ok((remote_addr, JsonRpcResponseEnum::Single(single))) => {
            process_single_response(client_info, remote_addr, request, single)
        },
        Ok((remote_addr, JsonRpcResponseEnum::Batch(batch))) => {
            let error = ERRL!("Expeced single response, found batch response: {:?}", batch);
            Err(JsonRpcError {
                client_info,
                request,
                error: JsonRpcErrorType::Parse(remote_addr, error),
            })
        },
        Err(e) => Err(JsonRpcError {
            client_info,
            request,
            error: JsonRpcErrorType::Transport(e),
        }),
    }
}

/// Checks if the given `result` is success and contains `JsonRpcBatchResponse`.
/// Tries to decode the batch response into `Vec<T>` in the same order in which they were requested.
fn process_transport_batch_result<T: DeserializeOwned + Send + 'static>(
    result: Result<(JsonRpcRemoteAddr, JsonRpcResponseEnum), String>,
    client_info: String,
    request: JsonRpcBatchRequest,
) -> Result<Vec<T>, JsonRpcError> {
    let orig_ids: Vec<_> = request.orig_sequence_ids().collect();
    let request = JsonRpcRequestEnum::Batch(request);

    let (remote_addr, batch) = match result {
        Ok((remote_addr, JsonRpcResponseEnum::Batch(batch))) => (remote_addr, batch),
        Ok((remote_addr, JsonRpcResponseEnum::Single(single))) => {
            let error = ERRL!("Expected batch response, found single response: {:?}", single);
            return Err(JsonRpcError {
                client_info,
                request,
                error: JsonRpcErrorType::Parse(remote_addr, error),
            });
        },
        Err(e) => {
            return Err(JsonRpcError {
                client_info,
                request,
                error: JsonRpcErrorType::Transport(e),
            })
        },
    };

    // Turn the vector of responses into a hashmap by their IDs to get quick access to the content of the responses.
    let mut response_map: HashMap<String, JsonRpcResponse> =
        batch.into_iter().map(|res| (res.id.clone(), res)).collect();
    if response_map.len() != orig_ids.len() {
        let error = ERRL!(
            "Expected '{}' elements in batch response, found '{}'",
            orig_ids.len(),
            response_map.len()
        );
        return Err(JsonRpcError {
            client_info,
            request,
            error: JsonRpcErrorType::Parse(remote_addr, error),
        });
    }

    let mut result = Vec::with_capacity(orig_ids.len());
    for id in orig_ids.iter() {
        let single_resp = match response_map.remove(id) {
            Some(res) => res,
            None => {
                let error = ERRL!("Batch response doesn't contain '{}' identifier", id);
                return Err(JsonRpcError {
                    client_info,
                    request,
                    error: JsonRpcErrorType::Parse(remote_addr, error),
                });
            },
        };

        result.push(process_single_response(
            client_info.clone(),
            remote_addr.clone(),
            request.clone(),
            single_resp,
        )?);
    }
    Ok(result)
}

/// Tries to decode the given single `response` into `T` if it doesn't contain an error,
/// otherwise returns `JsonRpcError`.
fn process_single_response<T: DeserializeOwned + Send + 'static>(
    client_info: String,
    remote_addr: JsonRpcRemoteAddr,
    request: JsonRpcRequestEnum,
    response: JsonRpcResponse,
) -> Result<T, JsonRpcError> {
    if !response.error.is_null() {
        return Err(JsonRpcError {
            client_info,
            request,
            error: JsonRpcErrorType::Response(remote_addr, response.error),
        });
    }

    json::from_value(response.result.clone()).map_err(|e| JsonRpcError {
        client_info,
        request,
        error: JsonRpcErrorType::Parse(
            remote_addr,
            ERRL!("error {:?} parsing result from response {:?}", e, response),
        ),
    })
}

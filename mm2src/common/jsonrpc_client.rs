use futures::Future;
use serde::de::DeserializeOwned;
use serde_json::{self as json, Value as Json};
use std::fmt;

/// Macro generating functions for RPC requests.
/// Args must implement/derive Serialize trait.
/// Generates params vector from input args, builds the request and sends it.
#[macro_export]
macro_rules! rpc_func {
    ($selff:ident, $method:expr $(, $arg_name:ident)*) => {{
        let mut params = vec![];
        $(
            params.push(unwrap!(json::value::to_value($arg_name)));
        )*
        let request = JsonRpcRequest {
            jsonrpc: $selff.version().into(),
            id: $selff.next_id(),
            method: $method.into(),
            params
        };
        $selff.send_request(request)
    }}
}

/// Serializable RPC request
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: String,
    pub method: String,
    pub params: Vec<Json>,
}

impl JsonRpcRequest {
    pub fn get_id(&self) -> &str {
        &self.id
    }
}

#[derive(Deserialize, Debug, Clone)]
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

#[derive(Debug)]
pub struct RequestError {
    request: JsonRpcRequest,
    pub error: Json,
}

#[derive(Debug)]
pub enum JsonRpcError {
    Transport(String),
    Request(RequestError),
    Parse(String),
}

impl fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub type JsonRpcResponseFut = Box<Future<Item=JsonRpcResponse, Error=String> + Send + 'static>;
pub type RpcRes<T> = Box<Future<Item=T, Error=JsonRpcError> + Send + 'static>;

pub trait JsonRpcClient {
    fn version(&self) -> &'static str;

    fn next_id(&self) -> String;

    fn transport(&self, request: JsonRpcRequest) -> JsonRpcResponseFut;

    fn send_request<T: DeserializeOwned + Send + 'static>(&self, request: JsonRpcRequest) -> RpcRes<T> {
        Box::new(self.transport(request.clone()).map_err(|e| JsonRpcError::Transport(e)).and_then(move |response| -> Result<T, JsonRpcError> {
            if !response.error.is_null() {
                return Err(JsonRpcError::Request(RequestError {
                    request,
                    error: response.error,
                }));
            }

            match json::from_value(response.result.clone()) {
                Ok(res) => Ok(res),
                Err(e) => Err(JsonRpcError::Parse(ERRL!("Request {:?} error {:?} parsing result from response {:?}", request, e, response))),
            }
        }))
    }
}

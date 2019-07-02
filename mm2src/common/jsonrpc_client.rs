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
pub struct  JsonRpcError {
    request: JsonRpcRequest,
    pub error: JsonRpcErrorType,
}

#[derive(Debug)]
pub enum JsonRpcErrorType {
    /// Error from transport layer
    Transport(String),
    /// Response parse error
    Parse(String),
    /// The JSON-RPC error returned from server
    Response(Json)
}

impl fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub type JsonRpcResponseFut = Box<dyn Future<Item=JsonRpcResponse, Error=String> + Send + 'static>;
pub type RpcRes<T> = Box<dyn Future<Item=T, Error=JsonRpcError> + Send + 'static>;

pub trait JsonRpcClient {
    fn version(&self) -> &'static str;

    fn next_id(&self) -> String;

    fn transport(&self, request: JsonRpcRequest) -> JsonRpcResponseFut;

    fn send_request<T: DeserializeOwned + Send + 'static>(&self, request: JsonRpcRequest) -> RpcRes<T> {
        let request_f = self.transport(request.clone()).map_err({
            let request = request.clone();
            move |e| JsonRpcError {
                request,
                error: JsonRpcErrorType::Transport(e)
            }
        });
        Box::new(request_f.and_then(move |response| -> Result<T, JsonRpcError> {
            if !response.error.is_null() {
                return Err(JsonRpcError {
                    request,
                    error: JsonRpcErrorType::Response(response.error),
                });
            }

            match json::from_value(response.result.clone()) {
                Ok(res) => Ok(res),
                Err(e) => Err(JsonRpcError {
                    request,
                    error: JsonRpcErrorType::Parse(ERRL!("error {:?} parsing result from response {:?}", e, response)),
                }),
            }
        }))
    }
}

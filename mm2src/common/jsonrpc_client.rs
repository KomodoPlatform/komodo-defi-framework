use futures01::Future;
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

/// Address of server from which an Rpc response was received
#[derive(Default)]
pub struct JsonRpcRemoteAddr(pub String);

impl fmt::Debug for JsonRpcRemoteAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<JsonRpcRemoteAddr> for String {
    fn from(addr: JsonRpcRemoteAddr) -> Self {
        addr.0
    }
}

impl From<String> for JsonRpcRemoteAddr {
    fn from(addr: String) -> Self {
        JsonRpcRemoteAddr(addr)
    }
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
    /// Additional member contains an instance info that implements the JsonRpcClient trait.
    /// The info is used in particular to supplement the error info.
    client_info: String,
    /// Source Rpc request.
    request: JsonRpcRequest,
    /// Error type.
    pub error: JsonRpcErrorType,
}

#[derive(Debug)]
pub enum JsonRpcErrorType {
    /// Error from transport layer
    Transport(String),
    /// Response parse error
    Parse(JsonRpcRemoteAddr, String),
    /// The JSON-RPC error returned from server
    Response(JsonRpcRemoteAddr, Json)
}

impl fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub type JsonRpcResponseFut = Box<dyn Future<Item=(JsonRpcRemoteAddr, JsonRpcResponse), Error=String> + Send + 'static>;
pub type RpcRes<T> = Box<dyn Future<Item=T, Error=JsonRpcError> + Send + 'static>;

pub trait JsonRpcClient {
    fn version(&self) -> &'static str;

    fn next_id(&self) -> String;

    /// Get info that is used in particular to supplement the error info
    fn client_info(&self) -> String;

    fn transport(&self, request: JsonRpcRequest) -> JsonRpcResponseFut;

    fn send_request<T: DeserializeOwned + Send + 'static>(&self, request: JsonRpcRequest) -> RpcRes<T> {
        let client_info = self.client_info();
        let request_f = self.transport(request.clone()).map_err({
            let client_info = client_info.clone();
            let request = request.clone();
            move |e| JsonRpcError {
                client_info,
                request,
                error: JsonRpcErrorType::Transport(e)
            }
        });
        Box::new(request_f.and_then(move |(addr, response)| -> Result<T, JsonRpcError> {
            if !response.error.is_null() {
                return Err(JsonRpcError {
                    client_info,
                    request,
                    error: JsonRpcErrorType::Response(addr, response.error),
                });
            }

            match json::from_value(response.result.clone()) {
                Ok(res) => Ok(res),
                Err(e) => Err(JsonRpcError {
                    client_info,
                    request,
                    error: JsonRpcErrorType::Parse(addr, ERRL!("error {:?} parsing result from response {:?}", e, response)),
                }),
            }
        }))
    }
}

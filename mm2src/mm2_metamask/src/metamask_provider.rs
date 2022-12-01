use crate::eth_provider::EthProvider;
use crate::metamask_error::{MetamaskError, MetamaskResult};
use futures::lock::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use itertools::Itertools;
use mm2_err_handle::prelude::*;
use serde::Serialize;
use serde_derive::Deserialize;
use serde_json::Value as Json;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// `MetamaskProvider` is designed the way that there can be only one active session at the moment.
/// This is highly unlikely that the channel will be full with `capacity = 1024` during this session.
const ETH_COMMAND_CHANNEL_CAPACITY: usize = 1024;
const EIP712_DOMAIN: &str = "EIP712Domain";

macro_rules! eth_rpc_await {
    ($selff:ident, $method:expr $(, $arg_name:expr)*) => {{
        let params = vec![
            $(
                serde_json::value::to_value($arg_name)
                    .map_to_mm(|e| MetamaskError::ErrorSerializingArguments(e.to_string()))?
            ),*
        ];
        $selff
            .eth_provider
            .invoke_method($method.to_string(), params)
            .await
            .mm_err(MetamaskError::from)
    }}
}

#[derive(Clone)]
pub struct MetamaskProvider {
    eth_provider: Arc<AsyncMutex<EthProvider>>,
}

impl MetamaskProvider {
    pub fn detect_metamask_provider() -> MetamaskResult<MetamaskProvider> {
        let eth_provider = EthProvider::detect_ethereum_provider(ETH_COMMAND_CHANNEL_CAPACITY)
            .or_mm_err(|| MetamaskError::EthProviderNotFound)?;
        Ok(MetamaskProvider {
            eth_provider: Arc::new(AsyncMutex::new(eth_provider)),
        })
    }

    /// Creates a session that can be used to invoke methods.
    /// We need to limit the number of concurrent requests to one.
    pub async fn session(&self) -> MetamaskSession<'_> {
        let eth_provider = self.eth_provider.lock().await;
        MetamaskSession { eth_provider }
    }
}

pub struct MetamaskSession<'a> {
    eth_provider: AsyncMutexGuard<'a, EthProvider>,
}

impl<'a> MetamaskSession<'a> {
    /// Invokes an arbitrary RPC method.
    /// [`MetamaskSession::eth_request`] is expected to be used within a Web3Transport as a plug.
    ///
    /// Please consider adding new methods or using existing ones
    /// if you have a direct access to a `MetamaskSession` instance.
    ///
    /// See the list of available RPCs:
    /// https://ethereum.org/en/developers/docs/apis/json-rpc/
    pub async fn eth_request(&mut self, method: String, params: Vec<Json>) -> MetamaskResult<Json> {
        self.eth_provider
            .invoke_method(method, params)
            .await
            .mm_err(MetamaskError::from)
    }

    /// Invokes the `eth_requestAccounts` method.
    /// https://docs.metamask.io/guide/rpc-api.html#restricted-methods
    pub async fn eth_request_accounts(&mut self) -> MetamaskResult<EthAccount> {
        let accounts: Vec<String> = eth_rpc_await!(self, "eth_requestAccounts")?;
        accounts
            .into_iter()
            .exactly_one()
            .map(|address| EthAccount { address })
            .map_to_mm(|_| MetamaskError::ExpectedOneEthAccount)
    }

    /// * user_address - Must match user's active address.
    /// * types - Defines the types of the domain and data you will be signing.
    /// * domain - Ensures that the signature will be unique across multiple DApps and across Blockchains.
    /// * sign_data - The message signing data content.
    /// * primary_type - name of the `sign_data` structured type.
    pub async fn sign_typed_data_v4<Domain, SignData>(
        &mut self,
        user_address: String,
        types: &[ObjectType],
        domain: Domain,
        sign_data: SignData,
        primary_type: String,
    ) -> MetamaskResult<String>
    where
        Domain: Serialize,
        SignData: Serialize,
    {
        let types = types
            .iter()
            .map(|object_type| (object_type.name.as_str(), object_type.properties.as_slice()))
            .collect();

        let req = SignTypedDataV4Request {
            types,
            domain,
            primary_type,
            message: sign_data,
        };

        eth_rpc_await!(self, "eth_signTypedDataV4", user_address, req)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct EthAccount {
    pub address: String,
}

/// `ObjectType` is used to describes an object type accordingly to:
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-712.md#definition-of-typed-structured-data-%F0%9D%95%8A
///
/// # Example
///
/// Let's you need to describe the following types:
///
/// ```rust
/// struct Mail {
///   message: String,
///   from: Person,
///   to: Vec<Person>,
/// }
///
/// struct Person {
///   address: String,
/// }
/// ```
///
/// They can be described as follows:
///
/// ```rust
/// let mut mail_type = ObjectType::new("Mail");
/// mail_type.property("message", PropertyType::String);
/// mail_type.property("from", PropertyType::Custom("Person"));
/// mail_type.property_array("to", PropertyType::Custom("Person"));
///
/// let mut person_type = ObjectType::new("Person");
/// person_type.property("address", PropertyType::Address);
///
/// let types = vec![mail_type, person_type];
/// ```
pub struct ObjectType {
    name: String,
    properties: Vec<ObjectProperty>,
}

impl ObjectType {
    /// Creates an `ObjectType` with the `EIP712Domain` name
    /// (required to be set for a domain typed structure).
    pub fn domain() -> ObjectType {
        ObjectType {
            name: EIP712_DOMAIN.to_string(),
            properties: Vec::new(),
        }
    }

    /// Creates an `ObjectType` with a custom `name`.
    pub fn new(name: &str) -> ObjectType {
        ObjectType {
            name: name.to_string(),
            properties: Vec::new(),
        }
    }

    /// Describes a property.
    pub fn property(&mut self, property_name: &str, property_type: PropertyType) -> &mut ObjectType {
        let property = ObjectProperty {
            name: property_name.to_string(),
            r#type: property_type.to_string(),
        };
        self.properties.push(property);
        self
    }

    /// Describes an array property.
    pub fn property_array(&mut self, property_name: &str, property_type: PropertyType) -> &mut ObjectType {
        let property = ObjectProperty {
            name: property_name.to_string(),
            r#type: format!("{property_type}[]"),
        };
        self.properties.push(property);
        self
    }
}

/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-712.md#definition-of-typed-structured-data-%F0%9D%95%8A
#[derive(Debug)]
pub enum PropertyType {
    Bool,
    String,
    Int64,
    Uint64,
    Int256,
    Uint256,
    Address,
    Bytes32,
    Custom(String),
}

impl fmt::Display for PropertyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyType::Bool => write!(f, "bool"),
            PropertyType::String => write!(f, "string"),
            PropertyType::Int64 => write!(f, "int64"),
            PropertyType::Uint64 => write!(f, "uint64"),
            PropertyType::Int256 => write!(f, "int256"),
            PropertyType::Uint256 => write!(f, "uint256"),
            PropertyType::Address => write!(f, "address"),
            PropertyType::Bytes32 => write!(f, "bytes32"),
            PropertyType::Custom(custom) => write!(f, "{custom}"),
        }
    }
}

#[derive(Debug, Serialize)]
struct ObjectProperty {
    name: String,
    r#type: String,
}

type ObjectPropertiesRef<'a> = &'a [ObjectProperty];

#[derive(Debug, Serialize)]
struct SignTypedDataV4Request<'a, Domain, SignData> {
    types: HashMap<&'a str, ObjectPropertiesRef<'a>>,
    domain: Domain,
    #[serde(rename = "primaryType")]
    primary_type: String,
    message: SignData,
}

use super::*;
#[cfg(target_arch = "wasm32")] use crate::EthMetamaskPolicy;
use common::executor::AbortedError;
use crypto::{CryptoCtxError, StandardHDCoinAddress};
use enum_derives::EnumFromTrait;
use instant::Instant;
use mm2_err_handle::common_errors::WithInternal;
#[cfg(target_arch = "wasm32")]
use mm2_metamask::{from_metamask_error, MetamaskError, MetamaskRpcError, WithMetamaskRpcError};
use web3_transport::websocket_transport::WebsocketTransport;

#[derive(Clone, Debug, Deserialize, Display, EnumFromTrait, PartialEq, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EthActivationV2Error {
    InvalidPayload(String),
    InvalidSwapContractAddr(String),
    InvalidFallbackSwapContract(String),
    #[display(fmt = "Expected either 'chain_id' or 'rpc_chain_id' to be set")]
    #[cfg(target_arch = "wasm32")]
    ExpectedRpcChainId,
    #[display(fmt = "Platform coin {} activation failed. {}", ticker, error)]
    ActivationFailed {
        ticker: String,
        error: String,
    },
    CouldNotFetchBalance(String),
    UnreachableNodes(String),
    #[display(fmt = "Enable request for ETH coin must have at least 1 node")]
    AtLeastOneNodeRequired,
    #[display(fmt = "Error deserializing 'derivation_path': {}", _0)]
    ErrorDeserializingDerivationPath(String),
    PrivKeyPolicyNotAllowed(PrivKeyPolicyNotAllowed),
    #[display(fmt = "Failed spawning balance events. Error: {_0}")]
    FailedSpawningBalanceEvents(String),
    #[cfg(target_arch = "wasm32")]
    #[from_trait(WithMetamaskRpcError::metamask_rpc_error)]
    #[display(fmt = "{}", _0)]
    MetamaskError(MetamaskRpcError),
    #[from_trait(WithInternal::internal)]
    #[display(fmt = "Internal: {}", _0)]
    InternalError(String),
}

impl From<MyAddressError> for EthActivationV2Error {
    fn from(err: MyAddressError) -> Self { Self::InternalError(err.to_string()) }
}

impl From<AbortedError> for EthActivationV2Error {
    fn from(e: AbortedError) -> Self { EthActivationV2Error::InternalError(e.to_string()) }
}

impl From<CryptoCtxError> for EthActivationV2Error {
    fn from(e: CryptoCtxError) -> Self { EthActivationV2Error::InternalError(e.to_string()) }
}

impl From<UnexpectedDerivationMethod> for EthActivationV2Error {
    fn from(e: UnexpectedDerivationMethod) -> Self { EthActivationV2Error::InternalError(e.to_string()) }
}

#[cfg(target_arch = "wasm32")]
impl From<MetamaskError> for EthActivationV2Error {
    fn from(e: MetamaskError) -> Self { from_metamask_error(e) }
}

/// An alternative to `crate::PrivKeyActivationPolicy`, typical only for ETH coin.
#[derive(Clone, Deserialize)]
pub enum EthPrivKeyActivationPolicy {
    ContextPrivKey,
    #[cfg(target_arch = "wasm32")]
    Metamask,
}

impl Default for EthPrivKeyActivationPolicy {
    fn default() -> Self { EthPrivKeyActivationPolicy::ContextPrivKey }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum EthRpcMode {
    Default,
    #[cfg(target_arch = "wasm32")]
    Metamask,
}

impl Default for EthRpcMode {
    fn default() -> Self { EthRpcMode::Default }
}

#[derive(Clone, Deserialize)]
pub struct EthActivationV2Request {
    #[serde(default)]
    pub nodes: Vec<EthNode>,
    #[serde(default)]
    pub rpc_mode: EthRpcMode,
    pub swap_contract_address: Address,
    pub fallback_swap_contract: Option<Address>,
    #[serde(default)]
    pub contract_supports_watchers: bool,
    pub gas_station_url: Option<String>,
    pub gas_station_decimals: Option<u8>,
    #[serde(default)]
    pub gas_station_policy: GasStationPricePolicy,
    pub mm2: Option<u8>,
    pub required_confirmations: Option<u64>,
    #[serde(default)]
    pub priv_key_policy: EthPrivKeyActivationPolicy,
    #[serde(default)]
    pub path_to_address: StandardHDCoinAddress,
}

#[derive(Clone, Deserialize)]
pub struct EthNode {
    pub url: String,
    #[serde(default)]
    pub gui_auth: bool,
}

#[derive(Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum Erc20TokenActivationError {
    InternalError(String),
    ClientConnectionFailed(String),
    CouldNotFetchBalance(String),
}

impl From<AbortedError> for Erc20TokenActivationError {
    fn from(e: AbortedError) -> Self { Erc20TokenActivationError::InternalError(e.to_string()) }
}

impl From<MyAddressError> for Erc20TokenActivationError {
    fn from(err: MyAddressError) -> Self { Self::InternalError(err.to_string()) }
}

#[derive(Clone, Deserialize)]
pub struct Erc20TokenActivationRequest {
    pub required_confirmations: Option<u64>,
}

pub struct Erc20Protocol {
    pub platform: String,
    pub token_addr: Address,
}

#[cfg_attr(test, mockable)]
impl EthCoin {
    pub async fn initialize_erc20_token(
        &self,
        activation_params: Erc20TokenActivationRequest,
        protocol: Erc20Protocol,
        ticker: String,
    ) -> MmResult<EthCoin, Erc20TokenActivationError> {
        // TODO
        // Check if ctx is required.
        // Remove it to avoid circular references if possible
        let ctx = MmArc::from_weak(&self.ctx)
            .ok_or_else(|| String::from("No context"))
            .map_err(Erc20TokenActivationError::InternalError)?;

        let conf = coin_conf(&ctx, &ticker);

        let decimals = match conf["decimals"].as_u64() {
            None | Some(0) => get_token_decimals(
                &self
                    .web3()
                    .await
                    .map_err(|e| Erc20TokenActivationError::ClientConnectionFailed(e.to_string()))?,
                protocol.token_addr,
            )
            .await
            .map_err(Erc20TokenActivationError::InternalError)?,
            Some(d) => d as u8,
        };

        let web3_instances: Vec<Web3Instance> = self
            .web3_instances
            .lock()
            .await
            .iter()
            .map(|node| {
                let mut transport = node.web3.transport().clone();
                if let Some(auth) = transport.gui_auth_validation_generator_as_mut() {
                    auth.coin_ticker = ticker.clone();
                }
                let web3 = Web3::new(transport);
                Web3Instance {
                    web3,
                    is_parity: node.is_parity,
                }
            })
            .collect();

        let required_confirmations = activation_params
            .required_confirmations
            .unwrap_or_else(|| conf["required_confirmations"].as_u64().unwrap_or(1))
            .into();

        // Create an abortable system linked to the `MmCtx` so if the app is stopped on `MmArc::stop`,
        // all spawned futures related to `ERC20` coin will be aborted as well.
        let abortable_system = ctx.abortable_system.create_subsystem()?;

        let token = EthCoinImpl {
            priv_key_policy: self.priv_key_policy.clone(),
            my_address: self.my_address,
            coin_type: EthCoinType::Erc20 {
                platform: protocol.platform,
                token_addr: protocol.token_addr,
            },
            sign_message_prefix: self.sign_message_prefix.clone(),
            swap_contract_address: self.swap_contract_address,
            fallback_swap_contract: self.fallback_swap_contract,
            contract_supports_watchers: self.contract_supports_watchers,
            decimals,
            ticker,
            gas_station_url: self.gas_station_url.clone(),
            gas_station_decimals: self.gas_station_decimals,
            gas_station_policy: self.gas_station_policy.clone(),
            web3_instances: AsyncMutex::new(web3_instances),
            history_sync_state: Mutex::new(self.history_sync_state.lock().unwrap().clone()),
            ctx: self.ctx.clone(),
            required_confirmations,
            chain_id: self.chain_id,
            logs_block_range: self.logs_block_range,
            nonce_lock: self.nonce_lock.clone(),
            erc20_tokens_infos: Default::default(),
            abortable_system,
        };

        Ok(EthCoin(Arc::new(token)))
    }
}

pub async fn eth_coin_from_conf_and_request_v2(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    req: EthActivationV2Request,
    priv_key_policy: EthPrivKeyBuildPolicy,
) -> MmResult<EthCoin, EthActivationV2Error> {
    let ticker = ticker.to_string();

    if req.swap_contract_address == Address::default() {
        return Err(EthActivationV2Error::InvalidSwapContractAddr(
            "swap_contract_address can't be zero address".to_string(),
        )
        .into());
    }

    if let Some(fallback) = req.fallback_swap_contract {
        if fallback == Address::default() {
            return Err(EthActivationV2Error::InvalidFallbackSwapContract(
                "fallback_swap_contract can't be zero address".to_string(),
            )
            .into());
        }
    }

    let (my_address, priv_key_policy) =
        build_address_and_priv_key_policy(conf, priv_key_policy, &req.path_to_address).await?;
    let my_address_str = checksum_address(&format!("{:02x}", my_address));

    let chain_id = conf["chain_id"].as_u64();

    let web3_instances = match (req.rpc_mode, &priv_key_policy) {
        (
            EthRpcMode::Default,
            EthPrivKeyPolicy::Iguana(key_pair)
            | EthPrivKeyPolicy::HDWallet {
                activated_key: key_pair,
                ..
            },
        ) => build_web3_instances(ctx, ticker.clone(), my_address_str, key_pair, req.nodes.clone()).await?,
        (EthRpcMode::Default, EthPrivKeyPolicy::Trezor) => {
            return MmError::err(EthActivationV2Error::PrivKeyPolicyNotAllowed(
                PrivKeyPolicyNotAllowed::HardwareWalletNotSupported,
            ));
        },
        #[cfg(target_arch = "wasm32")]
        (EthRpcMode::Metamask, EthPrivKeyPolicy::Metamask(_)) => {
            let chain_id = chain_id
                .or_else(|| conf["rpc_chain_id"].as_u64())
                .or_mm_err(|| EthActivationV2Error::ExpectedRpcChainId)?;
            build_metamask_transport(ctx, ticker.clone(), chain_id).await?
        },
        #[cfg(target_arch = "wasm32")]
        (EthRpcMode::Default, EthPrivKeyPolicy::Metamask(_)) | (EthRpcMode::Metamask, _) => {
            let error = r#"priv_key_policy="Metamask" and rpc_mode="Metamask" should be used both"#.to_string();
            return MmError::err(EthActivationV2Error::ActivationFailed { ticker, error });
        },
    };

    // param from request should override the config
    let required_confirmations = req
        .required_confirmations
        .unwrap_or_else(|| {
            conf["required_confirmations"]
                .as_u64()
                .unwrap_or(DEFAULT_REQUIRED_CONFIRMATIONS as u64)
        })
        .into();

    let sign_message_prefix: Option<String> = json::from_value(conf["sign_message_prefix"].clone()).ok();

    let nonce_lock = {
        let mut map = NONCE_LOCK.lock().unwrap();
        map.entry(ticker.clone()).or_insert_with(new_nonce_lock).clone()
    };

    // Create an abortable system linked to the `MmCtx` so if the app is stopped on `MmArc::stop`,
    // all spawned futures related to `ETH` coin will be aborted as well.
    let abortable_system = ctx.abortable_system.create_subsystem()?;

    let coin = EthCoinImpl {
        priv_key_policy,
        my_address,
        coin_type: EthCoinType::Eth,
        sign_message_prefix,
        swap_contract_address: req.swap_contract_address,
        fallback_swap_contract: req.fallback_swap_contract,
        contract_supports_watchers: req.contract_supports_watchers,
        decimals: ETH_DECIMALS,
        ticker,
        gas_station_url: req.gas_station_url,
        gas_station_decimals: req.gas_station_decimals.unwrap_or(ETH_GAS_STATION_DECIMALS),
        gas_station_policy: req.gas_station_policy,
        web3_instances: AsyncMutex::new(web3_instances),
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        ctx: ctx.weak(),
        required_confirmations,
        chain_id,
        logs_block_range: conf["logs_block_range"].as_u64().unwrap_or(DEFAULT_LOGS_BLOCK_RANGE),
        nonce_lock,
        erc20_tokens_infos: Default::default(),
        abortable_system,
    };

    let coin = EthCoin(Arc::new(coin));
    coin.spawn_balance_stream_if_enabled(ctx)
        .await
        .map_err(EthActivationV2Error::FailedSpawningBalanceEvents)?;

    Ok(coin)
}

/// Processes the given `priv_key_policy` and generates corresponding `KeyPair`.
/// This function expects either [`PrivKeyBuildPolicy::IguanaPrivKey`]
/// or [`PrivKeyBuildPolicy::GlobalHDAccount`], otherwise returns `PrivKeyPolicyNotAllowed` error.
pub(crate) async fn build_address_and_priv_key_policy(
    conf: &Json,
    priv_key_policy: EthPrivKeyBuildPolicy,
    path_to_address: &StandardHDCoinAddress,
) -> MmResult<(Address, EthPrivKeyPolicy), EthActivationV2Error> {
    match priv_key_policy {
        EthPrivKeyBuildPolicy::IguanaPrivKey(iguana) => {
            let key_pair = KeyPair::from_secret_slice(iguana.as_slice())
                .map_to_mm(|e| EthActivationV2Error::InternalError(e.to_string()))?;
            Ok((key_pair.address(), EthPrivKeyPolicy::Iguana(key_pair)))
        },
        EthPrivKeyBuildPolicy::GlobalHDAccount(global_hd_ctx) => {
            // Consider storing `derivation_path` at `EthCoinImpl`.
            let derivation_path = json::from_value(conf["derivation_path"].clone())
                .map_to_mm(|e| EthActivationV2Error::ErrorDeserializingDerivationPath(e.to_string()))?;
            let raw_priv_key = global_hd_ctx
                .derive_secp256k1_secret(&derivation_path, path_to_address)
                .mm_err(|e| EthActivationV2Error::InternalError(e.to_string()))?;
            let activated_key_pair = KeyPair::from_secret_slice(raw_priv_key.as_slice())
                .map_to_mm(|e| EthActivationV2Error::InternalError(e.to_string()))?;
            let bip39_secp_priv_key = global_hd_ctx.root_priv_key().clone();
            Ok((activated_key_pair.address(), EthPrivKeyPolicy::HDWallet {
                derivation_path,
                activated_key: activated_key_pair,
                bip39_secp_priv_key,
            }))
        },
        #[cfg(target_arch = "wasm32")]
        EthPrivKeyBuildPolicy::Metamask(metamask_ctx) => {
            let address = *metamask_ctx.check_active_eth_account().await?;
            let public_key_uncompressed = metamask_ctx.eth_account_pubkey_uncompressed();
            let public_key = compress_public_key(public_key_uncompressed)?;
            Ok((
                address,
                EthPrivKeyPolicy::Metamask(EthMetamaskPolicy {
                    public_key,
                    public_key_uncompressed,
                }),
            ))
        },
    }
}

async fn build_web3_instances(
    ctx: &MmArc,
    coin_ticker: String,
    address: String,
    key_pair: &KeyPair,
    mut eth_nodes: Vec<EthNode>,
) -> MmResult<Vec<Web3Instance>, EthActivationV2Error> {
    if eth_nodes.is_empty() {
        return MmError::err(EthActivationV2Error::AtLeastOneNodeRequired);
    }

    let mut rng = small_rng();
    eth_nodes.as_mut_slice().shuffle(&mut rng);
    drop_mutability!(eth_nodes);

    let event_handlers = rpc_event_handlers_for_eth_transport(ctx, coin_ticker.clone());

    let mut web3_instances = Vec::with_capacity(eth_nodes.len());
    for eth_node in eth_nodes {
        let uri: Uri = eth_node
            .url
            .parse()
            .map_err(|_| EthActivationV2Error::InvalidPayload(format!("{} could not be parsed.", eth_node.url)))?;

        let transport = match uri.scheme_str() {
            Some("ws") | Some("wss") => {
                const TMP_SOCKET_CONNECTION: Duration = Duration::from_secs(20);

                let node = WebsocketTransportNode {
                    uri: uri.clone(),
                    gui_auth: eth_node.gui_auth,
                };

                let mut websocket_transport = WebsocketTransport::with_event_handlers(node, event_handlers.clone());

                if eth_node.gui_auth {
                    websocket_transport.gui_auth_validation_generator = Some(GuiAuthValidationGenerator {
                        coin_ticker: coin_ticker.clone(),
                        secret: key_pair.secret().clone(),
                        address: address.clone(),
                    });
                }

                // Temporarily start the connection loop (we close the connection once we have the client version below).
                // Ideally, it would be much better to not do this workaround, which requires a lot of refactoring or
                // dropping websocket support on parity nodes.
                let fut = websocket_transport
                    .clone()
                    .start_connection_loop(Some(Instant::now() + TMP_SOCKET_CONNECTION));
                let settings = AbortSettings::info_on_abort(format!("connection loop stopped for {:?}", uri));
                ctx.spawner().spawn_with_settings(fut, settings);

                Web3Transport::Websocket(websocket_transport)
            },
            Some("http") | Some("https") => {
                let node = HttpTransportNode {
                    uri,
                    gui_auth: eth_node.gui_auth,
                };

                build_http_transport(
                    coin_ticker.clone(),
                    address.clone(),
                    key_pair,
                    node,
                    event_handlers.clone(),
                )
            },
            _ => {
                return MmError::err(EthActivationV2Error::InvalidPayload(format!(
                    "Invalid node address '{uri}'. Only http(s) and ws(s) nodes are supported"
                )));
            },
        };

        let web3 = Web3::new(transport);
        let version = match web3.web3().client_version().await {
            Ok(v) => v,
            Err(e) => {
                error!("Couldn't get client version for url {}: {}", eth_node.url, e);
                continue;
            },
        };

        web3_instances.push(Web3Instance {
            web3,
            is_parity: version.contains("Parity") || version.contains("parity"),
        });
    }

    if web3_instances.is_empty() {
        return Err(
            EthActivationV2Error::UnreachableNodes("Failed to get client version for all nodes".to_string()).into(),
        );
    }

    Ok(web3_instances)
}

fn build_http_transport(
    coin_ticker: String,
    address: String,
    key_pair: &KeyPair,
    node: HttpTransportNode,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Web3Transport {
    use crate::eth::web3_transport::http_transport::HttpTransport;

    let gui_auth = node.gui_auth;
    let mut http_transport = HttpTransport::with_event_handlers(node, event_handlers);

    if gui_auth {
        http_transport.gui_auth_validation_generator = Some(GuiAuthValidationGenerator {
            coin_ticker,
            secret: key_pair.secret().clone(),
            address,
        });
    }
    Web3Transport::from(http_transport)
}

#[cfg(target_arch = "wasm32")]
async fn build_metamask_transport(
    ctx: &MmArc,
    coin_ticker: String,
    chain_id: u64,
) -> MmResult<Vec<Web3Instance>, EthActivationV2Error> {
    let event_handlers = rpc_event_handlers_for_eth_transport(ctx, coin_ticker.clone());

    let eth_config = web3_transport::metamask_transport::MetamaskEthConfig { chain_id };
    let web3 = Web3::new(Web3Transport::new_metamask_with_event_handlers(
        eth_config,
        event_handlers,
    )?);

    // Check if MetaMask supports the given `chain_id`.
    // Please note that this request may take a long time.
    check_metamask_supports_chain_id(coin_ticker, &web3, chain_id).await?;

    // MetaMask doesn't use Parity nodes. So `MetamaskTransport` doesn't support `parity_nextNonce` RPC.
    // An example of the `web3_clientVersion` RPC - `MetaMask/v10.22.1`.
    let web3_instances = vec![Web3Instance { web3, is_parity: false }];

    Ok(web3_instances)
}

/// This method is based on the fact that `MetamaskTransport` tries to switch the `ChainId`
/// if the MetaMask is targeted to another ETH chain.
#[cfg(target_arch = "wasm32")]
async fn check_metamask_supports_chain_id(
    ticker: String,
    web3: &Web3<Web3Transport>,
    expected_chain_id: u64,
) -> MmResult<(), EthActivationV2Error> {
    use jsonrpc_core::ErrorCode;

    /// See the documentation:
    /// https://docs.metamask.io/guide/rpc-api.html#wallet-switchethereumchain
    const CHAIN_IS_NOT_REGISTERED_ERROR: ErrorCode = ErrorCode::ServerError(4902);

    match web3.eth().chain_id().await {
        Ok(chain_id) if chain_id == U256::from(expected_chain_id) => Ok(()),
        // The RPC client should have returned ChainId with which it has been created on [`Web3Transport::new_metamask_with_event_handlers`].
        Ok(unexpected_chain_id) => {
            let error = format!("Expected '{expected_chain_id}' ChainId, found '{unexpected_chain_id}'");
            MmError::err(EthActivationV2Error::InternalError(error))
        },
        Err(web3::Error::Rpc(rpc_err)) if rpc_err.code == CHAIN_IS_NOT_REGISTERED_ERROR => {
            let error = format!("Ethereum chain_id({expected_chain_id}) is not supported");
            MmError::err(EthActivationV2Error::ActivationFailed { ticker, error })
        },
        Err(other) => {
            let error = other.to_string();
            MmError::err(EthActivationV2Error::ActivationFailed { ticker, error })
        },
    }
}

#[cfg(target_arch = "wasm32")]
fn compress_public_key(uncompressed: H520) -> MmResult<H264, EthActivationV2Error> {
    let public_key = PublicKey::from_slice(uncompressed.as_bytes())
        .map_to_mm(|e| EthActivationV2Error::InternalError(e.to_string()))?;
    let compressed = public_key.serialize();
    Ok(H264::from(compressed))
}

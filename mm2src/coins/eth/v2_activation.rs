use super::*;

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EthActivationV2Error {
    InvalidPayload(String),
    InvalidSwapContractAddr(String),
    InvalidFallbackSwapContract(String),
    #[display(fmt = "Platform coin {} activation failed. {}", ticker, error)]
    ActivationFailed {
        ticker: String,
        error: String,
    },
    CouldNotFetchBalance(String),
    UnreachableNodes(String),
    #[display(fmt = "Enable request for ETH coin must have at least 1 node")]
    AtLeastOneNodeRequired,
    InternalError(String),
}

#[derive(Clone, Deserialize)]
pub struct EthActivationV2Request {
    pub nodes: Vec<EthNode>,
    pub swap_contract_address: Address,
    pub fallback_swap_contract: Option<Address>,
    pub gas_station_url: Option<String>,
    pub gas_station_decimals: Option<u8>,
    #[serde(default)]
    pub gas_station_policy: GasStationPricePolicy,
    pub mm2: Option<u8>,
    pub required_confirmations: Option<u64>,
}

#[derive(Clone, Deserialize)]
pub struct EthNode {
    pub url: String,
    pub gui_auth: bool,
}

#[derive(Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum Erc20TokenActivationError {
    InternalError(String),
    CouldNotFetchBalance(String),
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
    ) -> Result<EthCoin, MmError<Erc20TokenActivationError>> {
        // TODO
        // Check if ctx is required.
        // Remove it to avoid circular references if possible
        let ctx = MmArc::from_weak(&self.ctx)
            .ok_or_else(|| String::from("No context"))
            .map_err(Erc20TokenActivationError::InternalError)?;

        let conf = coin_conf(&ctx, &ticker);

        let decimals = match conf["decimals"].as_u64() {
            None | Some(0) => get_token_decimals(&self.web3, protocol.token_addr)
                .await
                .map_err(Erc20TokenActivationError::InternalError)?,
            Some(d) => d as u8,
        };

        let required_confirmations = activation_params
            .required_confirmations
            .unwrap_or_else(|| conf["required_confirmations"].as_u64().unwrap_or(1))
            .into();

        let token = EthCoinImpl {
            key_pair: self.key_pair.clone(),
            my_address: self.my_address,
            coin_type: EthCoinType::Erc20 {
                platform: protocol.platform,
                token_addr: protocol.token_addr,
            },
            sign_message_prefix: self.sign_message_prefix.clone(),
            swap_contract_address: self.swap_contract_address,
            fallback_swap_contract: self.fallback_swap_contract,
            decimals,
            ticker,
            gas_station_url: self.gas_station_url.clone(),
            gas_station_decimals: self.gas_station_decimals,
            gas_station_policy: self.gas_station_policy,
            web3: self.web3.clone(),
            web3_instances: self.web3_instances.clone(),
            history_sync_state: Mutex::new(self.history_sync_state.lock().unwrap().clone()),
            ctx: self.ctx.clone(),
            required_confirmations,
            chain_id: self.chain_id,
            logs_block_range: self.logs_block_range,
            nonce_lock: self.nonce_lock.clone(),
            erc20_tokens_infos: Default::default(),
        };

        Ok(EthCoin(Arc::new(token)))
    }
}

pub async fn eth_coin_from_conf_and_request_v2(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    req: EthActivationV2Request,
    priv_key: &[u8],
) -> Result<EthCoin, MmError<EthActivationV2Error>> {
    if req.nodes.is_empty() {
        return Err(EthActivationV2Error::AtLeastOneNodeRequired.into());
    }

    let mut rng = small_rng();
    let mut req = req;
    req.nodes.as_mut_slice().shuffle(&mut rng);
    drop_mutability!(req);

    let mut nodes = vec![];
    for node in req.nodes.iter() {
        let uri = node
            .url
            .parse()
            .map_err(|_| EthActivationV2Error::InvalidPayload(format!("{} could not be parsed.", node.url)))?;

        nodes.push(Web3TransportNode {
            uri,
            gui_auth: node.gui_auth,
        });
    }
    drop_mutability!(nodes);

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

    let key_pair: KeyPair =
        KeyPair::from_secret_slice(priv_key).map_err(|e| EthActivationV2Error::InternalError(e.to_string()))?;
    let my_address = checksum_address(&format!("{:02x}", key_pair.address()));

    let mut web3_instances = vec![];
    let event_handlers = rpc_event_handlers_for_eth_transport(ctx, ticker.to_string());
    for node in &nodes {
        let mut transport = Web3Transport::with_event_handlers(vec![node.clone()], event_handlers.clone());
        transport.gui_auth_validation_generator = Some(GuiAuthValidationGenerator {
            coin_ticker: ticker.to_string(),
            secret: key_pair.secret().clone(),
            address: my_address.clone(),
        });
        drop_mutability!(transport);

        let web3 = Web3::new(transport);
        let version = match web3.web3().client_version().compat().await {
            Ok(v) => v,
            Err(e) => {
                error!("Couldn't get client version for url {}: {}", node.uri, e);
                continue;
            },
        };
        web3_instances.push(Web3Instance {
            web3,
            is_parity: version.contains("Parity") || version.contains("parity"),
        })
    }

    if web3_instances.is_empty() {
        return Err(
            EthActivationV2Error::UnreachableNodes("Failed to get client version for all nodes".to_string()).into(),
        );
    }

    let mut transport = Web3Transport::with_event_handlers(nodes, event_handlers);
    transport.gui_auth_validation_generator = Some(GuiAuthValidationGenerator {
        coin_ticker: ticker.to_string(),
        secret: key_pair.secret().clone(),
        address: my_address,
    });
    drop_mutability!(transport);

    let web3 = Web3::new(transport);

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

    let mut map = NONCE_LOCK.lock().unwrap();
    let nonce_lock = map.entry(ticker.to_string()).or_insert_with(new_nonce_lock).clone();

    let coin = EthCoinImpl {
        key_pair: key_pair.clone(),
        my_address: key_pair.address(),
        coin_type: EthCoinType::Eth,
        sign_message_prefix,
        swap_contract_address: req.swap_contract_address,
        fallback_swap_contract: req.fallback_swap_contract,
        decimals: ETH_DECIMALS,
        ticker: ticker.into(),
        gas_station_url: req.gas_station_url,
        gas_station_decimals: req.gas_station_decimals.unwrap_or(ETH_GAS_STATION_DECIMALS),
        gas_station_policy: req.gas_station_policy,
        web3,
        web3_instances,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        ctx: ctx.weak(),
        required_confirmations,
        chain_id: conf["chain_id"].as_u64(),
        logs_block_range: conf["logs_block_range"].as_u64().unwrap_or(DEFAULT_LOGS_BLOCK_RANGE),
        nonce_lock,
        erc20_tokens_infos: Default::default(),
    };

    Ok(EthCoin(Arc::new(coin)))
}

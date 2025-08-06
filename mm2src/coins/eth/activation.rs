use super::*;

/// Activate eth coin or erc20 token from coin config and private key build policy
pub async fn eth_coin_from_conf_and_request(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    req: &Json,
    protocol: CoinProtocol,
    priv_key_policy: PrivKeyBuildPolicy,
) -> Result<EthCoin, String> {
    if conf["coin"].as_str() != Some(ticker) {
        return ERR!("Failed to activate '{}': ticker does not match coins config", ticker);
    }

    fn get_chain_id_from_platform(ctx: &MmArc, ticker: &str, platform: &str) -> Result<u64, String> {
        let platform_conf = coin_conf(ctx, platform);
        if platform_conf.is_null() {
            return ERR!(
                "Failed to activate ERC20 token '{}': the platform '{}' is not defined in the coins config.",
                ticker,
                platform
            );
        }
        let platform_protocol: CoinProtocol = json::from_value(platform_conf["protocol"].clone())
            .map_err(|e| ERRL!("Error parsing platform protocol for '{}': {}", platform, e))?;
        match platform_protocol {
            CoinProtocol::ETH { chain_id } => Ok(chain_id),
            protocol => ERR!(
                "Failed to activate ERC20 token '{}': the platform protocol '{:?}' must be ETH",
                ticker,
                protocol
            ),
        }
    }

    // Convert `PrivKeyBuildPolicy` to `EthPrivKeyBuildPolicy`.
    let priv_key_policy = match priv_key_policy {
        PrivKeyBuildPolicy::IguanaPrivKey(iguana) => EthPrivKeyBuildPolicy::IguanaPrivKey(iguana),
        PrivKeyBuildPolicy::GlobalHDAccount(global_hd) => EthPrivKeyBuildPolicy::GlobalHDAccount(global_hd),
        PrivKeyBuildPolicy::Trezor => EthPrivKeyBuildPolicy::Trezor,
        PrivKeyBuildPolicy::WalletConnect { .. } => {
            return ERR!("WalletConnect private key policy is not supported for legacy ETH coin activation");
        },
    };

    let mut urls: Vec<String> = try_s!(json::from_value(req["urls"].clone()));
    if urls.is_empty() {
        return ERR!("Enable request for ETH coin must have at least 1 node URL");
    }
    let mut rng = small_rng();
    urls.as_mut_slice().shuffle(&mut rng);

    let swap_contract_address: Address = try_s!(json::from_value(req["swap_contract_address"].clone()));
    if swap_contract_address == Address::default() {
        return ERR!("swap_contract_address can't be zero address");
    }
    let fallback_swap_contract: Option<Address> = try_s!(json::from_value(req["fallback_swap_contract"].clone()));
    if let Some(fallback) = fallback_swap_contract {
        if fallback == Address::default() {
            return ERR!("fallback_swap_contract can't be zero address");
        }
    }
    let contract_supports_watchers = req["contract_supports_watchers"].as_bool().unwrap_or_default();

    let path_to_address = try_s!(json::from_value::<Option<HDPathAccountToAddressId>>(
        req["path_to_address"].clone()
    ))
    .unwrap_or_default();
    let (key_pair, derivation_method) = try_s!(
        build_address_and_priv_key_policy(ctx, ticker, conf, priv_key_policy, &path_to_address, None, None).await
    );

    let mut web3_instances = vec![];
    let event_handlers = rpc_event_handlers_for_eth_transport(ctx, ticker.to_string());
    for url in urls.iter() {
        let uri: Uri = try_s!(url.parse());

        let transport = match uri.scheme_str() {
            Some("ws") | Some("wss") => {
                const TMP_SOCKET_CONNECTION: Duration = Duration::from_secs(20);

                let node = WebsocketTransportNode { uri: uri.clone() };
                let websocket_transport = WebsocketTransport::with_event_handlers(node, event_handlers.clone());

                // Temporarily start the connection loop (we close the connection once we have the client version below).
                // Ideally, it would be much better to not do this workaround, which requires a lot of refactoring or
                // dropping websocket support on parity nodes.
                let fut = websocket_transport
                    .clone()
                    .start_connection_loop(Some(Instant::now() + TMP_SOCKET_CONNECTION));
                let settings = AbortSettings::info_on_abort(format!("connection loop stopped for {uri:?}"));
                ctx.spawner().spawn_with_settings(fut, settings);

                Web3Transport::Websocket(websocket_transport)
            },
            Some("http") | Some("https") => {
                let node = HttpTransportNode {
                    uri,
                    komodo_proxy: false,
                };

                Web3Transport::new_http_with_event_handlers(node, event_handlers.clone())
            },
            _ => {
                return ERR!(
                    "Invalid node address '{}'. Only http(s) and ws(s) nodes are supported",
                    uri
                );
            },
        };

        let web3 = Web3::new(transport);

        web3_instances.push(Web3Instance(web3))
    }

    if web3_instances.is_empty() {
        return ERR!("Failed to get client version for all urls");
    }

    let (coin_type, decimals, chain_id) = match protocol {
        CoinProtocol::ETH { chain_id } => (EthCoinType::Eth, ETH_DECIMALS, chain_id),
        CoinProtocol::ERC20 {
            platform,
            contract_address,
        } => {
            let token_addr = try_s!(valid_addr_from_str(&contract_address));
            let decimals = match conf["decimals"].as_u64() {
                None | Some(0) => try_s!(
                    get_token_decimals(
                        web3_instances
                            .first()
                            .expect("web3_instances can't be empty in ETH activation")
                            .as_ref(),
                        token_addr
                    )
                    .await
                ),
                Some(d) => d as u8,
            };
            let chain_id = get_chain_id_from_platform(ctx, ticker, &platform)?;
            (EthCoinType::Erc20 { platform, token_addr }, decimals, chain_id)
        },
        CoinProtocol::NFT { platform } => {
            let chain_id = get_chain_id_from_platform(ctx, ticker, &platform)?;
            (EthCoinType::Nft { platform }, ETH_DECIMALS, chain_id)
        },
        _ => return ERR!("Expect ETH, ERC20 or NFT protocol"),
    };

    // param from request should override the config
    let required_confirmations = req["required_confirmations"]
        .as_u64()
        .unwrap_or_else(|| {
            conf["required_confirmations"]
                .as_u64()
                .unwrap_or(DEFAULT_REQUIRED_CONFIRMATIONS as u64)
        })
        .into();

    if req["requires_notarization"].as_bool().is_some() {
        warn!("requires_notarization doesn't take any effect on ETH/ERC20 coins");
    }

    let sign_message_prefix: Option<String> = json::from_value(conf["sign_message_prefix"].clone()).unwrap_or(None);

    let trezor_coin: Option<String> = json::from_value(conf["trezor_coin"].clone()).unwrap_or(None);

    let initial_history_state = if req["tx_history"].as_bool().unwrap_or(false) {
        HistorySyncState::NotStarted
    } else {
        HistorySyncState::NotEnabled
    };

    let platform_ticker = match &coin_type {
        EthCoinType::Eth => String::from(ticker),
        EthCoinType::Erc20 { platform, .. } | EthCoinType::Nft { platform } => String::from(platform),
    };

    // Create an abortable system linked to the `MmCtx` so if the context is stopped via `MmArc::stop`,
    // all spawned futures related to `ETH` coin will be aborted as well.
    let abortable_system = try_s!(ctx.abortable_system.create_subsystem());

    let max_eth_tx_type = get_conf_param_or_from_plaform_coin(ctx, conf, &coin_type, MAX_ETH_TX_TYPE_SUPPORTED)?;
    let gas_price_adjust = get_conf_param_or_from_plaform_coin(ctx, conf, &coin_type, GAS_PRICE_ADJUST)?;
    let estimate_gas_mult = get_conf_param_or_from_plaform_coin(ctx, conf, &coin_type, ESTIMATE_GAS_MULT)?;
    let gas_limit: EthGasLimit =
        get_conf_param_or_from_plaform_coin(ctx, conf, &coin_type, EthGasLimit::key())?.unwrap_or_default();
    let gas_limit_v2: EthGasLimitV2 =
        get_conf_param_or_from_plaform_coin(ctx, conf, &coin_type, EthGasLimitV2::key())?.unwrap_or_default();
    let swap_gas_fee_policy_default: SwapGasFeePolicy =
        get_conf_param_or_from_plaform_coin(ctx, conf, &coin_type, SWAP_GAS_FEE_POLICY)?.unwrap_or_default();
    let swap_gas_fee_policy: SwapGasFeePolicy =
        json::from_value(req["swap_gas_fee_policy"].clone()).unwrap_or(swap_gas_fee_policy_default);

    let coin = EthCoinImpl {
        priv_key_policy: key_pair,
        derivation_method: Arc::new(derivation_method),
        coin_type,
        // Tron is not supported for v1 activation
        chain_spec: ChainSpec::Evm { chain_id },
        sign_message_prefix,
        swap_contract_address,
        swap_v2_contracts: None,
        fallback_swap_contract,
        contract_supports_watchers,
        decimals,
        ticker: ticker.into(),
        web3_instances: AsyncMutex::new(web3_instances),
        history_sync_state: Mutex::new(initial_history_state),
        swap_gas_fee_policy: Mutex::new(swap_gas_fee_policy),
        max_eth_tx_type,
        gas_price_adjust,
        ctx: ctx.weak(),
        required_confirmations,
        trezor_coin,
        logs_block_range: conf["logs_block_range"].as_u64().unwrap_or(DEFAULT_LOGS_BLOCK_RANGE),
        address_nonce_locks: PerNetNonceLocks::get_net_locks(platform_ticker),
        erc20_tokens_infos: Default::default(),
        nfts_infos: Default::default(),
        gas_limit,
        gas_limit_v2,
        estimate_gas_mult,
        abortable_system,
    };

    Ok(EthCoin(Arc::new(coin)))
}

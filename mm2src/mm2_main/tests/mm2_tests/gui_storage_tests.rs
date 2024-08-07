#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_gui_storage_accounts_functionality() {
    let passphrase = "test_gui_storage passphrase";

    let conf = Mm2TestConf::seednode(passphrase, &json!([]));
    let mm = block_on(MarketMakerIt::start_async(conf.conf, conf.rpc_password, None)).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm.mm_dump();
    log!("Log path: {}", mm.log_path.display());

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::enable_account",
        "params": {
            "policy": "new",
            "account_id": {
                "type": "iguana"
            },
            "name": "My Iguana wallet",
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::enable_account: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::add_account",
        "params": {
            "account_id": {
                "type": "hw",
                "device_pubkey": "1549128bbfb33b997949b4105b6a6371c998e212"
            },
            "description": "Any description",
            "name": "My HW",
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::add_account: {}", resp.1);

    // Add `HD{1}` account that will be deleted later.
    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::add_account",
        "params": {
            "account_id": {
                "type": "hd",
                "account_idx": 1,
            },
            "name": "An HD account"
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::add_account: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::delete_account",
        "params": {
            "account_id": {
                "type": "hd",
                "account_idx": 1,
            }
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::delete_account: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::set_account_balance",
        "params": {
            "account_id": {
                "type": "hw",
                "device_pubkey": "1549128bbfb33b997949b4105b6a6371c998e212"
            },
            "balance_usd": "123.567",
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::set_account_balance: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::set_account_name",
        "params": {
            "account_id": {
                "type": "iguana"
            },
            "name": "New Iguana account name",
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::set_account_name: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::set_account_description",
        "params": {
            "account_id": {
                "type": "iguana"
            },
            "description": "Another description",
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::set_account_description: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::get_accounts"
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::get_accounts: {}", resp.1);

    let actual: RpcV2Response<Vec<gui_storage::AccountWithEnabledFlag>> = json::from_str(&resp.1).unwrap();
    let expected = vec![
        gui_storage::AccountWithEnabledFlag {
            account_id: gui_storage::AccountId::Iguana,
            name: "New Iguana account name".to_string(),
            description: "Another description".to_string(),
            balance_usd: BigDecimal::from(0i32),
            enabled: true,
        },
        gui_storage::AccountWithEnabledFlag {
            account_id: gui_storage::AccountId::HW {
                device_pubkey: "1549128bbfb33b997949b4105b6a6371c998e212".to_string(),
            },
            name: "My HW".to_string(),
            description: "Any description".to_string(),
            balance_usd: BigDecimal::from(123567i32) / BigDecimal::from(1000i32),
            enabled: false,
        },
    ];
    assert_eq!(actual.result, expected);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_gui_storage_coins_functionality() {
    let passphrase = "test_gui_storage passphrase";

    let conf = Mm2TestConf::seednode(passphrase, &json!([]));
    let mm = block_on(MarketMakerIt::start_async(conf.conf, conf.rpc_password, None)).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm.mm_dump();
    log!("Log path: {}", mm.log_path.display());

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::enable_account",
        "params": {
            "policy": "new",
            "account_id": {
                "type": "iguana"
            },
            "name": "My Iguana wallet",
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::enable_account: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::add_account",
        "params": {
            "account_id": {
                "type": "hw",
                "device_pubkey": "1549128bbfb33b997949b4105b6a6371c998e212"
            },
            "description": "Any description",
            "name": "My HW",
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::add_account: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::activate_coins",
        "params": {
            "account_id": {
                "type": "iguana"
            },
            "tickers": ["RICK", "MORTY", "KMD"],
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::activate_coins: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::activate_coins",
        "params": {
            "account_id": {
                "type": "hw",
                "device_pubkey": "1549128bbfb33b997949b4105b6a6371c998e212"
            },
            "tickers": ["KMD", "MORTY", "BCH"],
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::activate_coins: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::deactivate_coins",
        "params": {
            "account_id": {
                "type": "hw",
                "device_pubkey": "1549128bbfb33b997949b4105b6a6371c998e212"
            },
            "tickers": ["BTC", "MORTY"],
        },
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::deactivate_coins: {}", resp.1);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::get_enabled_account",
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::get_enabled_account: {}", resp.1);
    let actual: RpcV2Response<gui_storage::AccountWithCoins> = json::from_str(&resp.1).unwrap();
    let expected = gui_storage::AccountWithCoins {
        account_id: gui_storage::AccountId::Iguana,
        name: "My Iguana wallet".to_string(),
        description: String::new(),
        balance_usd: BigDecimal::from(0i32),
        coins: vec!["RICK".to_string(), "MORTY".to_string(), "KMD".to_string()]
            .into_iter()
            .collect(),
    };
    assert_eq!(actual.result, expected);

    let resp = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "gui_storage::get_account_coins",
        "params": {
            "account_id": {
                "type": "hw",
                "device_pubkey": "1549128bbfb33b997949b4105b6a6371c998e212"
            }
        }
    })))
    .unwrap();
    assert!(resp.0.is_success(), "!gui_storage::get_enabled_account: {}", resp.1);
    let actual: RpcV2Response<gui_storage::AccountCoins> = json::from_str(&resp.1).unwrap();
    let expected = gui_storage::AccountCoins {
        account_id: gui_storage::AccountId::HW {
            device_pubkey: "1549128bbfb33b997949b4105b6a6371c998e212".to_string(),
        },
        coins: vec!["KMD".to_string(), "BCH".to_string()].into_iter().collect(),
    };
    assert_eq!(actual.result, expected);
}

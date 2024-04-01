use super::*;
use crate::eth::for_tests::eth_coin_for_test;
use crate::lp_coininit;
use crypto::privkey::key_pair_from_seed;
use crypto::CryptoCtx;
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmCtxBuilder;
use mm2_test_helpers::for_tests::{ETH_DEV_NODE, ETH_DEV_SWAP_CONTRACT, ETH_SEPOLIA_NODES};
use mm2_test_helpers::get_passphrase;
use wasm_bindgen_test::*;
use web_sys::console;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn pass() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let _coins_context = CoinsContext::from_ctx(&ctx).unwrap();
}

#[wasm_bindgen_test]
async fn test_send() {
    let seed = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
    let keypair = key_pair_from_seed(&seed).unwrap();
    let key_pair = KeyPair::from_secret_slice(keypair.private_ref()).unwrap();
    let node = HttpTransportNode {
        uri: ETH_DEV_NODE.parse().unwrap(),
        gui_auth: false,
    };
    let transport = Web3Transport::new_http(node);
    let web3 = Web3::new(transport);
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let my_address = key_pair.address();
    let coin = EthCoin(Arc::new(EthCoinImpl {
        ticker: "ETH".into(),
        coin_type: EthCoinType::Eth,
        sign_message_prefix: Some(String::from("Ethereum Signed Message:\n")),
        priv_key_policy: key_pair.into(),
        derivation_method: Arc::new(DerivationMethod::SingleAddress(my_address)),
        swap_contract_address: Address::from_str(ETH_DEV_SWAP_CONTRACT).unwrap(),
        fallback_swap_contract: None,
        contract_supports_watchers: false,
        web3_instances: AsyncMutex::new(vec![Web3Instance { web3, is_parity: false }]),
        decimals: 18,
        gas_station_url: None,
        gas_station_decimals: ETH_GAS_STATION_DECIMALS,
        gas_station_policy: GasStationPricePolicy::MeanAverageFast,
        history_sync_state: Mutex::new(HistorySyncState::NotStarted),
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
        chain_id: 1,
        trezor_coin: None,
        logs_block_range: DEFAULT_LOGS_BLOCK_RANGE,
        address_nonce_locks: Arc::new(AsyncMutex::new(new_nonce_lock())),
        erc20_tokens_infos: Default::default(),
        nfts_infos: Default::default(),
        abortable_system: AbortableQueue::default(),
    }));
    let maker_payment_args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock: 1000,
        other_pubkey: &DEX_FEE_ADDR_RAW_PUBKEY,
        secret_hash: &[1; 20],
        amount: "0.001".parse().unwrap(),
        swap_contract_address: &coin.swap_contract_address(),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let tx = coin.send_maker_payment(maker_payment_args).compat().await.unwrap();
    console::log_1(&format!("{:?}", tx).into());

    let block = coin.current_block().compat().await.unwrap();
    console::log_1(&format!("{:?}", block).into());
}

async fn init_eth_coin_helper() -> Result<(MmArc, MmCoinEnum), String> {
    let conf = json!({
        "coins": [{
            "coin": "ETH",
            "name": "ethereum",
            "fname": "Ethereum",
            "protocol":{
                "type": "ETH"
            },
            "rpcport": 80,
            "mm2": 1
        }]
    });

    let ctx = MmCtxBuilder::new().with_conf(conf).into_mm_arc();
    CryptoCtx::init_with_iguana_passphrase(
        ctx.clone(),
        "spice describe gravity federal blast come thank unfair canal monkey style afraid",
    )
    .unwrap();

    let req = json!({
        "urls":[ETH_DEV_NODE],
        "swap_contract_address":ETH_DEV_SWAP_CONTRACT
    });
    Ok((ctx.clone(), lp_coininit(&ctx, "ETH", &req).await?))
}

#[wasm_bindgen_test]
async fn test_init_eth_coin() { let (_ctx, _coin) = init_eth_coin_helper().await.unwrap(); }

#[wasm_bindgen_test]
async fn wasm_test_sign_eth_tx() {
    // we need to hold ref to _ctx until the end of the test (because of the weak ref to MmCtx in EthCoinImpl)
    let (_ctx, coin) = init_eth_coin_helper().await.unwrap();
    let sign_req = json::from_value(json!({
        "coin": "ETH",
        "type": "ETH",
        "tx": {
            "to": "0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94".to_string(),
            "value": "1.234",
            "gas_limit": "21000"
        }
    }))
    .unwrap();
    let res = coin.sign_raw_tx(&sign_req).await;
    console::log_1(&format!("res={:?}", res).into());
    assert!(res.is_ok());
}

#[wasm_bindgen_test]
async fn wasm_test_withdraw_eth() {
    // we need to hold ref to _ctx until the end of the test (because of the weak ref to MmCtx in EthCoinImpl)
    let (_ctx, coin) = eth_coin_for_test(EthCoinType::Eth, ETH_SEPOLIA_NODES, None); // use sepolia as usualy some amount exists there

    let withdraw_req = WithdrawRequest {
        amount: "0.00001".parse().unwrap(),
        from: None,
        to: "0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94".to_string(),
        coin: "ETH".to_string(),
        max: false,
        fee: Some(WithdrawFee::EthGas {
            gas: ETH_GAS,
            gas_price: 1.into(),
        }),
        memo: None,
        broadcast: false,
    };
    let res = coin.withdraw(withdraw_req).compat().await;
    console::log_1(&format!("res={:?}", res).into());
    assert!(res.is_ok());
}

use super::*;
use crate::lp_coininit;
use crypto::CryptoCtx;
use mm2_core::mm_ctx::MmCtxBuilder;
use mm2_test_helpers::for_tests::{ETH_DEV_NODE, ETH_DEV_SWAP_CONTRACT};
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
    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    let transport = Web3Transport::single_node(ETH_DEV_NODE, false);
    let web3 = Web3::new(transport);
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let coin = EthCoin(Arc::new(EthCoinImpl {
        ticker: "ETH".into(),
        coin_type: EthCoinType::Eth,
        my_address: key_pair.address(),
        sign_message_prefix: Some(String::from("Ethereum Signed Message:\n")),
        priv_key_policy: key_pair.into(),
        swap_contract_address: Address::from_str(ETH_DEV_SWAP_CONTRACT).unwrap(),
        fallback_swap_contract: None,
        contract_supports_watchers: false,
        web3_instances: vec![Web3Instance {
            web3: web3.clone(),
            is_parity: false,
        }],
        web3,
        decimals: 18,
        gas_station_url: None,
        gas_station_decimals: ETH_GAS_STATION_DECIMALS,
        gas_station_policy: GasStationPricePolicy::MeanAverageFast,
        history_sync_state: Mutex::new(HistorySyncState::NotStarted),
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
        chain_id: None,
        logs_block_range: DEFAULT_LOGS_BLOCK_RANGE,
        nonce_lock: new_nonce_lock(),
        erc20_tokens_infos: Default::default(),
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

#[wasm_bindgen_test]
async fn test_init_eth_coin() {
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
    let _coin = lp_coininit(&ctx, "ETH", &req).await.unwrap();
}

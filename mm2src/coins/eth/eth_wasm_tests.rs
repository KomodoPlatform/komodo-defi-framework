use super::*;
use crate::lp_coininit;
use common::mm_ctx::MmCtxBuilder;
use crypto::CryptoCtx;
use wasm_bindgen_test::*;
use web_sys::console;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn pass() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let _coins_context = CoinsContext::from_ctx(&ctx).unwrap();
    assert_eq!(1, 1);
}

#[wasm_bindgen_test]
async fn test_send() {
    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    let transport = Web3Transport::new(vec!["http://195.201.0.6:8565".into()]).unwrap();
    let web3 = Web3::new(transport);
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let coin = EthCoin(Arc::new(EthCoinImpl {
        ticker: "ETH".into(),
        coin_type: EthCoinType::Eth,
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        fallback_swap_contract: None,
        web3_instances: vec![Web3Instance {
            web3: web3.clone(),
            is_parity: true,
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
    }));
    let tx = coin
        .send_maker_payment(
            1000,
            &[],
            &DEX_FEE_ADDR_RAW_PUBKEY,
            &[1; 20],
            "0.001".parse().unwrap(),
            &None,
        )
        .compat()
        .await;
    console::log_1(&format!("{:?}", tx).into());

    let block = coin.current_block().compat().await;
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
    CryptoCtx::init_with_passphrase(
        ctx.clone(),
        "spice describe gravity federal blast come thank unfair canal monkey style afraid",
    )
    .unwrap();

    let req = json!({
        "urls":["http://195.201.0.6:8565"],
        "swap_contract_address":"0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
    });
    let _coin = lp_coininit(&ctx, "ETH", &req).await.unwrap();
}

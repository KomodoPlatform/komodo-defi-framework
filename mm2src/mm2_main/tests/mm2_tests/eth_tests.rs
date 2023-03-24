use common::block_on;
use http::StatusCode;
use mm2_test_helpers::for_tests::{disable_coin, disable_coin_err, get_passphrase, MarketMakerIt, Mm2TestConf,
                                  ETH_DEV_NODES};
use serde_json::{json, Value as Json};
use std::str::FromStr;

#[cfg(not(target_arch = "wasm32"))]
async fn enable_eth_with_tokens(mm: &MarketMakerIt, platform_coin: &str, tokens: &[&str], nodes: &[&str]) -> Json {
    let erc20_tokens_requests: Vec<_> = tokens.iter().map(|ticker| json!({ "ticker": ticker })).collect();
    let nodes: Vec<_> = nodes.iter().map(|url| json!({ "url": url })).collect();

    let enable = mm
        .rpc(&json!({
        "userpass": mm.userpass,
        "method": "enable_eth_with_tokens",
        "mmrpc": "2.0",
        "params": {
                "ticker": platform_coin,
                "swap_contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E",
                "fallback_swap_contract":"0x8500AFc0bc5214728082163326C2FF0C73f4a871",
                "nodes": nodes,
                "tx_history": true,
                "erc20_tokens_requests": erc20_tokens_requests,
            }
        }))
        .await
        .unwrap();
    assert_eq!(
        enable.0,
        StatusCode::OK,
        "'enable_eth_with_tokens' failed: {}",
        enable.1
    );
    Json::from_str(&enable.1).unwrap()
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_disable_eth_coin_with_token() {
    let passphrase = get_passphrase(&".env.client", "BOB_PASSPHRASE").unwrap();
    let coins = json! ([
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80,"mm2":1},
        {"coin":"JST","name":"jst","rpcport":80,"mm2":1,"protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);
    let conf = Mm2TestConf::seednode(&passphrase, &coins);
    let mm = block_on(MarketMakerIt::start_async(conf.conf, conf.rpc_password, None)).unwrap();
    block_on(enable_eth_with_tokens(&mm, "ETH", &["JST"], ETH_DEV_NODES));

    // Create setprice order
    let req = json!({
        "userpass": mm.userpass,
        "method": "buy",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "base_confs": 5,
        "base_nota": false,
        "rel_confs": 4,
        "rel_nota": false,
    });
    let make_test_order = block_on(mm.rpc(&req)).unwrap();
    assert_eq!(make_test_order.0, StatusCode::OK);
    let order_uuid = Json::from_str(&make_test_order.1).unwrap();
    let order_uuid = order_uuid.get("result").unwrap().get("uuid").unwrap().as_str().unwrap();

    // Try to disable platform coin, ETH. This should fail due to the dependent tokens.
    let error = block_on(disable_coin_err(&mm, "ETH"));
    assert_eq!(error.dependent_tokens, ["JST"]);

    // Try to disable JST token first.
    // ETH and JST should be deactivated at once.
    let res = block_on(disable_coin(&mm, "JST"));
    // We expected make_test_order to be cancelled
    assert!(res.cancelled_orders.contains(order_uuid));
    // Then try to disable ETH platform coin.
    block_on(disable_coin(&mm, "ETH"));
}

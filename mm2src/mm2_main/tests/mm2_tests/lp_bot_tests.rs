use common::{block_on, log};
use http::HeaderMap;
use http::StatusCode;
use mm2_test_helpers::for_tests::MarketMakerIt;
use serde_json::json;

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_start_and_stop_simple_market_maker_bot() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"protocol":{"type":"UTXO"}},
    ]);

    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "passphrase": "bob passphrase",
            "rpc_password": "password",
            "coins": coins,
            "i_am_seed": true,
        }),
        "password".into(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("Log path: {}", mm.log_path.display());
    fn start_simple_market_maker_bot_rpc(mm: &MarketMakerIt) -> (StatusCode, String, HeaderMap) {
        block_on(mm.rpc(&json!({
             "userpass": "password",
             "mmrpc": "2.0",
             "method": "start_simple_market_maker_bot",
             "params": {
             "cfg": {
                "KMD-BEP20/BUSD-BEP20": {
                    "base": "KMD-BEP20",
                    "rel": "BUSD-BEP20",
                    "max": true,
                    "min_volume": {"percentage": "0.25"},
                    "spread": "1.025",
                    "base_confs": 3,
                    "base_nota": false,
                    "rel_confs": 1,
                    "rel_nota": false,
                    "enable": true
                }
            }
        },
        "id": 0
        })))
        .unwrap()
    }
    let mut start_simple_market_maker_bot = start_simple_market_maker_bot_rpc(&mm);

    // Must be 200
    assert_eq!(start_simple_market_maker_bot.0, 200);

    // Let's repeat - should get an already started
    start_simple_market_maker_bot = start_simple_market_maker_bot_rpc(&mm);

    // Must be 400
    assert_eq!(start_simple_market_maker_bot.0, 400);

    fn stop_simple_market_maker_bot_rpc(mm: &MarketMakerIt) -> (StatusCode, String, HeaderMap) {
        block_on(mm.rpc(&json!({
            "userpass": "password",
            "mmrpc": "2.0",
            "method": "stop_simple_market_maker_bot",
            "params": null,
            "id": 0
        })))
        .unwrap()
    }

    let mut stop_simple_market_maker_bot = stop_simple_market_maker_bot_rpc(&mm);
    // Must be 200
    assert_eq!(stop_simple_market_maker_bot.0, 200);

    stop_simple_market_maker_bot = stop_simple_market_maker_bot_rpc(&mm);

    assert_eq!(stop_simple_market_maker_bot.0, 400);
}

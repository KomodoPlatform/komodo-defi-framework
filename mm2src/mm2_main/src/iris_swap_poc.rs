use super::*;
use mm2_test_helpers::for_tests::enable_tendermint;

#[test]
#[ignore]
// cargo test mm2::mm2_tests::iris_swap_poc::test -- --exact --ignored
fn test() {
    block_on(trade_base_rel_iris(&[("IRIS-TEST", "IRIS-NIMDA")], 1, 2, 0.1));

    assert!(false);
}

pub async fn trade_base_rel_iris(
    pairs: &[(&'static str, &'static str)],
    maker_price: i32,
    taker_price: i32,
    volume: f64,
) {
    let bob_passphrase = String::from("iris test seed");
    let alice_passphrase = String::from("iris test2 seed");

    let coins = json! ([
        {"coin":"IRIS-USDC-IBC",
            "protocol":{
                "type":"TENDERMINT",
                "protocol_data": {
                    "decimals": 6,
                    "denom": "ibc/5C465997B4F582F602CD64E12031C6A6E18CAF1E6EDC9B5D808822DC0B5F850C",
                    "account_prefix": "iaa",
                    "chain_id": "nyancat-9",
                },
            }
        },
        {"coin":"IRIS-NIMDA",
            "protocol":{
                "type":"TENDERMINT",
                "protocol_data": {
                    "decimals": 6,
                    "denom": "nim",
                    "account_prefix": "iaa",
                    "chain_id": "nyancat-9",
                },
            }
        },
        {"coin":"IRIS-TEST",
            "protocol":{
                "type":"TENDERMINT",
                "protocol_data": {
                    "decimals": 6,
                    "denom": "unyan",
                    "account_prefix": "iaa",
                    "chain_id": "nyancat-9",
                },
            }
        }
    ]);

    let mut mm_bob = MarketMakerIt::start_async(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob"),
    )
    .await
    .unwrap();

    Timer::sleep(1.).await;

    let mut mm_alice = MarketMakerIt::start_async(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": alice_passphrase,
            "coins": coins,
            "seednodes": [mm_bob.my_seed_addr()],
            "rpc_password": "password",
            "skip_startup_checks": true,
        }),
        "password".into(),
        local_start!("alice"),
    )
    .await
    .unwrap();

    dbg!(enable_tendermint(&mm_bob, "IRIS-TEST", &["http://34.80.202.172:26657"]).await);
    dbg!(enable_tendermint(&mm_bob, "IRIS-NIMDA", &["http://34.80.202.172:26657"]).await);

    dbg!(enable_tendermint(&mm_alice, "IRIS-TEST", &["http://34.80.202.172:26657"]).await);
    dbg!(enable_tendermint(&mm_alice, "IRIS-NIMDA", &["http://34.80.202.172:26657"]).await);

    for (base, rel) in pairs.iter() {
        log!("Issue bob {}/{} sell request", base, rel);
        let rc = mm_bob
            .rpc(&json! ({
                "userpass": mm_bob.userpass,
                "method": "setprice",
                "base": base,
                "rel": rel,
                "price": maker_price,
                "volume": volume
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    }

    let mut uuids = vec![];

    for (base, rel) in pairs.iter() {
        common::log::info!(
            "Trigger alice subscription to {}/{} orderbook topic first and sleep for 1 second",
            base,
            rel
        );
        let rc = match mm_alice
            .rpc(&json! ({
                "userpass": mm_alice.userpass,
                "method": "orderbook",
                "base": base,
                "rel": rel,
            }))
            .await
        {
            Ok(t) => t,
            Err(_) => {
                Timer::sleep(5.).await;
                dbg!(mm_alice.log_as_utf8().unwrap());
                panic!();
            },
        };
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);
        Timer::sleep(1.).await;
        common::log::info!("Issue alice {}/{} buy request", base, rel);
        let rc = mm_alice
            .rpc(&json! ({
                "userpass": mm_alice.userpass,
                "method": "buy",
                "base": base,
                "rel": rel,
                "volume": volume,
                "price": taker_price
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);
        let buy_json: Json = serde_json::from_str(&rc.1).unwrap();
        uuids.push(buy_json["result"]["uuid"].as_str().unwrap().to_owned());
    }

    dbg!(mm_alice.log_as_utf8().unwrap());

    println!("\n `fn trade_base_rel_iris` hit end! \n");
}

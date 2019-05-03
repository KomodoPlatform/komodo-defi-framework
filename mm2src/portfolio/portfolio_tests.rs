use common::identity;
use common::for_tests::{enable_electrum, mm_spat, LocalStart};
use hyper::StatusCode;
use regex::Regex;
use serde_json::{self as json, Value as Json};

/// Integration test for the "autoprice" mode.
/// Starts MM in background and files a buy request with it, in the "autoprice" mode,
/// then checks the logs to see that the price fetching code works.
pub fn test_autoprice_coingecko (local_start: LocalStart) {
    // One of the ways we want to test the MarketMaker in the integration tests is by reading the logs.
    // Just like the end users, we learn of what's MarketMaker doing from the logs,
    // the information in the logs is actually a part of the user-visible functionality,
    // it should be readable, there should be enough information for both the users and the GUI to understand what's going on
    // and to make an informed decision about whether the MarketMaker is performing correctly.

    let (passphrase, mut mm, _dump_log, _dump_dashboard) = mm_spat (local_start, &identity);
    unwrap! (mm.wait_for_log (19., &mut |log| log.contains (">>>>>>>>> DEX stats ")));

    enable_electrum (&mm, "KMD", vec!["electrum1.cipig.net:10001"]);
    enable_electrum (&mm, "BTC", vec!["electrum1.cipig.net:10000"]);

    let autoprice = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "autoprice",
        "base": "PIZZA",
        "rel": "BEER",
        "margin": 0.5,
        // We're basing the price of our order on the price of DASH, triggering the extra price fetch in `lp_autoprice_iter`.
        // According to the examples in https://docs.komodoplatform.com/barterDEX/barterDEX-API.html the "refbase"
        // might be a lowercased coin name or it's ticker symbol (dash/DASH, litecoin/LTC, komodo/KMD).
        "refbase": "dash",
        "refrel": "coinmarketcap"
    })));
    assert! (autoprice.0.is_server_error(), "autoprice should finish with error if BTC and KMD are not enabled, bot got: {:?}", autoprice);

    enable_electrum (&mm, "BEER", vec!["electrum1.cipig.net:10022"]);
    enable_electrum (&mm, "PIZZA", vec!["electrum1.cipig.net:10024"]);

    // Looks like we don't need enabling the coin to base the price on it.
    // let electrum_dash = unwrap! (mm.rpc (json! ({
    //     "userpass": mm.userpass,
    //     "method": "electrum",
    //     "coin": "DASH",
    //     "ipaddr": "electrum1.cipig.net",
    //     "port": 10061
    // })));
    // assert_eq! (electrum_dash.0, StatusCode::OK);

    let address = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "calcaddress",
        "passphrase": passphrase
    })));
    assert_eq! (address.0, StatusCode::OK);
    let address: Json = unwrap! (json::from_str (&address.1));
    log! ({"test_autoprice] coinaddr: {}.", unwrap! (address["coinaddr"].as_str(), "!coinaddr")});

    // Trigger the autoprice.

    let autoprice = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "autoprice",
        "base": "PIZZA",
        "rel": "BEER",
        "margin": 0.5,
        // We're basing the price of our order on the price of DASH, triggering the extra price fetch in `lp_autoprice_iter`.
        "refbase": "dash",
        // The "refrel" remains the "coinmarketcap" for compatibility with the existing UIs,
        // even though without the `conf["cmc_key"]` we're using the CoinGecko instead.
        "refrel": "coinmarketcap"
    })));
    assert_eq! (autoprice.0, StatusCode::OK, "autoprice reply: {:?}", autoprice);

    // TODO: Turn into a proper (human-readable, tagged) log entry?
    unwrap! (mm.wait_for_log (9., &mut |log| log.contains ("lp_autoprice] 0 Using ref dash/coinmarketcap for PIZZA/BEER factor None")));

    unwrap! (mm.wait_for_log (44., &mut |log| log.contains ("Waiting for Bittrex market summaries... Ok.")));
    unwrap! (mm.wait_for_log (44., &mut |log| log.contains ("Waiting for Cryptopia markets... Ok.")));
    unwrap! (mm.wait_for_log (44., &mut |log| log.contains ("Waiting for coin prices (KMD, BCH, LTC)... Done!")));
    unwrap! (mm.wait_for_log (9., &mut |log| {
        log.contains ("[portfolio ext-price ref-num=0] Discovered the CoinGecko Bitcoin price of dash is 0.") ||
        log.contains ("[portfolio ext-price ref-num=0] Waiting for the CoinGecko Bitcoin price of dash ... Done")
    }));

    unwrap! (mm.stop());

    // See if `LogState` is properly dropped, which is needed in order to log the remaining dashboard entries.
    unwrap! (mm.wait_for_log (9., &mut |log| log.contains ("on_stop] firing shutdown_tx!")));
    //TODO//unwrap! (mm.wait_for_log (9., &|log| log.contains ("LogState] Bye!")));
}

/// Uses a private `conf["cmc_key"]` to test the CMC mode.
pub fn test_autoprice_coinmarketcap (local_start: LocalStart) {
    let (passphrase, mut mm, _dump_log, _dump_dashboard) = mm_spat (local_start, &|mut conf| {
        conf["cmc_key"] = "8498a278-a031-4ff1-9a7b-5f576d36149a".into();  // From "https://pro.coinmarketcap.com/account".
        // The command-line "coins" configuration is used to map the coin names to the corresponding ticker symbols.
        let coins = unwrap! (conf["coins"].as_array_mut());
        coins.push (json! ({
            "coin": "LTC",  // The ticker symbol.
            "name": "litecoin"  // The lowercased name of the coin. Should be compatible with the CoinGecko API.
        }));
        coins.push (json! ({"coin": "DASH", "name": "dash"}));
        coins.push (json! ({"coin": "KMD", "name": "komodo"}));
        // CoinGecko name is "bitcoin-cash" (https://www.coingecko.com/en/coins/bitcoin-cash), for CMC API we need to convert it to "BCH".
        coins.push (json! ({"coin": "BCH", "name": "bitcoin-cash"}));
        conf
    });
    unwrap! (mm.wait_for_log (19., &mut |log| log.contains (">>>>>>>>> DEX stats ")));

    enable_electrum (&mm, "BEER", vec!["electrum1.cipig.net:10022"]);
    enable_electrum (&mm, "PIZZA", vec!["electrum1.cipig.net:10024"]);

    let autoprice = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "autoprice",
        "base": "PIZZA",
        "rel": "BEER",
        "margin": 0.5,
        // We're basing the price of our order on the price of DASH, triggering the extra price fetch in `lp_autoprice_iter`.
        // According to the examples in https://docs.komodoplatform.com/barterDEX/barterDEX-API.html the "refbase"
        // might be a lowercased coin name or it's ticker symbol (dash/DASH, litecoin/LTC, komodo/KMD).
        "refbase": "dash",
        "refrel": "coinmarketcap"
    })));
    assert! (autoprice.0.is_server_error(), "autoprice should finish with error if BTC and KMD are not enabled, bot got: {:?}", autoprice);

    enable_electrum (&mm, "KMD", vec!["electrum1.cipig.net:10001"]);
    enable_electrum (&mm, "BTC", vec!["electrum1.cipig.net:10000"]);
    let address = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "calcaddress",
        "passphrase": passphrase
    })));
    assert_eq! (address.0, StatusCode::OK);
    let address: Json = unwrap! (json::from_str (&address.1));
    log! ({"test_autoprice] coinaddr: {}.", unwrap! (address["coinaddr"].as_str(), "!coinaddr")});

    // Trigger the autoprice.

    let autoprice = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "autoprice",
        "base": "PIZZA",
        "rel": "BEER",
        "margin": 0.5,
        // We're basing the price of our order on the price of DASH, triggering the extra price fetch in `lp_autoprice_iter`.
        // According to the examples in https://docs.komodoplatform.com/barterDEX/barterDEX-API.html the "refbase"
        // might be a lowercased coin name or it's ticker symbol (dash/DASH, litecoin/LTC, komodo/KMD).
        "refbase": "dash",
        "refrel": "coinmarketcap"
    })));
    assert_eq! (autoprice.0, StatusCode::OK, "autoprice reply: {:?}", autoprice);

    // TODO: Turn into a proper (human-readable, tagged) log entry?
    unwrap! (mm.wait_for_log (9., &|log| log.contains ("lp_autoprice] 0 Using ref dash/coinmarketcap for PIZZA/BEER factor None")));

    unwrap! (mm.wait_for_log (44., &|log| log.contains ("Waiting for Bittrex market summaries... Ok.")));
    unwrap! (mm.wait_for_log (44., &|log| log.contains ("Waiting for Cryptopia markets... Ok.")));
    unwrap! (mm.wait_for_log (44., &|log| log.contains ("Waiting for coin prices (KMD, BCH, LTC)... Done!")));
    unwrap! (mm.wait_for_log (9., &|log| {
        log.contains ("[portfolio ext-price ref-num=0] Discovered the CoinMarketCap Bitcoin price of dash is 0.") ||
        log.contains ("[portfolio ext-price ref-num=0] Waiting for the CoinMarketCap Bitcoin price of dash ... Done")
    }));

    unwrap! (mm.stop());

    // See if `LogState` is properly dropped, which is needed in order to log the remaining dashboard entries.
    unwrap! (mm.wait_for_log (9., &mut |log| log.contains ("on_stop] firing shutdown_tx!")));
    //TODO//unwrap! (mm.wait_for_log (9., &|log| log.contains ("LogState] Bye!")));
}

pub fn test_fundvalue (local_start: LocalStart) {
    let (_, mut mm, _dump_log, _dump_dashboard) = mm_spat (local_start, &identity);
    unwrap! (mm.wait_for_log (19., &|log| log.contains (">>>>>>>>> DEX stats ")));

    let fundvalue = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "fundvalue",
        "address": "RFf5mf3AoixXzmNLAmgs2L5eWGveSo6X7q",  // We have some BEER and PIZZA here.
        "holdings": [
            // Triggers the `LP_KMDvalue` code path and touches the `KMDholdings`.
            {"coin": "KMD", "balance": 123},
            // Triggers the `LP_CMCbtcprice` code path.
            {"coin": "litecoin", "balance": 123},
            // No such coin, should trigger the "no price source" part in the response.
            {"coin": "- bogus coin -", "balance": 123}
        ]
    })));
    assert! (fundvalue.0.is_server_error(), "Fundvalue must return error when BTC and KMD are not enabled, but got {:?}", fundvalue);

    enable_electrum (&mm, "KMD", vec!["electrum1.cipig.net:10001"]);
    enable_electrum (&mm, "BTC", vec!["electrum1.cipig.net:10000"]);

    let fundvalue = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "fundvalue",
        "address": "RFf5mf3AoixXzmNLAmgs2L5eWGveSo6X7q",  // We have some BEER and PIZZA here.
        "holdings": [
            // Triggers the `LP_KMDvalue` code path and touches the `KMDholdings`.
            {"coin": "KMD", "balance": 123},
            // Triggers the `LP_CMCbtcprice` code path.
            {"coin": "litecoin", "balance": 123},
            // No such coin, should trigger the "no price source" part in the response.
            {"coin": "- bogus coin -", "balance": 123}
        ]
    })));
    assert! (fundvalue.0.is_success(), "{:?}", fundvalue);
    let fundvalue: Json = unwrap! (json::from_str (&fundvalue.1));
    log! ({"fundvalue response: {}", unwrap! (json::to_string_pretty (&fundvalue))});

    // NB: Ideally we'd have `LP_balances` find the BEER and PIZZA balances we have on the "address",
    // but as of now I don't see a simple way to trigger the "importaddress" and "rescan" that seems necessary for that.

    assert! (!fundvalue["KMD_BTC"].is_null());
    assert_eq! (fundvalue["KMDholdings"].as_f64(), Some (123.));
    assert! (!fundvalue["btc2kmd"].is_null());
    assert! (!fundvalue["btcsum"].is_null());
    assert! (!fundvalue["fundvalue"].is_null());

    let holdings = unwrap! (fundvalue["holdings"].as_array());

    let kmd = unwrap! (holdings.iter().find (|en| en["coin"].as_str() == Some ("KMD")));
    assert_eq! (kmd["KMD"].as_f64(), Some (123.));

    let litecoin = unwrap! (holdings.iter().find (|en| en["coin"].as_str() == Some ("litecoin")));
    assert_eq! (litecoin["balance"].as_f64(), Some (123.));

    let bogus = unwrap! (holdings.iter().find (|en| en["coin"].as_str() == Some ("- bogus coin -")));
    assert_eq! (bogus["error"].as_str(), Some ("no price source"));

    let two_of_three = unwrap! (Regex::new (
        r"\[portfolio fundvalue ext-prices\] Waiting for prices \([\w, -]+\) ... 2 out of \d+ obtained"
    ));
    unwrap! (mm.wait_for_log (1., &|log|
        log.contains ("lp_fundvalue] LP_KMDvalue of 'KMD' is 12300000000") &&
        two_of_three.is_match (log)
    ));
}

use crate::docker_tests::docker_tests_common::{eth_distributor, generate_erc20_coin,
    generate_eth_coin_with_random_privkey};
use crate::integration_tests_common::*;
use crate::{generate_utxo_coin_with_privkey, generate_utxo_coin_with_random_privkey, random_secp256k1_secret,
SecretKey};
use coins::coin_errors::ValidatePaymentError;
use coins::utxo::UtxoCommonOps;
use coins::{FoundSwapTxSpend, MarketCoinOps, MmCoinEnum, SearchForSwapTxSpendInput, SendTakerPaymentArgs,
SendWatcherRefundsPaymentArgs, SwapOps, WatcherOps, WatcherValidateTakerFeeInput,
EARLY_CONFIRMATION_ERR_LOG, INVALID_RECEIVER_ERR_LOG, INVALID_SENDER_ERR_LOG, OLD_TRANSACTION_ERR_LOG};
use common::{block_on, now_ms, DEX_FEE_ADDR_RAW_PUBKEY};
use futures01::Future;
use mm2_main::mm2::lp_swap::{dex_fee_amount_from_taker_coin, MAKER_PAYMENT_SENT_LOG, MAKER_PAYMENT_SPEND_FOUND_LOG,
MAKER_PAYMENT_SPEND_SENT_LOG, TAKER_PAYMENT_REFUND_SENT_LOG, WATCHER_MESSAGE_SENT_LOG};
use mm2_number::MmNumber;
use mm2_test_helpers::for_tests::{enable_eth_coin, eth_jst_conf, eth_testnet_conf, mm_dump, mycoin1_conf, mycoin_conf,
start_swaps, MarketMakerIt, Mm2TestConf, ETH_SEPOLIA_NODE,
ETH_SEPOLIA_SWAP_CONTRACT, ETH_SEPOLIA_TOKEN_CONTRACT};
use mm2_test_helpers::get_passphrase;
use num_traits::Pow;
use serde_json::Value as Json;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

fn round(num: f64, digits: u8) -> f64 {
let digits: f64 = (10.).pow(digits);
(digits * num).round() / digits
}

fn enable_eth_and_jst(mm_node: &MarketMakerIt) {
dbg!(block_on(enable_eth_coin(
mm_node,
"ETH",
ETH_SEPOLIA_NODE,
ETH_SEPOLIA_SWAP_CONTRACT,
Some(ETH_SEPOLIA_SWAP_CONTRACT)
)));

dbg!(block_on(enable_eth_coin(
mm_node,
"JST",
ETH_SEPOLIA_NODE,
ETH_SEPOLIA_SWAP_CONTRACT,
Some(ETH_SEPOLIA_SWAP_CONTRACT)
)));
}

fn get_balance(mm_node: &MarketMakerIt, ticker: &str) -> String {
let rc = block_on(mm_node.rpc(&json!({
"userpass": mm_node.userpass,
"method": "my_balance",
"coin": ticker
})))
.unwrap();
assert!(rc.0.is_success(), "!my_balance: {}", rc.1);

let json: Json = serde_json::from_str(&rc.1).unwrap();
json["balance"].as_str().unwrap().to_string()
}

fn get_balance_f64(mm_node: &MarketMakerIt, ticker: &str) -> f64 {
get_balance(mm_node, ticker).parse::<f64>().unwrap()
}

#[test]
fn test_watcher_spends_maker_payment_spend_eth_erc20() {
let coins = json!([eth_testnet_conf(), eth_jst_conf(ETH_SEPOLIA_TOKEN_CONTRACT)]);

let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
let alice_conf = Mm2TestConf::seednode_using_watchers(&alice_passphrase, &coins);
let mut mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password.clone(), None).unwrap();
let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
log!("Alice log path: {}", mm_alice.log_path.display());

let bob_passphrase = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
let bob_conf = Mm2TestConf::light_node(&bob_passphrase, &coins, &[&mm_alice.ip.to_string()]);
let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
log!("Bob log path: {}", mm_bob.log_path.display());

let watcher_passphrase =
String::from("also shoot benefit prefer juice shell thank unfair canal monkey style afraid");
let watcher_conf = Mm2TestConf::watcher_light_node(
&watcher_passphrase,
&coins,
&[&mm_alice.ip.to_string()],
0.,
1.5,
1.,
0.,
)
.conf;
let mut mm_watcher = MarketMakerIt::start(watcher_conf, "pass".to_string(), None).unwrap();
let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

enable_eth_and_jst(&mm_alice);
enable_eth_and_jst(&mm_bob);
enable_eth_and_jst(&mm_watcher);

let alice_eth_balance_before = get_balance_f64(&mm_alice, "ETH");
let alice_jst_balance_before = get_balance_f64(&mm_alice, "JST");
let bob_eth_balance_before = get_balance_f64(&mm_bob, "ETH");
let bob_jst_balance_before = get_balance_f64(&mm_bob, "JST");

block_on(start_swaps(&mut mm_bob, &mut mm_alice, &[("ETH", "JST")], 1., 1., 0.01));

block_on(mm_alice.wait_for_log(180., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
block_on(mm_alice.stop()).unwrap();
block_on(mm_watcher.wait_for_log(180., |log| log.contains(MAKER_PAYMENT_SPEND_SENT_LOG))).unwrap();
thread::sleep(Duration::from_secs(25));

let mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password.clone(), None).unwrap();
enable_eth_and_jst(&mm_alice);

let alice_eth_balance_after = get_balance_f64(&mm_alice, "ETH");
let alice_jst_balance_after = get_balance_f64(&mm_alice, "JST");
let bob_eth_balance_after = get_balance_f64(&mm_bob, "ETH");
let bob_jst_balance_after = get_balance_f64(&mm_bob, "JST");

assert_eq!(
round(alice_jst_balance_before - 0.01, 2),
round(alice_jst_balance_after, 2)
);
assert_eq!(round(bob_jst_balance_before + 0.01, 2), round(bob_jst_balance_after, 2));
assert_eq!(
round(alice_eth_balance_before + 0.01, 2),
round(alice_eth_balance_after, 2)
);
assert_eq!(round(bob_eth_balance_before - 0.01, 2), round(bob_eth_balance_after, 2));
}

#[test]
fn test_watcher_spends_maker_payment_spend_erc20_eth() {
let coins = json!([eth_testnet_conf(), eth_jst_conf(ETH_SEPOLIA_TOKEN_CONTRACT)]);

let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
let alice_conf = Mm2TestConf::seednode_using_watchers(&alice_passphrase, &coins);
let mut mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password.clone(), None).unwrap();
let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
log!("Alice log path: {}", mm_alice.log_path.display());

let bob_passphrase = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
let bob_conf = Mm2TestConf::light_node(&bob_passphrase, &coins, &[&mm_alice.ip.to_string()]);
let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
log!("Bob log path: {}", mm_bob.log_path.display());

let watcher_passphrase =
String::from("also shoot benefit prefer juice shell thank unfair canal monkey style afraid");
let watcher_conf = Mm2TestConf::watcher_light_node(
&watcher_passphrase,
&coins,
&[&mm_alice.ip.to_string()],
0.,
1.5,
1.,
0.,
)
.conf;

let mut mm_watcher = MarketMakerIt::start(watcher_conf, "pass".to_string(), None).unwrap();
let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

enable_eth_and_jst(&mm_alice);
enable_eth_and_jst(&mm_bob);
enable_eth_and_jst(&mm_watcher);

let alice_eth_balance_before = get_balance_f64(&mm_alice, "ETH");
let alice_jst_balance_before = get_balance_f64(&mm_alice, "JST");
let bob_eth_balance_before = get_balance_f64(&mm_bob, "ETH");
let bob_jst_balance_before = get_balance_f64(&mm_bob, "JST");

block_on(start_swaps(&mut mm_bob, &mut mm_alice, &[("JST", "ETH")], 1., 1., 0.01));

block_on(mm_alice.wait_for_log(180., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
block_on(mm_alice.stop()).unwrap();
block_on(mm_watcher.wait_for_log(180., |log| log.contains(MAKER_PAYMENT_SPEND_SENT_LOG))).unwrap();
thread::sleep(Duration::from_secs(25));

let mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password.clone(), None).unwrap();
enable_eth_and_jst(&mm_alice);

let alice_eth_balance_after = get_balance_f64(&mm_alice, "ETH");
let alice_jst_balance_after = get_balance_f64(&mm_alice, "JST");
let bob_eth_balance_after = get_balance_f64(&mm_bob, "ETH");
let bob_jst_balance_after = get_balance_f64(&mm_bob, "JST");

assert_eq!(
round(alice_jst_balance_before + 0.01, 2),
round(alice_jst_balance_after, 2)
);
assert_eq!(round(bob_jst_balance_before - 0.01, 2), round(bob_jst_balance_after, 2));
assert_eq!(
round(alice_eth_balance_before - 0.01, 2),
round(alice_eth_balance_after, 2)
);
assert_eq!(round(bob_eth_balance_before + 0.01, 2), round(bob_eth_balance_after, 2));
}

#[test]
fn test_watcher_refunds_taker_payment_erc20() {
let coins = json!([eth_testnet_conf(), eth_jst_conf(ETH_SEPOLIA_TOKEN_CONTRACT)]);

let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
let alice_conf = Mm2TestConf::seednode_using_watchers(&alice_passphrase, &coins);
let mut mm_alice = block_on(MarketMakerIt::start_with_envs(
alice_conf.conf.clone(),
alice_conf.rpc_password.clone(),
None,
&[("USE_TEST_LOCKTIME", "")],
))
.unwrap();
let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
log!("Alice log path: {}", mm_alice.log_path.display());

let bob_passphrase = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
let bob_conf = Mm2TestConf::light_node(&bob_passphrase, &coins, &[&mm_alice.ip.to_string()]);
let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
log!("Bob log path: {}", mm_bob.log_path.display());

let watcher_passphrase =
String::from("also shoot benefit prefer juice shell thank unfair canal monkey style afraid");
let watcher_conf =
Mm2TestConf::watcher_light_node(&watcher_passphrase, &coins, &[&mm_alice.ip.to_string()], 1., 0., 1., 0.).conf;
let mut mm_watcher = block_on(MarketMakerIt::start_with_envs(
watcher_conf,
"pass".to_string(),
None,
&[("REFUND_TEST", "")],
))
.unwrap();
let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

enable_eth_and_jst(&mm_alice);
enable_eth_and_jst(&mm_bob);
enable_eth_and_jst(&mm_watcher);

let alice_eth_balance_before = get_balance_f64(&mm_alice, "ETH");
let alice_jst_balance_before = get_balance_f64(&mm_alice, "JST");

block_on(start_swaps(&mut mm_bob, &mut mm_alice, &[("ETH", "JST")], 1., 1., 0.01));

block_on(mm_bob.wait_for_log(160., |log| log.contains(MAKER_PAYMENT_SENT_LOG))).unwrap();
block_on(mm_bob.stop()).unwrap();
block_on(mm_alice.wait_for_log(160., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
block_on(mm_alice.stop()).unwrap();
block_on(mm_watcher.wait_for_log(160., |log| log.contains(TAKER_PAYMENT_REFUND_SENT_LOG))).unwrap();
thread::sleep(Duration::from_secs(25));

let mm_alice = MarketMakerIt::start(alice_conf.conf, alice_conf.rpc_password, None).unwrap();
enable_eth_and_jst(&mm_alice);

let alice_eth_balance_after = get_balance_f64(&mm_alice, "ETH");
let alice_jst_balance_after = get_balance_f64(&mm_alice, "JST");

assert_eq!(round(alice_jst_balance_before, 2), round(alice_jst_balance_after, 2));
assert_eq!(round(alice_eth_balance_before, 2), round(alice_eth_balance_after, 2));
}

#[test]
fn test_watcher_refunds_taker_payment_eth() {
let coins = json!([eth_testnet_conf(), eth_jst_conf(ETH_SEPOLIA_TOKEN_CONTRACT)]);

let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
let alice_conf = Mm2TestConf::seednode_using_watchers(&alice_passphrase, &coins);
let mut mm_alice = block_on(MarketMakerIt::start_with_envs(
alice_conf.conf.clone(),
alice_conf.rpc_password.clone(),
None,
&[("USE_TEST_LOCKTIME", "")],
))
.unwrap();
let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
log!("Alice log path: {}", mm_alice.log_path.display());

let bob_passphrase = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
let bob_conf = Mm2TestConf::light_node(&bob_passphrase, &coins, &[&mm_alice.ip.to_string()]);
let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
log!("Bob log path: {}", mm_bob.log_path.display());

let watcher_passphrase =
String::from("also shoot benefit prefer juice shell thank unfair canal monkey style afraid");
let watcher_conf =
Mm2TestConf::watcher_light_node(&watcher_passphrase, &coins, &[&mm_alice.ip.to_string()], 1., 0., 1., 0.).conf;
let mut mm_watcher = block_on(MarketMakerIt::start_with_envs(
watcher_conf,
"pass".to_string(),
None,
&[("REFUND_TEST", "")],
))
.unwrap();
let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

enable_eth_and_jst(&mm_alice);
enable_eth_and_jst(&mm_bob);
enable_eth_and_jst(&mm_watcher);

let alice_eth_balance_before = get_balance_f64(&mm_alice, "ETH");
let alice_jst_balance_before = get_balance_f64(&mm_alice, "JST");

block_on(start_swaps(&mut mm_bob, &mut mm_alice, &[("JST", "ETH")], 1., 1., 0.01));

block_on(mm_bob.wait_for_log(160., |log| log.contains(MAKER_PAYMENT_SENT_LOG))).unwrap();
block_on(mm_bob.stop()).unwrap();
block_on(mm_alice.wait_for_log(160., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
block_on(mm_alice.stop()).unwrap();
block_on(mm_watcher.wait_for_log(160., |log| log.contains(TAKER_PAYMENT_REFUND_SENT_LOG))).unwrap();
thread::sleep(Duration::from_secs(25));

let mm_alice = MarketMakerIt::start(alice_conf.conf, alice_conf.rpc_password, None).unwrap();
enable_eth_and_jst(&mm_alice);

let alice_eth_balance_after = get_balance_f64(&mm_alice, "ETH");
let alice_jst_balance_after = get_balance_f64(&mm_alice, "JST");

assert_eq!(round(alice_jst_balance_before, 2), round(alice_jst_balance_after, 2));
assert_eq!(round(alice_eth_balance_before, 2), round(alice_eth_balance_after, 2));
}

#[test]
fn test_watcher_validate_taker_fee_eth() {
let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run

let taker_coin = eth_distributor();
let taker_keypair = taker_coin.derive_htlc_key_pair(&[]);
let taker_pubkey = taker_keypair.public();

let maker_coin = generate_eth_coin_with_random_privkey();
let maker_keypair = maker_coin.derive_htlc_key_pair(&[]);
let maker_pubkey = maker_keypair.public();

let taker_amount = MmNumber::from((10, 1));
let fee_amount = dex_fee_amount_from_taker_coin(
&MmCoinEnum::EthCoin(taker_coin.clone()),
maker_coin.ticker(),
&taker_amount,
);
let uuid = Uuid::parse_str("936DA01F-9ABD-4D9D-80C7-02AF85C822A8").unwrap();
let taker_fee = taker_coin
.send_taker_fee(&DEX_FEE_ADDR_RAW_PUBKEY, fee_amount.clone().into(), uuid.as_bytes())
.wait()
.unwrap();

taker_coin
.wait_for_confirmations(&taker_fee.tx_hex(), 1, false, timeout, 1)
.wait()
.unwrap();

let validate_taker_fee_res = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: 0,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 7800,
})
.wait();
assert!(validate_taker_fee_res.is_ok());

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: maker_pubkey.to_vec(),
min_block_number: 0,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 7800,
})
.wait()
.unwrap_err()
.into_inner();

log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(INVALID_SENDER_ERR_LOG))
},
_ => panic!("Expected `WrongPaymentTx` invalid public key, found {:?}", error),
}

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: std::u64::MAX,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 7800,
})
.wait()
.unwrap_err()
.into_inner();
log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(EARLY_CONFIRMATION_ERR_LOG))
},
_ => panic!(
"Expected `WrongPaymentTx` confirmed before min_block, found {:?}",
error
),
}

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: 0,
fee_addr: taker_pubkey.to_vec(),
lock_duration: 7800,
})
.wait()
.unwrap_err()
.into_inner();
log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(INVALID_RECEIVER_ERR_LOG))
},
_ => panic!(
"Expected `WrongPaymentTx` tx output script_pubkey doesn't match expected, found {:?}",
error
),
}
}

#[test]
fn test_watcher_validate_taker_fee_erc20() {
let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run

let taker_coin = generate_erc20_coin();
let taker_keypair = taker_coin.derive_htlc_key_pair(&[]);
let taker_pubkey = taker_keypair.public();

let maker_coin = generate_eth_coin_with_random_privkey();
let maker_keypair = maker_coin.derive_htlc_key_pair(&[]);
let maker_pubkey = maker_keypair.public();

let taker_amount = MmNumber::from((10, 1));
let fee_amount = dex_fee_amount_from_taker_coin(
&MmCoinEnum::EthCoin(taker_coin.clone()),
maker_coin.ticker(),
&taker_amount,
);
let uuid = Uuid::parse_str("936DA01F-9ABD-4D9D-80C7-02AF85C822A8").unwrap();
let taker_fee = taker_coin
.send_taker_fee(&DEX_FEE_ADDR_RAW_PUBKEY, fee_amount.clone().into(), uuid.as_bytes())
.wait()
.unwrap();

taker_coin
.wait_for_confirmations(&taker_fee.tx_hex(), 1, false, timeout, 1)
.wait()
.unwrap();

let validate_taker_fee_res = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: 0,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 7800,
})
.wait();
assert!(validate_taker_fee_res.is_ok());

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: maker_pubkey.to_vec(),
min_block_number: 0,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 7800,
})
.wait()
.unwrap_err()
.into_inner();

log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(INVALID_SENDER_ERR_LOG))
},
_ => panic!("Expected `WrongPaymentTx` invalid public key, found {:?}", error),
}

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: std::u64::MAX,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 7800,
})
.wait()
.unwrap_err()
.into_inner();
log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(EARLY_CONFIRMATION_ERR_LOG))
},
_ => panic!(
"Expected `WrongPaymentTx` confirmed before min_block, found {:?}",
error
),
}

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: 0,
fee_addr: taker_pubkey.to_vec(),
lock_duration: 7800,
})
.wait()
.unwrap_err()
.into_inner();
log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(INVALID_RECEIVER_ERR_LOG))
},
_ => panic!(
"Expected `WrongPaymentTx` tx output script_pubkey doesn't match expected, found {:?}",
error
),
}
}

#[test]
fn test_watcher_spends_maker_payment_spend_utxo() {
let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 100.into());
generate_utxo_coin_with_privkey("MYCOIN1", 100.into(), bob_priv_key);
let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 100.into());
generate_utxo_coin_with_privkey("MYCOIN", 100.into(), alice_priv_key);

let watcher_priv_key = random_secp256k1_secret();

let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

let alice_conf = Mm2TestConf::seednode_using_watchers(&format!("0x{}", hex::encode(alice_priv_key)), &coins).conf;
let mut mm_alice = MarketMakerIt::start(alice_conf.clone(), "pass".to_string(), None).unwrap();
let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

let bob_conf = Mm2TestConf::light_node(&format!("0x{}", hex::encode(bob_priv_key)), &coins, &[&mm_alice
.ip
.to_string()])
.conf;
let mm_bob = MarketMakerIt::start(bob_conf, "pass".to_string(), None).unwrap();
let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

let watcher_conf = Mm2TestConf::watcher_light_node(
&format!("0x{}", hex::encode(watcher_priv_key)),
&coins,
&[&mm_alice.ip.to_string()],
0.,
1.5,
1.,
0.,
)
.conf;
let mut mm_watcher = MarketMakerIt::start(watcher_conf, "pass".to_string(), None).unwrap();
let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[])));
log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));
log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN1", &[])));

let rc = block_on(mm_bob.rpc(&json!({
"userpass": mm_bob.userpass,
"method": "setprice",
"base": "MYCOIN",
"rel": "MYCOIN1",
"price": 25,
"max": true,
})))
.unwrap();
assert!(rc.0.is_success(), "!setprice: {}", rc.1);

let rc = block_on(mm_alice.rpc(&json!({
"userpass": mm_alice.userpass,
"method": "buy",
"base": "MYCOIN",
"rel": "MYCOIN1",
"price": 25,
"volume": "2",
})))
.unwrap();
assert!(rc.0.is_success(), "!buy: {}", rc.1);

block_on(mm_alice.wait_for_log(60., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
block_on(mm_alice.stop()).unwrap();
block_on(mm_watcher.wait_for_log(60., |log| log.contains(MAKER_PAYMENT_SPEND_SENT_LOG))).unwrap();
thread::sleep(Duration::from_secs(5));

let mm_alice = MarketMakerIt::start(alice_conf, "pass".to_string(), None).unwrap();
let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));

let alice_mycoin1_balance = get_balance(&mm_alice, "MYCOIN1");
assert_eq!(alice_mycoin1_balance, "49.93562994");

let alice_mycoin_balance = get_balance(&mm_alice, "MYCOIN");
assert_eq!(alice_mycoin_balance, "101.99999");

let bob_mycoin1_balance = get_balance(&mm_bob, "MYCOIN1");
assert_eq!(bob_mycoin1_balance, "149.99999");

let bob_mycoin_balance = get_balance(&mm_bob, "MYCOIN");
assert_eq!(bob_mycoin_balance, "97.99999");
}

#[test]
fn test_watcher_waits_for_taker_utxo() {
let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 100.into());
generate_utxo_coin_with_privkey("MYCOIN1", 100.into(), bob_priv_key);
let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 100.into());
generate_utxo_coin_with_privkey("MYCOIN", 100.into(), alice_priv_key);
let watcher_priv_key = *SecretKey::new(&mut rand6::thread_rng()).as_ref();

let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

let alice_conf = Mm2TestConf::seednode_using_watchers(&format!("0x{}", hex::encode(alice_priv_key)), &coins).conf;
let mm_alice = MarketMakerIt::start(alice_conf.clone(), "pass".to_string(), None).unwrap();
let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

let bob_conf = Mm2TestConf::light_node(&format!("0x{}", hex::encode(bob_priv_key)), &coins, &[&mm_alice
.ip
.to_string()])
.conf;
let mm_bob = MarketMakerIt::start(bob_conf, "pass".to_string(), None).unwrap();
let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

let watcher_conf = Mm2TestConf::watcher_light_node(
&format!("0x{}", hex::encode(watcher_priv_key)),
&coins,
&[&mm_alice.ip.to_string()],
1.,
1.5,
1.,
0.,
)
.conf;
let mut mm_watcher = MarketMakerIt::start(watcher_conf, "pass".to_string(), None).unwrap();
let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[])));
log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));
log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN1", &[])));

let rc = block_on(mm_bob.rpc(&json!({
"userpass": mm_bob.userpass,
"method": "setprice",
"base": "MYCOIN",
"rel": "MYCOIN1",
"price": 25,
"max": true,
})))
.unwrap();
assert!(rc.0.is_success(), "!setprice: {}", rc.1);

let rc = block_on(mm_alice.rpc(&json!({
"userpass": mm_alice.userpass,
"method": "buy",
"base": "MYCOIN",
"rel": "MYCOIN1",
"price": 25,
"volume": "2",
})))
.unwrap();
assert!(rc.0.is_success(), "!buy: {}", rc.1);

block_on(mm_watcher.wait_for_log(160., |log| log.contains(MAKER_PAYMENT_SPEND_FOUND_LOG))).unwrap();
}

#[test]
fn test_watcher_refunds_taker_payment_utxo() {
let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 100.into());
generate_utxo_coin_with_privkey("MYCOIN1", 100.into(), bob_priv_key);
let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 100.into());
generate_utxo_coin_with_privkey("MYCOIN", 100.into(), alice_priv_key);
let watcher_priv_key = *SecretKey::new(&mut rand6::thread_rng()).as_ref();

let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

let alice_conf = Mm2TestConf::seednode_using_watchers(&format!("0x{}", hex::encode(alice_priv_key)), &coins).conf;
let mut mm_alice = block_on(MarketMakerIt::start_with_envs(
alice_conf.clone(),
"pass".to_string(),
None,
&[("USE_TEST_LOCKTIME", "")],
))
.unwrap();
let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

let bob_conf = Mm2TestConf::light_node(&format!("0x{}", hex::encode(bob_priv_key)), &coins, &[&mm_alice
.ip
.to_string()])
.conf;
let mut mm_bob = MarketMakerIt::start(bob_conf, "pass".to_string(), None).unwrap();
let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

let watcher_conf = Mm2TestConf::watcher_light_node(
&format!("0x{}", hex::encode(watcher_priv_key)),
&coins,
&[&mm_alice.ip.to_string()],
1.,
0.,
1.,
0.,
)
.conf;
let mut mm_watcher = block_on(MarketMakerIt::start_with_envs(
watcher_conf,
"pass".to_string(),
None,
&[("REFUND_TEST", "")],
))
.unwrap();
let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[])));
log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));
log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN1", &[])));

let rc = block_on(mm_bob.rpc(&json!({
"userpass": mm_bob.userpass,
"method": "setprice",
"base": "MYCOIN",
"rel": "MYCOIN1",
"price": 25,
"max": true,
})))
.unwrap();
assert!(rc.0.is_success(), "!setprice: {}", rc.1);

let rc = block_on(mm_alice.rpc(&json!({
"userpass": mm_alice.userpass,
"method": "buy",
"base": "MYCOIN",
"rel": "MYCOIN1",
"price": 25,
"volume": "2",
})))
.unwrap();
assert!(rc.0.is_success(), "!buy: {}", rc.1);

block_on(mm_bob.wait_for_log(160., |log| log.contains(MAKER_PAYMENT_SENT_LOG))).unwrap();
block_on(mm_bob.stop()).unwrap();
block_on(mm_alice.wait_for_log(160., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
block_on(mm_alice.stop()).unwrap();
block_on(mm_watcher.wait_for_log(160., |log| log.contains(TAKER_PAYMENT_REFUND_SENT_LOG))).unwrap();
thread::sleep(Duration::from_secs(5));

let mm_alice = MarketMakerIt::start(alice_conf, "pass".to_string(), None).unwrap();
let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));

let alice_mycoin1_balance = get_balance(&mm_alice, "MYCOIN1");
assert_eq!(alice_mycoin1_balance, "99.93561994");

let alice_mycoin_balance = get_balance(&mm_alice, "MYCOIN");
assert_eq!(alice_mycoin_balance, "100");
}

#[test]
fn test_watcher_validate_taker_fee_utxo() {
let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
let (_ctx, taker_coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
let (_ctx, maker_coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
let taker_pubkey = taker_coin.my_public_key().unwrap();

let taker_amount = MmNumber::from((10, 1));
let fee_amount = dex_fee_amount_from_taker_coin(
&MmCoinEnum::UtxoCoin(taker_coin.clone()),
maker_coin.ticker(),
&taker_amount,
);
let uuid = Uuid::parse_str("936DA01F-9ABD-4D9D-80C7-02AF85C822A8").unwrap();

let taker_fee = taker_coin
.send_taker_fee(&DEX_FEE_ADDR_RAW_PUBKEY, fee_amount.clone().into(), uuid.as_bytes())
.wait()
.unwrap();

taker_coin
.wait_for_confirmations(&taker_fee.tx_hex(), 1, false, timeout, 1)
.wait()
.unwrap();

let validate_taker_fee_res = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: 0,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 7800,
})
.wait();
assert!(validate_taker_fee_res.is_ok());

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: maker_coin.my_public_key().unwrap().to_vec(),
min_block_number: 0,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 7800,
})
.wait()
.unwrap_err()
.into_inner();

log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(INVALID_SENDER_ERR_LOG))
},
_ => panic!("Expected `WrongPaymentTx` invalid public key, found {:?}", error),
}

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: std::u64::MAX,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 7800,
})
.wait()
.unwrap_err()
.into_inner();
log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(EARLY_CONFIRMATION_ERR_LOG))
},
_ => panic!(
"Expected `WrongPaymentTx` confirmed before min_block, found {:?}",
error
),
}

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: 0,
fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
lock_duration: 0,
})
.wait()
.unwrap_err()
.into_inner();
log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(OLD_TRANSACTION_ERR_LOG))
},
_ => panic!("Expected `WrongPaymentTx` transaction too old, found {:?}", error),
}

let error = taker_coin
.watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
taker_fee_hash: taker_fee.tx_hash().into_vec(),
sender_pubkey: taker_pubkey.to_vec(),
min_block_number: 0,
fee_addr: taker_pubkey.to_vec(),
lock_duration: 7800,
})
.wait()
.unwrap_err()
.into_inner();
log!("error: {:?}", error);
match error {
ValidatePaymentError::WrongPaymentTx(err) => {
assert!(err.contains(INVALID_RECEIVER_ERR_LOG))
},
_ => panic!(
"Expected `WrongPaymentTx` tx output script_pubkey doesn't match expected, found {:?}",
error
),
}
}

#[test]
fn test_send_taker_payment_refund_preimage_utxo() {
let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
let my_public_key = coin.my_public_key().unwrap();

let time_lock = (now_ms() / 1000) as u32 - 3600;
let taker_payment_args = SendTakerPaymentArgs {
time_lock_duration: 0,
time_lock,
other_pubkey: my_public_key,
secret_hash: &[0; 20],
amount: 1u64.into(),
swap_contract_address: &None,
swap_unique_data: &[],
payment_instructions: &None,
};
let tx = coin.send_taker_payment(taker_payment_args).wait().unwrap();

coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1)
.wait()
.unwrap();

let refund_tx = coin
.create_taker_payment_refund_preimage(&tx.tx_hex(), time_lock, my_public_key, &[0; 20], &None, &[])
.wait()
.unwrap();

let refund_tx = coin
.send_taker_payment_refund_preimage(SendWatcherRefundsPaymentArgs {
payment_tx: &refund_tx.tx_hex(),
swap_contract_address: &None,
secret_hash: &[0; 20],
other_pubkey: my_public_key,
time_lock,
swap_unique_data: &[],
})
.wait()
.unwrap();

coin.wait_for_confirmations(&refund_tx.tx_hex(), 1, false, timeout, 1)
.wait()
.unwrap();

let search_input = SearchForSwapTxSpendInput {
time_lock,
other_pub: &*coin.my_public_key().unwrap(),
secret_hash: &[0; 20],
tx: &tx.tx_hex(),
search_from_block: 0,
swap_contract_address: &None,
swap_unique_data: &[],
};
let found = block_on(coin.search_for_swap_tx_spend_my(search_input))
.unwrap()
.unwrap();
assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
}
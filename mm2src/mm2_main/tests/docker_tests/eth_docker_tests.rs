use crate::docker_tests::docker_tests_common::{random_secp256k1_secret, GETH_ACCOUNT, GETH_ERC20_CONTRACT,
                                               GETH_NONCE_LOCK, GETH_SWAP_CONTRACT, GETH_WATCHERS_SWAP_CONTRACT,
                                               GETH_WEB3, MM_CTX};
use bitcrypto::dhash160;
use coins::eth::{checksum_address, eth_coin_from_conf_and_request, EthCoin, ERC20_ABI};
use coins::{CoinProtocol, ConfirmPaymentInput, FoundSwapTxSpend, MarketCoinOps, PrivKeyBuildPolicy, RefundPaymentArgs,
            SearchForSwapTxSpendInput, SendPaymentArgs, SpendPaymentArgs, SwapOps, SwapTxTypeWithSecretHash};
use common::{block_on, now_sec};
use ethereum_types::U256;
use futures01::Future;
use mm2_test_helpers::for_tests::{erc20_dev_conf, eth_dev_conf};
use std::thread;
use std::time::Duration;
use web3::contract::{Contract, Options};
use web3::ethabi::Token;
use web3::types::{Address, TransactionRequest, H256};

/// # Safety
///
/// GETH_ACCOUNT is set once during initialization before tests start
fn geth_account() -> Address { unsafe { GETH_ACCOUNT } }

/// # Safety
///
/// GETH_SWAP_CONTRACT is set once during initialization before tests start
pub fn swap_contract() -> Address { unsafe { GETH_SWAP_CONTRACT } }

/// # Safety
///
/// GETH_WATCHERS_SWAP_CONTRACT is set once during initialization before tests start
pub fn watchers_swap_contract() -> Address { unsafe { GETH_WATCHERS_SWAP_CONTRACT } }

/// # Safety
///
/// GETH_ERC20_CONTRACT is set once during initialization before tests start
pub fn erc20_contract() -> Address { unsafe { GETH_ERC20_CONTRACT } }

/// Return ERC20 dev token contract address in checksum format
pub fn erc20_contract_checksum() -> String { checksum_address(&format!("{:02x}", erc20_contract())) }

fn wait_for_confirmation(tx_hash: H256) {
    loop {
        match block_on(GETH_WEB3.eth().transaction_receipt(tx_hash)) {
            Ok(Some(r)) => match r.block_hash {
                Some(_) => break,
                None => thread::sleep(Duration::from_millis(100)),
            },
            _ => {
                thread::sleep(Duration::from_millis(100));
            },
        }
    }
}

pub fn fill_eth(to_addr: Address, amount: U256) {
    let _guard = GETH_NONCE_LOCK.lock().unwrap();
    let tx_request = TransactionRequest {
        from: geth_account(),
        to: Some(to_addr),
        gas: None,
        gas_price: None,
        value: Some(amount),
        data: None,
        nonce: None,
        condition: None,
        transaction_type: None,
        access_list: None,
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
    };
    let tx_hash = block_on(GETH_WEB3.eth().send_transaction(tx_request)).unwrap();
    wait_for_confirmation(tx_hash);
}

fn fill_erc20(to_addr: Address, amount: U256) {
    let _guard = GETH_NONCE_LOCK.lock().unwrap();
    let erc20_contract = Contract::from_json(GETH_WEB3.eth(), erc20_contract(), ERC20_ABI.as_bytes()).unwrap();

    let tx_hash = block_on(erc20_contract.call(
        "transfer",
        (Token::Address(to_addr), Token::Uint(amount)),
        geth_account(),
        Options::default(),
    ))
    .unwrap();
    wait_for_confirmation(tx_hash);
}

/// Creates ETH protocol coin supplied with 100 ETH
pub fn eth_coin_with_random_privkey(swap_contract: Address) -> EthCoin {
    let eth_conf = eth_dev_conf();
    let req = json!({
        "method": "enable",
        "coin": "ETH",
        "urls": ["http://127.0.0.1:8545"],
        "swap_contract_address": swap_contract,
    });

    let secret = random_secp256k1_secret();
    let eth_coin = block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ETH",
        &eth_conf,
        &req,
        CoinProtocol::ETH,
        PrivKeyBuildPolicy::IguanaPrivKey(secret),
    ))
    .unwrap();

    // 100 ETH
    fill_eth(eth_coin.my_address, U256::from(10).pow(U256::from(20)));

    eth_coin
}

/// Creates ERC20 protocol coin supplied with 1 ETH and 100 token
pub fn erc20_coin_with_random_privkey(swap_contract: Address) -> EthCoin {
    let erc20_conf = erc20_dev_conf(&erc20_contract_checksum());
    let req = json!({
        "method": "enable",
        "coin": "ERC20DEV",
        "urls": ["http://127.0.0.1:8545"],
        "swap_contract_address": swap_contract,
    });

    let erc20_coin = block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ERC20DEV",
        &erc20_conf,
        &req,
        CoinProtocol::ERC20 {
            platform: "ETH".to_string(),
            contract_address: checksum_address(&format!("{:02x}", erc20_contract())),
        },
        PrivKeyBuildPolicy::IguanaPrivKey(random_secp256k1_secret()),
    ))
    .unwrap();

    // 1 ETH
    fill_eth(erc20_coin.my_address, U256::from(10).pow(U256::from(18)));
    // 100 tokens (it has 8 decimals)
    fill_erc20(erc20_coin.my_address, U256::from(10000000000u64));

    erc20_coin
}

#[test]
fn send_and_refund_eth_maker_payment() {
    let eth_coin = eth_coin_with_random_privkey(swap_contract());

    let time_lock = now_sec() - 100;
    let other_pubkey = &[
        0x02, 0xc6, 0x6e, 0x7d, 0x89, 0x66, 0xb5, 0xc5, 0x55, 0xaf, 0x58, 0x05, 0x98, 0x9d, 0xa9, 0xfb, 0xf8, 0xdb,
        0x95, 0xe1, 0x56, 0x31, 0xce, 0x35, 0x8c, 0x3a, 0x17, 0x10, 0xc9, 0x62, 0x67, 0x90, 0x63,
    ];

    let send_payment_args = SendPaymentArgs {
        time_lock_duration: 100,
        time_lock,
        other_pubkey,
        secret_hash: &[0; 20],
        amount: 1.into(),
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let eth_maker_payment = eth_coin.send_maker_payment(send_payment_args).wait().unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: eth_maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let refund_args = RefundPaymentArgs {
        payment_tx: &eth_maker_payment.tx_hex(),
        time_lock,
        other_pubkey,
        tx_type_with_secret_hash: SwapTxTypeWithSecretHash::TakerOrMakerPayment {
            maker_secret_hash: &[0; 20],
        },
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let payment_refund = block_on(eth_coin.send_maker_refunds_payment(refund_args)).unwrap();
    println!("Payment refund tx hash {:02x}", payment_refund.tx_hash());

    let confirm_input = ConfirmPaymentInput {
        payment_tx: payment_refund.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: other_pubkey,
        secret_hash: &[0; 20],
        tx: &eth_maker_payment.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let search_tx = block_on(eth_coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();

    let expected = FoundSwapTxSpend::Refunded(payment_refund);
    assert_eq!(expected, search_tx);
}

#[test]
fn send_and_spend_eth_maker_payment() {
    let maker_eth_coin = eth_coin_with_random_privkey(swap_contract());
    let taker_eth_coin = eth_coin_with_random_privkey(swap_contract());

    let time_lock = now_sec() + 1000;
    let maker_pubkey = maker_eth_coin.derive_htlc_pubkey(&[]);
    let taker_pubkey = taker_eth_coin.derive_htlc_pubkey(&[]);
    let secret = &[1; 32];
    let secret_hash_owned = dhash160(secret);
    let secret_hash = secret_hash_owned.as_slice();

    let send_payment_args = SendPaymentArgs {
        time_lock_duration: 1000,
        time_lock,
        other_pubkey: &taker_pubkey,
        secret_hash,
        amount: 1.into(),
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let eth_maker_payment = maker_eth_coin.send_maker_payment(send_payment_args).wait().unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: eth_maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    taker_eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let spend_args = SpendPaymentArgs {
        other_payment_tx: &eth_maker_payment.tx_hex(),
        time_lock,
        other_pubkey: &maker_pubkey,
        secret,
        secret_hash,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let payment_spend = taker_eth_coin
        .send_taker_spends_maker_payment(spend_args)
        .wait()
        .unwrap();
    println!("Payment spend tx hash {:02x}", payment_spend.tx_hash());

    let confirm_input = ConfirmPaymentInput {
        payment_tx: payment_spend.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    taker_eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: &taker_pubkey,
        secret_hash,
        tx: &eth_maker_payment.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let search_tx = block_on(maker_eth_coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();

    let expected = FoundSwapTxSpend::Spent(payment_spend);
    assert_eq!(expected, search_tx);
}

#[test]
fn send_and_refund_erc20_maker_payment() {
    let erc20_coin = erc20_coin_with_random_privkey(swap_contract());

    let time_lock = now_sec() - 100;
    let other_pubkey = &[
        0x02, 0xc6, 0x6e, 0x7d, 0x89, 0x66, 0xb5, 0xc5, 0x55, 0xaf, 0x58, 0x05, 0x98, 0x9d, 0xa9, 0xfb, 0xf8, 0xdb,
        0x95, 0xe1, 0x56, 0x31, 0xce, 0x35, 0x8c, 0x3a, 0x17, 0x10, 0xc9, 0x62, 0x67, 0x90, 0x63,
    ];
    let secret_hash = &[1; 20];

    let send_payment_args = SendPaymentArgs {
        time_lock_duration: 100,
        time_lock,
        other_pubkey,
        secret_hash,
        amount: 1.into(),
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: now_sec() + 60,
    };
    let eth_maker_payment = erc20_coin.send_maker_payment(send_payment_args).wait().unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: eth_maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    erc20_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let refund_args = RefundPaymentArgs {
        payment_tx: &eth_maker_payment.tx_hex(),
        time_lock,
        other_pubkey,
        tx_type_with_secret_hash: SwapTxTypeWithSecretHash::TakerOrMakerPayment {
            maker_secret_hash: secret_hash,
        },
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let payment_refund = block_on(erc20_coin.send_maker_refunds_payment(refund_args)).unwrap();
    println!("Payment refund tx hash {:02x}", payment_refund.tx_hash());

    let confirm_input = ConfirmPaymentInput {
        payment_tx: payment_refund.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    erc20_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: other_pubkey,
        secret_hash,
        tx: &eth_maker_payment.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let search_tx = block_on(erc20_coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();

    let expected = FoundSwapTxSpend::Refunded(payment_refund);
    assert_eq!(expected, search_tx);
}

#[test]
fn send_and_spend_erc20_maker_payment() {
    let maker_erc20_coin = erc20_coin_with_random_privkey(swap_contract());
    let taker_erc20_coin = erc20_coin_with_random_privkey(swap_contract());

    let time_lock = now_sec() + 1000;
    let maker_pubkey = maker_erc20_coin.derive_htlc_pubkey(&[]);
    let taker_pubkey = taker_erc20_coin.derive_htlc_pubkey(&[]);
    let secret = &[2; 32];
    let secret_hash_owned = dhash160(secret);
    let secret_hash = secret_hash_owned.as_slice();

    let send_payment_args = SendPaymentArgs {
        time_lock_duration: 1000,
        time_lock,
        other_pubkey: &taker_pubkey,
        secret_hash,
        amount: 1.into(),
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: now_sec() + 60,
    };
    let eth_maker_payment = maker_erc20_coin.send_maker_payment(send_payment_args).wait().unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: eth_maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    taker_erc20_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let spend_args = SpendPaymentArgs {
        other_payment_tx: &eth_maker_payment.tx_hex(),
        time_lock,
        other_pubkey: &maker_pubkey,
        secret,
        secret_hash,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let payment_spend = taker_erc20_coin
        .send_taker_spends_maker_payment(spend_args)
        .wait()
        .unwrap();
    println!("Payment spend tx hash {:02x}", payment_spend.tx_hash());

    let confirm_input = ConfirmPaymentInput {
        payment_tx: payment_spend.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    taker_erc20_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: &taker_pubkey,
        secret_hash,
        tx: &eth_maker_payment.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let search_tx = block_on(maker_erc20_coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();

    let expected = FoundSwapTxSpend::Spent(payment_spend);
    assert_eq!(expected, search_tx);
}

use super::utils::*;

use crate::lp_swap::SecretHashAlgo;
use crate::lp_wallet::initialize_wallet_passphrase;
use coins::siacoin::{ApiClientHelpers, SiaCoin, SiaCoinActivationRequest};
use coins::Transaction;
use coins::{PrivKeyBuildPolicy, RefundPaymentArgs, SendPaymentArgs, SpendPaymentArgs, SwapOps,
            SwapTxTypeWithSecretHash, TransactionEnum};
use common::now_sec;
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_number::BigDecimal;
use mm2_test_helpers::electrums::doc_electrums;
use mm2_test_helpers::for_tests::enable_utxo_v2_electrum;

use testcontainers::clients::Cli;

/// Initialize Alice
#[tokio::test]
async fn test_init_alice() {
    let temp_dir = init_test_dir(current_function_name!());

    let (_ctx_bob, _mm_bob) = init_alice(&temp_dir, 7778, 9998).await;
}

/// Initialize Bob
#[tokio::test]
async fn test_init_bob() {
    let temp_dir = init_test_dir(current_function_name!());

    let (_ctx_bob, _mm_bob) = init_bob(&temp_dir, 7777, 9998).await;
}

/// Initialize Alice and Bob, check that they connected via p2p network
#[tokio::test]
async fn test_init_alice_and_bob() {
    let temp_dir = init_test_dir(current_function_name!());

    // initialize Bob first because he acts as a seed node
    let (_ctx_bob, mm_bob) = init_bob(&temp_dir, 7777, 9998).await;
    let (_ctx_alice, mm_alice) = init_alice(&temp_dir, 7778, 9998).await;

    // fetch alice peers
    let alice_peers = mm_alice
        .rpc_typed::<GetDirectlyConnectedPeersResponse>(&json!({"method": "get_directly_connected_peers"}))
        .await
        .unwrap();

    // fetch bob PeerId
    let bob_peer_id = mm_bob
        .rpc_typed::<String>(&json!({"method": "get_my_peer_id"}))
        .await
        .unwrap();

    // check that alice has bob in her peers
    assert!(alice_peers.0.contains_key(&bob_peer_id));
}

/// Initialize Alice and Bob, initialize Sia testnet container, enable DSIA for both parties
#[tokio::test]
async fn test_alice_and_bob_enable_dsia() {
    let temp_dir = init_test_dir(current_function_name!());
    let docker = Cli::default();

    let (_ctx_alice, mm_alice) = init_alice(&temp_dir, 7778, 9998).await;
    let (_ctx_bob, mm_bob) = init_bob(&temp_dir, 7777, 9998).await;

    let (_container, walletd_port) = init_walletd_container(&docker);

    let sia_client_url = format!("http://localhost:{}/", walletd_port);
    let _bob_enable_sia_resp = enable_dsia(&mm_alice, &sia_client_url).await;
    let _alice_enable_sia_resp = enable_dsia(&mm_bob, &sia_client_url).await;
}

/// Initialize Alice and Bob, enable DOC via electrum for both parties
#[tokio::test]
async fn test_alice_and_bob_enable_doc() {
    let temp_dir = init_test_dir(current_function_name!());

    let (_ctx_alice, mm_alice) = init_alice(&temp_dir, 7778, 9998).await;
    let (_ctx_bob, mm_bob) = init_bob(&temp_dir, 7777, 9998).await;

    let _bob_enable_utxo_resp = enable_utxo_v2_electrum(&mm_bob, "DOC", doc_electrums(), None, 60, None).await;
    let _alice_enable_utxo_resp = enable_utxo_v2_electrum(&mm_alice, "DOC", doc_electrums(), None, 60, None).await;
}

fn helper_activation_request(port: u16) -> SiaCoinActivationRequest {
    let activation_request_json = json!(
        {
            "tx_history": true,
            "client_conf": {
                "server_url": format!("http://localhost:{}/", port),
                "password": "password"
            }
        }
    );
    serde_json::from_value::<SiaCoinActivationRequest>(activation_request_json).unwrap()
}

/// Initialize a minimal MarketMaker intended for unit testing.
/// See `init_bob` or `init_alice` for creating "full" MarketMaker instances.
async fn init_ctx(passphrase: &str, netid: u16) -> MmArc {
    let kdf_conf = json!({
        "gui": "sia-docker-tests",
        "netid": netid,
        "rpc_password": "rpc_password",
        "passphrase": passphrase,
    });

    let ctx = MmCtxBuilder::new().with_conf(kdf_conf).into_mm_arc();

    initialize_wallet_passphrase(&ctx).await.unwrap();
    ctx
}

async fn init_siacoin(ctx: MmArc, ticker: &str, request: &SiaCoinActivationRequest) -> SiaCoin {
    let coin_conf_str = json!(
        {
            "coin": ticker,
            "required_confirmations": 1,
        }
    );

    let priv_key_policy = PrivKeyBuildPolicy::detect_priv_key_policy(&ctx).unwrap();
    SiaCoin::new(&ctx, coin_conf_str, request, priv_key_policy)
        .await
        .unwrap()
}

/**
 * Initialize ctx and SiaCoin for both parties, maker and taker
 * Initialize a new SiaCoin testnet and mine blocks to maker for funding
 * Send a HTLC payment from maker
 * Spend the HTLC payment from taker
 *
 * maker_* indicates data created by the maker
 * taker_* indicates data created by the taker
 * negotiated_* indicates data that is negotiated via p2p communication
 */
#[tokio::test]
async fn test_send_maker_payment_then_spend_maker_payment() {
    let docker = Cli::default();

    // Start the container
    let (_container, host_port) = init_walletd_container(&docker);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let maker_ctx = init_ctx("maker passphrase", 9995).await;
    let maker_sia_coin = init_siacoin(maker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let maker_public_key = maker_sia_coin.my_keypair().unwrap().public();
    let maker_address = maker_public_key.address();
    let maker_secret = vec![0u8; 32];
    let maker_secret_hash = SecretHashAlgo::SHA256.hash_secret(&maker_secret);
    mine_sia_blocks(&maker_sia_coin.client, 201, &maker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let taker_ctx = init_ctx("taker passphrase", 9995).await;
    let taker_sia_coin = init_siacoin(taker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let taker_public_key = taker_sia_coin.my_keypair().unwrap().public();

    let negotiated_time_lock = now_sec();
    let negotiated_time_lock_duration = 10u64;
    let negotiated_amount: BigDecimal = 1u64.into();

    let maker_send_payment_args = SendPaymentArgs {
        time_lock_duration: negotiated_time_lock_duration,
        time_lock: negotiated_time_lock,
        other_pubkey: taker_public_key.as_bytes(),
        secret_hash: &maker_secret_hash,
        amount: negotiated_amount,
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let maker_payment_tx = match maker_sia_coin
        .send_maker_payment(maker_send_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&maker_sia_coin.client, 1, &maker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let taker_spend_payment_args = SpendPaymentArgs {
        other_payment_tx: &maker_payment_tx.tx_hex(),
        time_lock: negotiated_time_lock,
        other_pubkey: maker_public_key.as_bytes(),
        secret: &maker_secret,
        secret_hash: &maker_secret_hash,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };

    let taker_spends_maker_payment_tx = match taker_sia_coin
        .send_taker_spends_maker_payment(taker_spend_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&maker_sia_coin.client, 1, &maker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let event = maker_sia_coin
        .client
        .get_event(&taker_spends_maker_payment_tx.txid())
        .await
        .unwrap();
    assert_eq!(event.confirmations, 1u64);
}

/**
 * Initialize ctx and SiaCoin for both parties, maker and taker
 * Initialize a new SiaCoin testnet and mine blocks to taker for funding
 * Send a HTLC payment from taker
 * Spend the HTLC payment from maker
 */
#[tokio::test]
async fn test_send_taker_payment_then_spend_taker_payment() {
    let docker = Cli::default();

    // Start the container
    let (_container, host_port) = init_walletd_container(&docker);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let taker_ctx = init_ctx("taker passphrase", 9995).await;
    let taker_sia_coin = init_siacoin(taker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let taker_public_key = taker_sia_coin.my_keypair().unwrap().public();
    let taker_address = taker_public_key.address();
    mine_sia_blocks(&taker_sia_coin.client, 201, &taker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let maker_ctx = init_ctx("maker passphrase", 9995).await;
    let maker_sia_coin = init_siacoin(maker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let maker_public_key = maker_sia_coin.my_keypair().unwrap().public();
    let maker_secret = vec![0u8; 32];
    let maker_secret_hash = SecretHashAlgo::SHA256.hash_secret(&maker_secret);

    let negotiated_time_lock = now_sec();
    let negotiated_time_lock_duration = 10u64;
    let negotiated_amount: BigDecimal = 1u64.into();

    let taker_send_payment_args = SendPaymentArgs {
        time_lock_duration: negotiated_time_lock_duration,
        time_lock: negotiated_time_lock,
        other_pubkey: maker_public_key.as_bytes(),
        secret_hash: &maker_secret_hash,
        amount: negotiated_amount,
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let taker_payment_tx = match taker_sia_coin
        .send_taker_payment(taker_send_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&taker_sia_coin.client, 1, &taker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let maker_spend_payment_args = SpendPaymentArgs {
        other_payment_tx: &taker_payment_tx.tx_hex(),
        time_lock: negotiated_time_lock,
        other_pubkey: taker_public_key.as_bytes(),
        secret: &maker_secret,
        secret_hash: &maker_secret_hash,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };

    let maker_spends_taker_payment_tx = match maker_sia_coin
        .send_maker_spends_taker_payment(maker_spend_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&taker_sia_coin.client, 1, &taker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    taker_sia_coin
        .client
        .get_transaction(&maker_spends_taker_payment_tx.txid())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_send_maker_payment_then_refund_maker_payment() {
    let docker = Cli::default();

    // Start the container
    let (_container, host_port) = init_walletd_container(&docker);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let maker_ctx = init_ctx("maker passphrase", 9995).await;
    let maker_sia_coin = init_siacoin(maker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let maker_public_key = maker_sia_coin.my_keypair().unwrap().public();
    let maker_address = maker_public_key.address();
    let maker_secret = vec![0u8; 32];
    let maker_secret_hash = SecretHashAlgo::SHA256.hash_secret(&maker_secret);
    mine_sia_blocks(&maker_sia_coin.client, 201, &maker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let taker_ctx = init_ctx("taker passphrase", 9995).await;
    let taker_sia_coin = init_siacoin(taker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let taker_public_key = taker_sia_coin.my_keypair().unwrap().public();

    // time lock is set in the past to allow immediate refund
    let negotiated_time_lock = now_sec() - 1000;
    let negotiated_time_lock_duration = 10u64;
    let negotiated_amount: BigDecimal = 1u64.into();

    let maker_send_payment_args = SendPaymentArgs {
        time_lock_duration: negotiated_time_lock_duration,
        time_lock: negotiated_time_lock,
        other_pubkey: taker_public_key.as_bytes(),
        secret_hash: &maker_secret_hash,
        amount: negotiated_amount,
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let maker_payment_tx = match maker_sia_coin
        .send_maker_payment(maker_send_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&maker_sia_coin.client, 1, &maker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let secret_hash_type = SwapTxTypeWithSecretHash::TakerOrMakerPayment {
        maker_secret_hash: &maker_secret_hash,
    };
    let maker_refunds_payment_args = RefundPaymentArgs {
        payment_tx: &maker_payment_tx.tx_hex(),
        time_lock: negotiated_time_lock,
        other_pubkey: taker_public_key.as_bytes(),
        tx_type_with_secret_hash: secret_hash_type,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };

    let maker_refunds_maker_payment_tx = match maker_sia_coin
        .send_maker_refunds_payment(maker_refunds_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&maker_sia_coin.client, 1, &maker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    maker_sia_coin
        .client
        .get_transaction(&maker_refunds_maker_payment_tx.txid())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_send_taker_payment_then_refund_taker_payment() {
    let docker = Cli::default();

    // Start the container
    let (_container, host_port) = init_walletd_container(&docker);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let maker_ctx = init_ctx("maker passphrase", 9995).await;
    let maker_sia_coin = init_siacoin(maker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let maker_public_key = maker_sia_coin.my_keypair().unwrap().public();
    let maker_secret = vec![0u8; 32];
    let maker_secret_hash = SecretHashAlgo::SHA256.hash_secret(&maker_secret);

    let taker_ctx = init_ctx("taker passphrase", 9995).await;
    let taker_sia_coin = init_siacoin(taker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let taker_public_key = taker_sia_coin.my_keypair().unwrap().public();
    let taker_address = taker_public_key.address();
    mine_sia_blocks(&taker_sia_coin.client, 201, &taker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // time lock is set in the past to allow immediate refund
    let negotiated_time_lock = now_sec() - 1000;
    let negotiated_time_lock_duration = 10u64;
    let negotiated_amount: BigDecimal = 1u64.into();

    let taker_send_payment_args = SendPaymentArgs {
        time_lock_duration: negotiated_time_lock_duration,
        time_lock: negotiated_time_lock,
        other_pubkey: maker_public_key.as_bytes(),
        secret_hash: &maker_secret_hash,
        amount: negotiated_amount,
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let taker_payment_tx = match taker_sia_coin
        .send_maker_payment(taker_send_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&taker_sia_coin.client, 1, &taker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let secret_hash_type = SwapTxTypeWithSecretHash::TakerOrMakerPayment {
        maker_secret_hash: &maker_secret_hash,
    };
    let taker_refunds_payment_args = RefundPaymentArgs {
        payment_tx: &taker_payment_tx.tx_hex(),
        time_lock: negotiated_time_lock,
        other_pubkey: maker_public_key.as_bytes(),
        tx_type_with_secret_hash: secret_hash_type,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };

    let taker_refunds_taker_payment_tx = match taker_sia_coin
        .send_taker_refunds_payment(taker_refunds_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&taker_sia_coin.client, 1, &taker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    taker_sia_coin
        .client
        .get_transaction(&taker_refunds_taker_payment_tx.txid())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_spend_taker_payment_then_taker_extract_secret() {
    let docker = Cli::default();

    // Start the container
    let (_container, host_port) = init_walletd_container(&docker);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let taker_ctx = init_ctx("taker passphrase", 9995).await;
    let taker_sia_coin = init_siacoin(taker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let taker_public_key = taker_sia_coin.my_keypair().unwrap().public();
    let taker_address = taker_public_key.address();
    mine_sia_blocks(&taker_sia_coin.client, 201, &taker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let maker_ctx = init_ctx("maker passphrase", 9995).await;
    let maker_sia_coin = init_siacoin(maker_ctx, "TSIA", &helper_activation_request(host_port)).await;
    let maker_public_key = maker_sia_coin.my_keypair().unwrap().public();
    let maker_secret = vec![0u8; 32];
    let maker_secret_hash = SecretHashAlgo::SHA256.hash_secret(&maker_secret);

    let negotiated_time_lock = now_sec();
    let negotiated_time_lock_duration = 10u64;
    let negotiated_amount: BigDecimal = 1u64.into();

    let taker_send_payment_args = SendPaymentArgs {
        time_lock_duration: negotiated_time_lock_duration,
        time_lock: negotiated_time_lock,
        other_pubkey: maker_public_key.as_bytes(),
        secret_hash: &maker_secret_hash,
        amount: negotiated_amount,
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let taker_payment_tx = match taker_sia_coin
        .send_taker_payment(taker_send_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&taker_sia_coin.client, 1, &taker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let maker_spend_payment_args = SpendPaymentArgs {
        other_payment_tx: &taker_payment_tx.tx_hex(),
        time_lock: negotiated_time_lock,
        other_pubkey: taker_public_key.as_bytes(),
        secret: &maker_secret,
        secret_hash: &maker_secret_hash,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };

    let maker_spends_taker_payment_tx = match maker_sia_coin
        .send_maker_spends_taker_payment(maker_spend_payment_args)
        .await
        .unwrap()
    {
        TransactionEnum::SiaTransaction(tx) => tx,
        _ => panic!("Expected SiaTransaction"),
    };
    mine_sia_blocks(&taker_sia_coin.client, 1, &taker_address)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    taker_sia_coin
        .client
        .get_transaction(&maker_spends_taker_payment_tx.txid())
        .await
        .unwrap();

    let maker_spends_taker_payment_tx_hex = maker_spends_taker_payment_tx.tx_hex();

    let taker_extracted_secret = taker_sia_coin
        .extract_secret(&maker_secret_hash, maker_spends_taker_payment_tx_hex.as_slice(), false)
        .await
        .unwrap();

    assert_eq!(taker_extracted_secret, maker_secret);
}

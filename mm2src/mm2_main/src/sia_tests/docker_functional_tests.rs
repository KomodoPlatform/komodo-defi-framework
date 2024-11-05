use coins::siacoin::sia_rust::transport::client::native::NativeClient;
use coins::siacoin::sia_rust::transport::client::{ApiClient as SiaApiClient, ApiClientError};
use coins::siacoin::sia_rust::transport::endpoints::DebugMineRequest;
use coins::siacoin::sia_rust::types::Address;
use coins::siacoin::{ApiClientHelpers, SiaCoin, SiaCoinActivationRequest};
use coins::Transaction;
use coins::{PrivKeyBuildPolicy, RefundPaymentArgs, SendPaymentArgs, SpendPaymentArgs, SwapOps,
            SwapTxTypeWithSecretHash, TransactionEnum};
use common::now_sec;
use mm2_number::BigDecimal;
use testcontainers::clients::Cli;

use crate::lp_swap::SecretHashAlgo;
use crate::lp_wallet::initialize_wallet_passphrase;
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};

use testcontainers::{Container, GenericImage, RunnableImage};

async fn mine_blocks(client: &NativeClient, n: i64, addr: &Address) -> Result<(), ApiClientError> {
    client
        .dispatcher(DebugMineRequest {
            address: addr.clone(),
            blocks: n,
        })
        .await?;
    Ok(())
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

/// initialize a walletd docker container with walletd API bound to a random host port
/// returns the container and the host port it is bound to
fn init_walletd_container(docker: &Cli) -> (Container<GenericImage>, u16) {
    // Define the Docker image with a tag
    let image = GenericImage::new("docker.io/alrighttt/walletd-komodo", "latest").with_exposed_port(9980);

    // Wrap the image in `RunnableImage` to allow custom port mapping to an available host port
    // 0 indicates that the host port will be automatically assigned to an available port
    let runnable_image = RunnableImage::from(image).with_mapped_port((0, 9980));

    // Start the container. It will run until `Container` falls out of scope
    let container = docker.run(runnable_image);

    // Retrieve the host port that is mapped to the container's 9980 port
    let host_port = container.get_host_port_ipv4(9980);

    (container, host_port)
}

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
    SiaCoin::from_conf_and_request(&ctx, coin_conf_str, request, priv_key_policy)
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
    mine_blocks(&maker_sia_coin.client, 201, &maker_address).await.unwrap();
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
    mine_blocks(&maker_sia_coin.client, 1, &maker_address).await.unwrap();
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
    mine_blocks(&maker_sia_coin.client, 1, &maker_address).await.unwrap();
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
    mine_blocks(&taker_sia_coin.client, 201, &taker_address).await.unwrap();
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
    mine_blocks(&taker_sia_coin.client, 1, &taker_address).await.unwrap();
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
    mine_blocks(&taker_sia_coin.client, 1, &taker_address).await.unwrap();
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
    mine_blocks(&maker_sia_coin.client, 201, &maker_address).await.unwrap();
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
    mine_blocks(&maker_sia_coin.client, 1, &maker_address).await.unwrap();
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
    mine_blocks(&maker_sia_coin.client, 1, &maker_address).await.unwrap();
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
    mine_blocks(&taker_sia_coin.client, 201, &taker_address).await.unwrap();
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
    mine_blocks(&taker_sia_coin.client, 1, &taker_address).await.unwrap();
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
    mine_blocks(&taker_sia_coin.client, 1, &taker_address).await.unwrap();
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
    mine_blocks(&taker_sia_coin.client, 201, &taker_address).await.unwrap();
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
    mine_blocks(&taker_sia_coin.client, 1, &taker_address).await.unwrap();
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
    mine_blocks(&taker_sia_coin.client, 1, &taker_address).await.unwrap();
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

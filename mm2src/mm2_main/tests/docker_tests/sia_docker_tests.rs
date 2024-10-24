use coins::siacoin::{sia_coin_from_conf_and_request, SiaCoin, SiaCoinActivationRequest, SiaCoinConf};
use coins::PrivKeyBuildPolicy;
use common::block_on;
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_main::lp_wallet::initialize_wallet_passphrase;
use sia_rust::transport::client::native::{Conf, NativeClient};
use sia_rust::transport::client::{ApiClient, ApiClientError, ApiClientHelpers};
use sia_rust::transport::endpoints::{AddressBalanceRequest, ConsensusTipRequest, DebugMineRequest,
                                     GetAddressUtxosRequest, TxpoolBroadcastRequest};
use sia_rust::types::{Address, Currency, Keypair, SiacoinOutput, SpendPolicy, V2TransactionBuilder};
use std::process::Command;
use std::str::FromStr;
use url::Url;

/*
These tests are intended to ran manually for now.
Otherwise, they can interfere with each other since there is only one docker container initialized for all of them.
TODO: refactor; see block comment in ../docker_tests_sia_unique.rs for more information.
*/

fn mine_blocks(client: &NativeClient, n: i64, addr: &Address) -> Result<(), ApiClientError> {
    block_on(client.dispatcher(DebugMineRequest {
        address: addr.clone(),
        blocks: n,
    }))?;
    Ok(())
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

    sia_coin_from_conf_and_request(&ctx, ticker, coin_conf_str, request, priv_key_policy)
        .await
        .unwrap()
}

fn default_activation_request() -> SiaCoinActivationRequest {
    let activation_request_json = json!(
        {
            "tx_history": true,
            "client_conf": {
                "server_url": "http://localhost:9980/",
                "password": "password"
            }
        }
    );
    serde_json::from_value::<SiaCoinActivationRequest>(activation_request_json).unwrap()
}
// FIXME WIP
#[test]
fn test_sia_swap_ops_send_taker_fee_wip() {
    use coins::DexFee;
    use mm2_number::MmNumber;
    use uuid::Uuid;

    let ctx = block_on(init_ctx("horribly insecure passphrase", 9995));
    let coin = block_on(init_siacoin(ctx, "TSIA", &default_activation_request()));
    mine_blocks(&coin.client, 201, &coin.address).unwrap();

    let uuid = Uuid::new_v4();
    let dex_fee = DexFee::Standard(MmNumber::from(0.0001));

    assert_eq!(block_on(coin.client.current_height()).unwrap(), 0);
}

#[test]
fn test_sia_init_siacoin() {
    let ctx = block_on(init_ctx("horribly insecure passphrase", 9995));
    let coin = block_on(init_siacoin(ctx, "TSIA", &default_activation_request()));
    assert_eq!(block_on(coin.client.current_height()).unwrap(), 0);
}

#[test]
fn test_sia_new_client() {
    let conf = Conf {
        server_url: Url::parse("http://localhost:9980/").unwrap(),
        password: Some("password".to_string()),
        timeout: Some(10),
    };
    let _api_client = block_on(NativeClient::new(conf)).unwrap();
}

#[test]
fn test_sia_endpoint_consensus_tip() {
    let conf = Conf {
        server_url: Url::parse("http://localhost:9980/").unwrap(),
        password: Some("password".to_string()),
        timeout: Some(10),
    };
    let api_client = block_on(NativeClient::new(conf)).unwrap();
    let _response = block_on(api_client.dispatcher(ConsensusTipRequest)).unwrap();
}

#[test]
fn test_sia_endpoint_debug_mine() {
    let conf = Conf {
        server_url: Url::parse("http://localhost:9980/").unwrap(),
        password: Some("password".to_string()),
        timeout: Some(10),
    };
    let api_client = block_on(NativeClient::new(conf)).unwrap();

    let address =
        Address::from_str("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f").unwrap();
    block_on(api_client.dispatcher(DebugMineRequest {
        address: address.clone(),
        blocks: 100,
    }))
    .unwrap();

    let height = block_on(api_client.current_height()).unwrap();
    assert_eq!(height, 100);

    // test the helper function as well
    mine_blocks(&api_client, 100, &address).unwrap();
    let response = block_on(api_client.dispatcher(ConsensusTipRequest)).unwrap();
    assert_eq!(response.height, 200);
}

#[test]
fn test_sia_endpoint_address_balance() {
    let conf = Conf {
        server_url: Url::parse("http://localhost:9980/").unwrap(),
        password: Some("password".to_string()),
        timeout: Some(10),
    };
    let api_client = block_on(NativeClient::new(conf)).unwrap();

    let address =
        Address::from_str("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f").unwrap();
    mine_blocks(&api_client, 10, &address).unwrap();

    let request = AddressBalanceRequest { address };
    let response = block_on(api_client.dispatcher(request)).unwrap();

    let expected = Currency::from(1);
    assert_eq!(response.siacoins, expected);
    assert_eq!(*expected, 1000000000000000000000000000000000000);
}

#[test]
fn test_sia_build_tx() {
    let conf = Conf {
        server_url: Url::parse("http://localhost:9980/").unwrap(),
        password: Some("password".to_string()),
        timeout: Some(10),
    };
    let api_client = block_on(NativeClient::new(conf)).unwrap();
    let keypair = Keypair::from_private_bytes(
        &hex::decode("0100000000000000000000000000000000000000000000000000000000000000").unwrap(),
    )
    .unwrap();

    let address = Address::from_public_key(&keypair.public());

    mine_blocks(&api_client, 201, &address).unwrap();

    let utxos = block_on(api_client.dispatcher(GetAddressUtxosRequest {
        address: address.clone(),
        limit: None,
        offset: None,
    }))
    .unwrap();
    let spend_this = utxos[0].clone();
    let vin = spend_this.clone();
    println!("utxo[0]: {:?}", spend_this);
    let vout = SiacoinOutput {
        value: spend_this.siacoin_output.value,
        address,
    };
    let tx = V2TransactionBuilder::new()
        .add_siacoin_input(vin, spend_policy)
        .add_siacoin_output(vout)
        .sign_simple(vec![&keypair])
        .unwrap()
        .build();

    let req = TxpoolBroadcastRequest {
        transactions: vec![],
        v2transactions: vec![tx],
    };
    let _response = block_on(api_client.dispatcher(req)).unwrap();
}

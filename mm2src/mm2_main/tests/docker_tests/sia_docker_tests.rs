use common::block_on;
use sia_rust::transport::client::native::{Conf, NativeClient};
use sia_rust::transport::client::ApiClient;
use sia_rust::transport::endpoints::{AddressBalanceRequest, ConsensusTipRequest, GetAddressUtxosRequest,
                                     TxpoolBroadcastRequest};
use sia_rust::types::{Address, Currency, Keypair, SiacoinOutput, SpendPolicy, V2TransactionBuilder};
use std::process::Command;
use std::str::FromStr;
use url::Url;

#[cfg(test)]
fn mine_blocks(n: u64, addr: &Address) {
    Command::new("docker")
        .arg("exec")
        .arg("sia-docker")
        .arg("walletd")
        .arg("mine")
        .arg(format!("-addr={}", addr))
        .arg(format!("-n={}", n))
        .status()
        .expect("Failed to execute docker command");
}

#[test]
fn test_sia_new_client() {
    let conf = Conf {
        server_url: Url::parse("http://localhost:9980/").unwrap(),
        password: None,
        timeout: Some(10),
    };
    let _api_client = block_on(NativeClient::new(conf)).unwrap();
}

#[test]
fn test_sia_client_consensus_tip() {
    let conf = Conf {
        server_url: Url::parse("http://localhost:9980/").unwrap(),
        password: None,
        timeout: Some(10),
    };
    let api_client = block_on(NativeClient::new(conf)).unwrap();
    let _response = block_on(api_client.dispatcher(ConsensusTipRequest)).unwrap();
}

// This test likely needs to be removed because mine_blocks has possibility of interfering with other async tests
// related to block height
#[test]
fn test_sia_client_address_balance() {
    let conf = Conf {
        server_url: Url::parse("http://localhost:9980/").unwrap(),
        password: None,
        timeout: Some(10),
    };
    let api_client = block_on(NativeClient::new(conf)).unwrap();

    let address =
        Address::from_str("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f").unwrap();
    mine_blocks(10, &address);

    let request = AddressBalanceRequest { address };
    let response = block_on(api_client.dispatcher(request)).unwrap();

    let expected = Currency::from(1);
    assert_eq!(response.siacoins, expected);
    assert_eq!(*expected, 1000000000000000000000000000000000000);
}

#[test]
fn test_sia_client_build_tx() {
    let conf = Conf {
        server_url: Url::parse("http://localhost:9980/").unwrap(),
        password: None,
        timeout: Some(10),
    };
    let api_client = block_on(NativeClient::new(conf)).unwrap();
    let keypair = Keypair::from_private_bytes(
        &hex::decode("0100000000000000000000000000000000000000000000000000000000000000").unwrap(),
    )
    .unwrap();
    let spend_policy = SpendPolicy::PublicKey(keypair.public());

    let address = spend_policy.address();

    mine_blocks(201, &address);

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

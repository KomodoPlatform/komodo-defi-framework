use mm2_number::MmNumber;
use sia::http_client::{SiaApiClient, SiaApiClientError, SiaHttpConf};
use sia::http_endpoints::{AddressBalanceRequest, ConsensusTipRequest};
use sia::types::Address;
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

#[tokio::test]
async fn test_sia_new_client() {
    let conf = SiaHttpConf {
        url: Url::parse("http://localhost:9980/").unwrap(),
        password: "password".to_string(),
    };
    let _api_client = SiaApiClient::new(conf).await.unwrap();
}

#[tokio::test]
async fn test_sia_client_bad_auth() {
    let conf = SiaHttpConf {
        url: Url::parse("http://localhost:9980/").unwrap(),
        password: "foo".to_string(),
    };
    let result = SiaApiClient::new(conf).await;
    assert!(matches!(result, Err(SiaApiClientError::UnexpectedHttpStatus(401))));
}

#[tokio::test]
async fn test_sia_client_consensus_tip() {
    let conf = SiaHttpConf {
        url: Url::parse("http://localhost:9980/").unwrap(),
        password: "password".to_string(),
    };
    let api_client = SiaApiClient::new(conf).await.unwrap();
    let _response = api_client.dispatcher(ConsensusTipRequest).await.unwrap();
}

// This test likely needs to be removed because mine_blocks has possibility of interferring with other async tests
// related to block height
#[tokio::test]
async fn test_sia_client_address_balance() {
    let conf = SiaHttpConf {
        url: Url::parse("http://localhost:9980/").unwrap(),
        password: "password".to_string(),
    };
    let api_client = SiaApiClient::new(conf).await.unwrap();

    let address =
        Address::from_str("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f").unwrap();
    mine_blocks(10, &address);

    let request = AddressBalanceRequest { address };
    let response = api_client.dispatcher(request).await.unwrap();

    assert_eq!(
        response.siacoins,
        MmNumber::from("1000000000000000000000000000000000000")
    )
}

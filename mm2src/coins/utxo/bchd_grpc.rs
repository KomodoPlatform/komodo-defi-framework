mod pb {
    tonic::include_proto!("/pb");
}

use pb::{bchrpc_client::BchrpcClient, get_block_request::HashOrHeight, GetBlockRequest};
use tonic::transport::{Channel, ClientTlsConfig};

#[test]
fn test_get_block() {
    use common::block_on;
    use common::wio::CORE;

    let _guard = CORE.0.enter();

    let mut conf = rustls::ClientConfig::new();
    conf.root_store
        .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
    conf.set_protocols(&[Vec::from("h2")]);
    let tls = ClientTlsConfig::new()
        .rustls_client_config(conf)
        .domain_name("bchd-testnet.greyh.at");

    let channel = block_on(
        Channel::from_static("https://bchd-testnet.greyh.at:18335")
            .tls_config(tls)
            .unwrap()
            .connect(),
    )
    .unwrap();

    let mut client = BchrpcClient::new(channel);
    let request = tonic::Request::new(GetBlockRequest {
        hash_or_height: Some(HashOrHeight::Height(1)),
        full_transactions: false,
    });

    let response = block_on(client.get_block(request)).unwrap();

    println!("RESPONSE={:?}", response);
}

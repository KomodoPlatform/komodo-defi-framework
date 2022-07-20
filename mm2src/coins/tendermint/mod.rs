/// Module implementing Tendermint (Cosmos) integration
/// Useful resources
/// https://docs.cosmos.network/
mod tendermint_coin;
pub use tendermint_coin::*;

#[test]
fn try_tendermint_rpc() {
    use cosmrs::proto::cosmos::bank::v1beta1::{QueryAllBalancesRequest, QueryAllBalancesResponse};
    use cosmrs::rpc::endpoint::abci_query::Request as AbciRequest;
    use cosmrs::rpc::Client;
    use cosmrs::rpc::HttpClient;
    use cosmrs::tendermint::abci::Path as AbciPath;
    use cosmrs::AccountId;
    let cosmos_url = "https://cosmos-testnet-rpc.allthatnode.com:26657";
    let client = HttpClient::new(cosmos_url).unwrap();
    use common::block_on;
    use prost::Message;
    use std::str::FromStr;
    println!("{:?}", client);

    let request = cosmrs::rpc::endpoint::abci_info::Request {};
    let response = block_on(client.perform(request));
    println!("{:?}", response);

    let path = AbciPath::from_str("/cosmos.bank.v1beta1.Query/AllBalances").unwrap();
    let request = QueryAllBalancesRequest {
        address: "cosmos1m7uyxn26sz6w4755k6rch4dc2fj6cmzajkszvn".to_string(),
        pagination: None,
    };
    let request = AbciRequest::new(Some(path), request.encode_to_vec(), None, false);

    let response = block_on(client.perform(request)).unwrap();
    println!("{:?}", response);
    let response = QueryAllBalancesResponse::decode(response.response.value.as_slice()).unwrap();
    println!("{:?}", response);

    /*
    let path = AbciPath::from_str("/cosmos.bank.v1beta1.Query/Balance").unwrap();
    let request = QueryBalanceRequest {
        address: "cosmos1m7uyxn26sz6w4755k6rch4dc2fj6cmzajkszvn".to_string(),
        denom: "uosmo".into(),
    };
    let request = AbciRequest::new(Some(path), request.encode_to_vec(), None, false);

    let response = block_on(client.perform(request)).unwrap();
    println!("{:?}", response);
    let response = QueryBalanceResponse::decode(response.response.value.as_slice()).unwrap();
    println!("{:?}", response);

     */
}

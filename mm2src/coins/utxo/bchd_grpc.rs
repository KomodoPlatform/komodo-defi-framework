/// https://bchd.cash/
/// https://bchd.fountainhead.cash/
use super::bchd_pb::*;
use common::block_on;
use common::grpc_web::post_grpc_web;

#[test]
fn test_get_block_grpc_web() {
    let mut tx_hash = hex::decode("0ba1b91abbfceaa0777424165edb2928dace87d59669c913989950da31968032").unwrap();
    tx_hash.reverse();

    let request = GetSlpTrustedValidationRequest {
        queries: vec![
            get_slp_trusted_validation_request::Query {
                prev_out_hash: tx_hash.clone(),
                prev_out_vout: 1,
                graphsearch_valid_hashes: vec![],
            },
            get_slp_trusted_validation_request::Query {
                prev_out_hash: tx_hash.clone(),
                prev_out_vout: 2,
                graphsearch_valid_hashes: vec![],
            },
        ],
        include_graphsearch_count: false,
    };

    let url = "https://bchd-testnet.greyh.at:18335/pb.bchrpc/GetSlpTrustedValidation";
    let response: GetSlpTrustedValidationResponse = block_on(post_grpc_web(url, request)).unwrap();

    println!("RESPONSE={:?}", response);
}

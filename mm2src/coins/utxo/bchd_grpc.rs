/// https://bchd.cash/
/// https://bchd.fountainhead.cash/
use super::bchd_pb::*;
use crate::utxo::slp::SlpUnspent;
use chain::OutPoint;
use common::grpc_web::{post_grpc_web, PostGrpcWebErr};
use common::mm_error::prelude::*;
use futures::future::join_all;
use get_slp_trusted_validation_response::validity_result::ValidityResultType;
use keys::hash::H256;

#[derive(Debug)]
pub enum ValidateSlpUtxosErr {
    PostGrpcError {
        to_url: String,
        err: PostGrpcWebErr,
    },
    UnexpectedTokenId {
        expected: H256,
        actual: H256,
    },
    UnexpectedValidityResultType {
        for_unspent: SlpUnspent,
        validity_result: Option<ValidityResultType>,
    },
    UnexpectedUtxoInResponse {
        outpoint: OutPoint,
    },
}

pub async fn validate_slp_utxos(
    bchd_urls: &[&str],
    utxos: Vec<SlpUnspent>,
    token_id: &H256,
) -> Result<(), MmError<ValidateSlpUtxosErr>> {
    let queries = utxos
        .iter()
        .map(|utxo| get_slp_trusted_validation_request::Query {
            prev_out_hash: utxo.bch_unspent.outpoint.hash.take().into(),
            prev_out_vout: utxo.bch_unspent.outpoint.index,
            graphsearch_valid_hashes: Vec::new(),
        })
        .collect();
    let request = GetSlpTrustedValidationRequest {
        queries,
        include_graphsearch_count: false,
    };

    let futures = bchd_urls
        .iter()
        .map(|url| post_grpc_web::<_, GetSlpTrustedValidationResponse>(url, &request));
    let results = join_all(futures).await;
    for (i, result) in results.into_iter().enumerate() {
        let response = result.mm_err(|e| ValidateSlpUtxosErr::PostGrpcError {
            to_url: bchd_urls.get(i).map(|url| url.to_string()).unwrap_or_default(),
            err: e,
        })?;

        for validation_result in response.results {
            let actual_token_id = validation_result.token_id.as_slice().into();
            if actual_token_id != *token_id {
                return MmError::err(ValidateSlpUtxosErr::UnexpectedTokenId {
                    expected: *token_id,
                    actual: actual_token_id,
                });
            }

            let outpoint = OutPoint {
                hash: validation_result.prev_out_hash.as_slice().into(),
                index: validation_result.prev_out_vout,
            };

            let initial_unspent = utxos
                .iter()
                .find(|unspent| unspent.bch_unspent.outpoint == outpoint)
                .or_mm_err(|| ValidateSlpUtxosErr::UnexpectedUtxoInResponse { outpoint })?;

            match validation_result.validity_result_type {
                Some(ValidityResultType::V1TokenAmount(slp_amount)) => {
                    if slp_amount != initial_unspent.slp_amount {
                        return MmError::err(ValidateSlpUtxosErr::UnexpectedValidityResultType {
                            for_unspent: initial_unspent.clone(),
                            validity_result: validation_result.validity_result_type,
                        });
                    }
                },
                _ => {
                    return MmError::err(ValidateSlpUtxosErr::UnexpectedValidityResultType {
                        for_unspent: initial_unspent.clone(),
                        validity_result: validation_result.validity_result_type,
                    })
                },
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod bchd_grpc_tests {
    use super::*;
    use crate::utxo::rpc_clients::UnspentInfo;
    use common::block_on;

    #[test]
    fn test_validate_slp_utxos_valid() {
        let tx_hash = H256::from_reversed_str("0ba1b91abbfceaa0777424165edb2928dace87d59669c913989950da31968032");

        let slp_utxos = vec![
            SlpUnspent {
                bch_unspent: UnspentInfo {
                    outpoint: OutPoint {
                        hash: tx_hash,
                        index: 1,
                    },
                    value: 0,
                    height: None,
                },
                slp_amount: 1000,
            },
            SlpUnspent {
                bch_unspent: UnspentInfo {
                    outpoint: OutPoint {
                        hash: tx_hash,
                        index: 2,
                    },
                    value: 0,
                    height: None,
                },
                slp_amount: 8999,
            },
        ];

        let url = "https://bchd-testnet.greyh.at:18335/pb.bchrpc/GetSlpTrustedValidation";
        let token_id = H256::from("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7");
        block_on(validate_slp_utxos(&[url], slp_utxos, &token_id)).unwrap();
    }

    #[test]
    fn test_validate_slp_utxos_non_slp_input() {
        let tx_hash = H256::from_reversed_str("0ba1b91abbfceaa0777424165edb2928dace87d59669c913989950da31968032");

        let slp_utxos = vec![
            SlpUnspent {
                bch_unspent: UnspentInfo {
                    outpoint: OutPoint {
                        hash: tx_hash,
                        index: 1,
                    },
                    value: 0,
                    height: None,
                },
                slp_amount: 1000,
            },
            SlpUnspent {
                bch_unspent: UnspentInfo {
                    outpoint: OutPoint {
                        hash: tx_hash,
                        index: 2,
                    },
                    value: 0,
                    height: None,
                },
                slp_amount: 8999,
            },
            SlpUnspent {
                bch_unspent: UnspentInfo {
                    outpoint: OutPoint {
                        hash: tx_hash,
                        index: 3,
                    },
                    value: 0,
                    height: None,
                },
                slp_amount: 8999,
            },
        ];

        let url = "https://bchd-testnet.greyh.at:18335/pb.bchrpc/GetSlpTrustedValidation";
        let token_id = H256::from("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7");
        let err = block_on(validate_slp_utxos(&[url], slp_utxos, &token_id)).unwrap_err();
        match err.into_inner() {
            ValidateSlpUtxosErr::PostGrpcError { .. } => (),
            err @ _ => panic!("Unexpected error {:?}", err),
        }
    }

    #[test]
    fn test_validate_slp_utxos_invalid_amount() {
        let tx_hash = H256::from_reversed_str("0ba1b91abbfceaa0777424165edb2928dace87d59669c913989950da31968032");
        let invalid_utxo = SlpUnspent {
            bch_unspent: UnspentInfo {
                outpoint: OutPoint {
                    hash: tx_hash,
                    index: 1,
                },
                value: 0,
                height: None,
            },
            slp_amount: 999,
        };

        let slp_utxos = vec![invalid_utxo.clone(), SlpUnspent {
            bch_unspent: UnspentInfo {
                outpoint: OutPoint {
                    hash: tx_hash,
                    index: 2,
                },
                value: 0,
                height: None,
            },
            slp_amount: 8999,
        }];

        let url = "https://bchd-testnet.greyh.at:18335/pb.bchrpc/GetSlpTrustedValidation";
        let token_id = H256::from("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7");
        let err = block_on(validate_slp_utxos(&[url], slp_utxos, &token_id)).unwrap_err();
        match err.into_inner() {
            ValidateSlpUtxosErr::UnexpectedValidityResultType {
                for_unspent,
                validity_result,
            } => {
                let expected_validity = Some(ValidityResultType::V1TokenAmount(1000));
                assert_eq!(invalid_utxo, for_unspent);
                assert_eq!(expected_validity, validity_result);
            },
            err @ _ => panic!("Unexpected error {:?}", err),
        }
    }

    #[test]
    fn test_validate_slp_utxos_unexpected_token_id() {
        let tx_hash = H256::from_reversed_str("0ba1b91abbfceaa0777424165edb2928dace87d59669c913989950da31968032");

        let slp_utxos = vec![
            SlpUnspent {
                bch_unspent: UnspentInfo {
                    outpoint: OutPoint {
                        hash: tx_hash,
                        index: 1,
                    },
                    value: 0,
                    height: None,
                },
                slp_amount: 1000,
            },
            SlpUnspent {
                bch_unspent: UnspentInfo {
                    outpoint: OutPoint {
                        hash: tx_hash,
                        index: 2,
                    },
                    value: 0,
                    height: None,
                },
                slp_amount: 8999,
            },
        ];

        let url = "https://bchd-testnet.greyh.at:18335/pb.bchrpc/GetSlpTrustedValidation";
        let valid_token_id = H256::from("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7");
        let invalid_token_id = H256::from("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb8");
        let err = block_on(validate_slp_utxos(&[url], slp_utxos, &invalid_token_id)).unwrap_err();
        match err.into_inner() {
            ValidateSlpUtxosErr::UnexpectedTokenId { expected, actual } => {
                assert_eq!(invalid_token_id, expected);
                assert_eq!(valid_token_id, actual);
            },
            err @ _ => panic!("Unexpected error {:?}", err),
        }
    }
}

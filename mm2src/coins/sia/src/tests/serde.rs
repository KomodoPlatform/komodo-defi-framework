use crate::encoding::PrefixedH256;
use crate::spend_policy::UnlockKey;
use crate::transaction::{SiacoinElement, SiacoinOutput, StateElement, V2Transaction};
use crate::types::{Address, BlockID, Event};

// Ensure the original value matches the value after round-trip (serialize -> deserialize -> serialize)
macro_rules! test_serde {
    ($type:ty, $json_value:expr) => {{
        let json_str = $json_value.to_string();
        let value: $type = serde_json::from_str(&json_str).unwrap();
        let serialized = serde_json::to_string(&value).unwrap();
        let serialized_json_value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!($json_value, serialized_json_value);
    }};
}

// FIXME reminder to populate the following tests
#[test]
#[ignore]
fn test_serde_block_id() {
    test_serde!(
        BlockID,
        json!("bid:c67c3b2e57490617a25a9fcb9fd54ab6acbe72fc1e4f1f432cb9334177917667")
    );
    test_serde!(BlockID, json!("bid:badc0de"));
    test_serde!(BlockID, json!("bid:1badc0de"));
    test_serde!(BlockID, json!("1badc0de"));
    test_serde!(BlockID, json!(1));
}

#[test]
fn test_serde_address() {
    test_serde!(
        Address,
        json!("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f")
    );
}

#[test]
fn test_serde_unlock_key() {
    test_serde!(
        UnlockKey,
        json!("ed25519:0102030000000000000000000000000000000000000000000000000000000000")
    );
}

#[test]
fn test_serde_sia_hash() {
    test_serde!(
        PrefixedH256,
        json!("h:dc07e5bf84fbda867a7ed7ca80c6d1d81db05cef16ff38f6ba80b6bf01e1ddb1")
    );
}

#[test]
fn test_serde_siacoin_output() {
    let j = json!({
      "value": "300000000000000000000000000000",
      "address": "addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f"
    });
    test_serde!(SiacoinOutput, j);
}

#[test]
fn test_serde_state_element() {
    let j = json!({
      "id": "h:dc07e5bf84fbda867a7ed7ca80c6d1d81db05cef16ff38f6ba80b6bf01e1ddb1",
      "leafIndex": 21,
      "merkleProof": null
    });
    serde_json::from_value::<StateElement>(j).unwrap();
}

#[test]
fn test_serde_siacoin_element() {
    let j = json!(  {
          "id": "h:dc07e5bf84fbda867a7ed7ca80c6d1d81db05cef16ff38f6ba80b6bf01e1ddb1",
          "leafIndex": 21,
          "merkleProof": ["h:8dfc4731c4ef4bf35f789893e72402a39c7ea63ba9e75565cb11000d0159959e"],
          "siacoinOutput": {
            "value": "300000000000000000000000000000",
            "address": "addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f"
          },
          "maturityHeight": 154
        }
    );
    serde_json::from_value::<SiacoinElement>(j).unwrap();
}

#[test]
fn test_serde_siacoin_element_null_merkle_proof() {
    let j = json!(  {
          "id": "h:dc07e5bf84fbda867a7ed7ca80c6d1d81db05cef16ff38f6ba80b6bf01e1ddb1",
          "leafIndex": 21,
          "merkleProof": null,
          "siacoinOutput": {
            "value": "300000000000000000000000000000",
            "address": "addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f"
          },
          "maturityHeight": 154
        }
    );
    serde_json::from_value::<SiacoinElement>(j).unwrap();
}

#[test]
fn test_serde_event_v2_contract_resolution_storage_proof() {
    let j = json!(
      {
        "id": "h:a863dbc4f02efdfbf9f8d03e1aada090ede0a5752b71503787617d5f395c1335",
        "index": {
          "height": 201,
          "id": "bid:e6e5282f107f2957844a93612e71003ec67238f32504b151e9e21fbb9224e8cf"
        },
        "timestamp": "2024-07-18T19:04:16Z",
        "maturityHeight": 345,
        "type": "v2ContractResolution",
        "data": {
          "resolution": {
            "parent": {
              "id": "h:b30e0d25d4e414763378236b00a98cfbf9cd6a5e81540d1dcd40338ab6a5c636",
              "leafIndex": 397,
              "merkleProof": [
                "h:4d2a433de745231ff1eb0736ba68ffc3f8b1a976dbc3eca9649b5cf2dd5c2c44",
                "h:e23fdf53d7c3c2bc7dc58660cb16e5b66dbf2e71c0a46c778af1c4d59a83cf63",
                "h:0e63636af15d58fd9a87e21719899c2d518a948305e325929cbc4652d0fc3b38",
                "h:37e5cee3bb2607e537209807b07dafef9658253080751b11858a9ae844364c0b",
                "h:077252892fc0b8e687f14baf2ad3d2812539d05a293bfcabe8f0b884d8c91b01"
              ],
              "v2FileContract": {
                "filesize": 0,
                "fileMerkleRoot": "h:0000000000000000000000000000000000000000000000000000000000000000",
                "proofHeight": 200,
                "expirationHeight": 210,
                "renterOutput": {
                  "value": "0",
                  "address": "addr:000000000000000000000000000000000000000000000000000000000000000089eb0d6a8a69"
                },
                "hostOutput": {
                  "value": "10000000000000000000000000000",
                  "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
                },
                "missedHostValue": "0",
                "totalCollateral": "0",
                "renterPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "hostPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "revisionNumber": 0,
                "renterSignature": "sig:9d001e60633801956d1ce8b281b18a4b7da1249e8cb1e13b808f19c23e31c52596c303bd5efca278461877050412f1bec489037f101b7f41d3069906c60be30d",
                "hostSignature": "sig:9d001e60633801956d1ce8b281b18a4b7da1249e8cb1e13b808f19c23e31c52596c303bd5efca278461877050412f1bec489037f101b7f41d3069906c60be30d"
              }
            },
            "type": "storageProof",
            "resolution": {
              "proofIndex": {
                "id": "h:ee154b9b26af5a130d189c2467bd0157f24f4357478bfe5184243ab918c20290",
                "leafIndex": 416,
                "merkleProof": [],
                "chainIndex": {
                  "height": 200,
                  "id": "bid:ee154b9b26af5a130d189c2467bd0157f24f4357478bfe5184243ab918c20290"
                }
              },
              "leaf": "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
              "proof": []
            }
          },
          "siacoinElement": {
            "id": "h:a863dbc4f02efdfbf9f8d03e1aada090ede0a5752b71503787617d5f395c1335",
            "leafIndex": 418,
            "merkleProof": null,
            "siacoinOutput": {
              "value": "10000000000000000000000000000",
              "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
            },
            "maturityHeight": 345
          },
          "missed": false
        }
      }
    );

    let _event = serde_json::from_value::<Event>(j).unwrap();

    // FIXME this should deserialize from a JSON object generated from walletd and recalcuate the txid to check encoding/serde
}

#[test]
fn test_serde_event_v2_contract_resolution_renewal() {
    let j = json!(
      {
        "id": "h:debd3b8461d1aaa9011ba62d79c7ed7991eb0c60f9576880faadf2a8051aad54",
        "index": {
          "height": 203,
          "id": "bid:bd04c08bb96203c7f24adf2d405cb1069c7da8573573011379a986be62fc2a29"
        },
        "timestamp": "2024-07-18T19:04:16Z",
        "maturityHeight": 347,
        "type": "v2ContractResolution",
        "data": {
          "resolution": {
            "parent": {
              "id": "h:06b6349f4e76819aa36b7f1190d276b9ca97f0d5fc4564f153d6a36ed3c38033",
              "leafIndex": 423,
              "merkleProof": [
                "h:ba1427aad85e9985b61f262a2ea768a74f24af02d7e6c17f0cdb92559e7951ea",
                "h:147817a1d32c3f068be5456d935bc6cddd6306fe5633b576d91260d43a82e6d8",
                "h:f447a5360e1a7c4cab3062dd1699f56ea642b4f6cc6464fdfca0d1aa15fa436c",
                "h:1cdf40c0a759931ff590496b953938fbe7315394ce3726b4e4c4b81fed3d5498"
              ],
              "v2FileContract": {
                "filesize": 0,
                "fileMerkleRoot": "h:0000000000000000000000000000000000000000000000000000000000000000",
                "proofHeight": 211,
                "expirationHeight": 221,
                "renterOutput": {
                  "value": "10000000000000000000000000000",
                  "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
                },
                "hostOutput": {
                  "value": "0",
                  "address": "addr:000000000000000000000000000000000000000000000000000000000000000089eb0d6a8a69"
                },
                "missedHostValue": "0",
                "totalCollateral": "0",
                "renterPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "hostPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "revisionNumber": 0,
                "renterSignature": "sig:7d6f0e5b799c689dca7b55b1ff8ad028c7285b777d6df0e68235bde5778802adfb87e80afaf5d6c9b9fa63cd0e433aaa7189e3fdf2c7bf374c0ca20858071f03",
                "hostSignature": "sig:7d6f0e5b799c689dca7b55b1ff8ad028c7285b777d6df0e68235bde5778802adfb87e80afaf5d6c9b9fa63cd0e433aaa7189e3fdf2c7bf374c0ca20858071f03"
              }
            },
            "type": "renewal",
            "resolution": {
              "finalRevision": {
                "filesize": 0,
                "fileMerkleRoot": "h:0000000000000000000000000000000000000000000000000000000000000000",
                "proofHeight": 211,
                "expirationHeight": 221,
                "renterOutput": {
                  "value": "10000000000000000000000000000",
                  "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
                },
                "hostOutput": {
                  "value": "0",
                  "address": "addr:000000000000000000000000000000000000000000000000000000000000000089eb0d6a8a69"
                },
                "missedHostValue": "0",
                "totalCollateral": "0",
                "renterPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "hostPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "revisionNumber": 18446744073709551615u64,
                "renterSignature": "sig:00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                "hostSignature": "sig:00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
              },
              "newContract": {
                "filesize": 0,
                "fileMerkleRoot": "h:0000000000000000000000000000000000000000000000000000000000000000",
                "proofHeight": 221,
                "expirationHeight": 231,
                "renterOutput": {
                  "value": "10000000000000000000000000000",
                  "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
                },
                "hostOutput": {
                  "value": "0",
                  "address": "addr:000000000000000000000000000000000000000000000000000000000000000089eb0d6a8a69"
                },
                "missedHostValue": "0",
                "totalCollateral": "0",
                "renterPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "hostPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "revisionNumber": 0,
                "renterSignature": "sig:00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                "hostSignature": "sig:00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
              },
              "renterRollover": "0",
              "hostRollover": "0",
              "renterSignature": "sig:54a4bb0247518f62b20bf141686e2c05858e91acd23ae5e42436d173e331aca92af344e8cb9b5da98f0bdef01c7b7d840cbe7e781b8f7acc7c33b0fa44c7ef08",
              "hostSignature": "sig:54a4bb0247518f62b20bf141686e2c05858e91acd23ae5e42436d173e331aca92af344e8cb9b5da98f0bdef01c7b7d840cbe7e781b8f7acc7c33b0fa44c7ef08"
            }
          },
          "siacoinElement": {
            "id": "h:debd3b8461d1aaa9011ba62d79c7ed7991eb0c60f9576880faadf2a8051aad54",
            "leafIndex": 427,
            "merkleProof": null,
            "siacoinOutput": {
              "value": "10000000000000000000000000000",
              "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
            },
            "maturityHeight": 347
          },
          "missed": false
        }
      }
    );

    let _event = serde_json::from_value::<Event>(j).unwrap();

    // FIXME this should deserialize from a JSON object generated from walletd and recalcuate the txid to check encoding/serde
}

#[test]
#[ignore] // FIXME Error("expected an empty map for expiration", line: 0, column: 0)
fn test_serde_event_v2_contract_resolution_expiration() {
    let j = json!(
      {
        "id": "h:4c0170b9e82eacc2d14a13b974ce0c03560358276f135403bd060b53ce53be1c",
        "index": {
          "height": 190,
          "id": "bid:730f554f8cd5e6bd855b21b8c53f59808f3aa7351093f44da7761181283e3c6b"
        },
        "timestamp": "2024-07-18T19:04:16Z",
        "maturityHeight": 334,
        "type": "v2ContractResolution",
        "data": {
          "resolution": {
            "parent": {
              "id": "h:34f6bb9b9ed58dedebce2f39d29a526ea3012e9ae005cfca6a5257761c5412f6",
              "leafIndex": 351,
              "merkleProof": [
                "h:e805430ecdd47bcaca574f78721c3b6a24f0a877110fc9fa7ab347fd231a9885",
                "h:70782818a59e512d4995efd4ee94299e601496011b9c42b47eb0a3cd65aa89c9",
                "h:42ab48d2ef2b54352d44ab2ef33c1a6d76589360c0dd556d703a452b7d3e4a2c",
                "h:4af61bcae0a46d70f9b826b9bace336647389c38e6cb4c54356b9dd7fd6060aa",
                "h:59d21dd10aa3def083106844e23ad7f6b93e309c80b24a03e2c9b6eba8acef33",
                "h:f95c3f0fc4d632e5da8adcaa9249ea6b0c5fe66466a951871f5dc30a0c96b76d",
                "h:3374baebf913a23e0b9811ae22e72f6cdf6999d332ccda4b4dbab87f58b2a574"
              ],
              "v2FileContract": {
                "filesize": 0,
                "fileMerkleRoot": "h:0000000000000000000000000000000000000000000000000000000000000000",
                "proofHeight": 179,
                "expirationHeight": 189,
                "renterOutput": {
                  "value": "10000000000000000000000000000",
                  "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
                },
                "hostOutput": {
                  "value": "0",
                  "address": "addr:000000000000000000000000000000000000000000000000000000000000000089eb0d6a8a69"
                },
                "missedHostValue": "0",
                "totalCollateral": "0",
                "renterPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "hostPublicKey": "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc",
                "revisionNumber": 0,
                "renterSignature": "sig:c293b22c9feee5a081699ddbf83486704df855129c2bbe27c2dc56afcb7e68cd355785fa36954471c1e48691864b240969168422b1fd6396e18f720ebec50e00",
                "hostSignature": "sig:c293b22c9feee5a081699ddbf83486704df855129c2bbe27c2dc56afcb7e68cd355785fa36954471c1e48691864b240969168422b1fd6396e18f720ebec50e00"
              }
            },
            "type": "expiration",
            "resolution": {}
          },
          "siacoinElement": {
            "id": "h:4c0170b9e82eacc2d14a13b974ce0c03560358276f135403bd060b53ce53be1c",
            "leafIndex": 391,
            "merkleProof": null,
            "siacoinOutput": {
              "value": "10000000000000000000000000000",
              "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
            },
            "maturityHeight": 334
          },
          "missed": true
        }
      }
    );

    let _event = serde_json::from_value::<Event>(j).unwrap();
}

#[test]
#[ignore] // I don't have a good test case for this yet because wallet_test.go TestEventTypes doesn't output this type
fn test_serde_event_v2_contract_resolution_finalization() {
    let j = json!(
      {
        "id": "h:4057e021e1d6dec8d4e4ef9d6e9fa2e4491c559144848b9af5765e03b39bb69d",
        "index": {
          "height": 0,
          "id": "bid:0000000000000000000000000000000000000000000000000000000000000000"
        },
        "timestamp": "2024-07-12T10:04:18.564506-07:00",
        "maturityHeight": 0,
        "type": "v2ContractResolution",
        "data": {
          "parent": {
            "id": "h:ee87ab83f9d16c9377d6154c477ac40d2ee70619de2ba146fcfe36fd0de86bf5",
            "leafIndex": 6680213938505633000u64,
            "merkleProof": [
              "h:0000000000000000000000000000000000000000000000000000000000000000",
              "h:0000000000000000000000000000000000000000000000000000000000000000",
              "h:0000000000000000000000000000000000000000000000000000000000000000",
              "h:0000000000000000000000000000000000000000000000000000000000000000",
              "h:0000000000000000000000000000000000000000000000000000000000000000"
            ],
            "v2FileContract": {
              "filesize": 0,
              "fileMerkleRoot": "h:0000000000000000000000000000000000000000000000000000000000000000",
              "proofHeight": 10,
              "expirationHeight": 20,
              "renterOutput": {
                "value": "10000000000000000000000000000",
                "address": "addr:c899f7795bb20c94e57c764f06699e09e6ad071ad95539eef4fb505e79ab22e8be4d64067ccc"
              },
              "hostOutput": {
                "value": "0",
                "address": "addr:000000000000000000000000000000000000000000000000000000000000000089eb0d6a8a69"
              },
              "missedHostValue": "0",
              "totalCollateral": "0",
              "renterPublicKey": "ed25519:65ea9701c409d4457a830b6fe7a2513d6f466ab4e424b3941de9f34a4a2d6170",
              "hostPublicKey": "ed25519:65ea9701c409d4457a830b6fe7a2513d6f466ab4e424b3941de9f34a4a2d6170",
              "revisionNumber": 0,
              "renterSignature": "sig:bd1794b9266fa0de94aea0f0ffb6550efd7e8874133963022413c8ccfe1a0e31c14690d3a5bbd343b160ed59219bd67f79103c45aee07f519d72b5ab4319440f",
              "hostSignature": "sig:bd1794b9266fa0de94aea0f0ffb6550efd7e8874133963022413c8ccfe1a0e31c14690d3a5bbd343b160ed59219bd67f79103c45aee07f519d72b5ab4319440f"
            }
          },
          "type": "finalization",
          "resolution": {
            "filesize": 0,
            "fileMerkleRoot": "h:0000000000000000000000000000000000000000000000000000000000000000",
            "proofHeight": 10,
            "expirationHeight": 20,
            "renterOutput": {
              "value": "10000000000000000000000000000",
              "address": "addr:c899f7795bb20c94e57c764f06699e09e6ad071ad95539eef4fb505e79ab22e8be4d64067ccc"
            },
            "hostOutput": {
              "value": "0",
              "address": "addr:000000000000000000000000000000000000000000000000000000000000000089eb0d6a8a69"
            },
            "missedHostValue": "0",
            "totalCollateral": "0",
            "renterPublicKey": "ed25519:65ea9701c409d4457a830b6fe7a2513d6f466ab4e424b3941de9f34a4a2d6170",
            "hostPublicKey": "ed25519:65ea9701c409d4457a830b6fe7a2513d6f466ab4e424b3941de9f34a4a2d6170",
            "revisionNumber": 18446744073709551615u64,
            "renterSignature": "sig:bd1794b9266fa0de94aea0f0ffb6550efd7e8874133963022413c8ccfe1a0e31c14690d3a5bbd343b160ed59219bd67f79103c45aee07f519d72b5ab4319440f",
            "hostSignature": "sig:bd1794b9266fa0de94aea0f0ffb6550efd7e8874133963022413c8ccfe1a0e31c14690d3a5bbd343b160ed59219bd67f79103c45aee07f519d72b5ab4319440f"
          }
        }
      }
    );

    let _event = serde_json::from_value::<Event>(j).unwrap();

    // FIXME this should deserialize from a JSON object generated from walletd and recalcuate the txid to check encoding/serde
}

#[test]
fn test_serde_event_v2_transaction() {
    let j = json!(
      {
        "id": "h:5900e475aace932c94bcc94cf296596ccff1d77d9aba52a079e9f429605671cd",
        "index": {
          "height": 203,
          "id": "bid:bd04c08bb96203c7f24adf2d405cb1069c7da8573573011379a986be62fc2a29"
        },
        "timestamp": "2024-07-18T19:04:16Z",
        "maturityHeight": 203,
        "type": "v2Transaction",
        "data": {
          "siacoinInputs": [
            {
              "parent": {
                "id": "h:78d58090bcdeaccf22abf99b6e0de25273e9eb82210359a16cefbd743a85fd50",
                "leafIndex": 421,
                "merkleProof": [
                  "h:f26accb7c256e867a9ed62671ebe6c3eb34d085e5266f67073af2daa549f980d",
                  "h:d39e139147168c70da11c3f6db4fa54d35914ef67ba5654a75107da9c099ddda",
                  "h:f447a5360e1a7c4cab3062dd1699f56ea642b4f6cc6464fdfca0d1aa15fa436c"
                ],
                "siacoinOutput": {
                  "value": "256394172736732570239334030000",
                  "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
                },
                "maturityHeight": 0
              },
              "satisfiedPolicy": {
                "policy": {
                  "type": "uc",
                  "policy": {
                    "timelock": 0,
                    "publicKeys": [
                      "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc"
                    ],
                    "signaturesRequired": 1
                  }
                },
                "signatures": [
                  "sig:c432fea5f147205e49235ddbd75c232fd8e9c7526b2b1575f70653ae2b3c0d0338c7fe710be338482060cf6ef2dea5e2319252fc28deaf70c77a2be60a533400"
                ]
              }
            }
          ],
          "siacoinOutputs": [
            {
              "value": "10400000000000000000000000000",
              "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
            },
            {
              "value": "245994172736732570239334030000",
              "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
            }
          ],
          "minerFee": "0"
        }
      }
    );
    test_serde!(Event, j);
}

#[test]
fn test_v2_transaction_serde_basic_send() {
    let j = json!(
        {
            "siacoinInputs": [
                {
                    "parent": {
                        "id": "h:f59e395dc5cbe3217ee80eff60585ffc9802e7ca580d55297782d4a9b4e08589",
                        "leafIndex": 3,
                        "merkleProof": [
                            "h:ab0e1726444c50e2c0f7325eb65e5bd262a97aad2647d2816c39d97958d9588a",
                            "h:467e2be4d8482eca1f99440b6efd531ab556d10a8371a98a05b00cb284620cf0",
                            "h:64d5766fce1ff78a13a4a4744795ad49a8f8d187c01f9f46544810049643a74a",
                            "h:31d5151875152bc25d1df18ca6bbda1bef5b351e8d53c277791ecf416fcbb8a8",
                            "h:12a92a1ba87c7b38f3c4e264c399abfa28fb46274cfa429605a6409bd6d0a779",
                            "h:eda1d58a9282dbf6c3f1beb4d6c7bdc036d14a1cfee8ab1e94fabefa9bd63865",
                            "h:e03dee6e27220386c906f19fec711647353a5f6d76633a191cbc2f6dce239e89",
                            "h:e70fcf0129c500f7afb49f4f2bb82950462e952b7cdebb2ad0aa1561dc6ea8eb"
                        ],
                        "siacoinOutput": {
                            "value": "300000000000000000000000000000",
                            "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
                        },
                        "maturityHeight": 145
                    },
                    "satisfiedPolicy": {
                        "policy": {
                            "type": "uc",
                            "policy": {
                                "timelock": 0,
                                "publicKeys": [
                                    "ed25519:cecc1507dc1ddd7295951c290888f095adb9044d1b73d696e6df065d683bd4fc"
                                ],
                                "signaturesRequired": 1
                            }
                        },
                        "signatures": [
                            "sig:f0a29ba576eb0dbc3438877ac1d3a6da4f3c4cbafd9030709c8a83c2fffa64f4dd080d37444261f023af3bd7a10a9597c33616267d5371bf2c0ade5e25e61903"
                        ]
                    }
                }
            ],
            "siacoinOutputs": [
                {
                    "value": "1000000000000000000000000000",
                    "address": "addr:000000000000000000000000000000000000000000000000000000000000000089eb0d6a8a69"
                },
                {
                    "value": "299000000000000000000000000000",
                    "address": "addr:f7843ac265b037658b304468013da4fd0f304a1b73df0dc68c4273c867bfa38d01a7661a187f"
                }
            ],
            "minerFee": "0"
        }
    );
    let tx = serde_json::from_value::<V2Transaction>(j).unwrap();

    let j2 = serde_json::to_value(&tx).unwrap().to_string();
    let tx2 = serde_json::from_str::<V2Transaction>(&j2).unwrap();
    assert_eq!(tx, tx2);
}

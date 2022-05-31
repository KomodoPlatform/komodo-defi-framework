use chain::{BlockHeader, RawBlockHeader};
use primitives::hash::H256;
use primitives::U256;
use ripemd160::Digest;
use serialization::parse_compact_int;
use sha2::Sha256;
use types::{extract_script_sig_len, MerkleArray, SPVError, TxIn};

/// Determines the length of an input from its scriptsig:
/// 36 for outpoint, 1 for scriptsig length, 4 for sequence.
///
/// # Arguments
///
/// * `tx_in` - The input as a u8 array
pub fn determine_input_length(tx_in: &[u8]) -> Result<usize, SPVError> {
    let script_sig_len = extract_script_sig_len(&TxIn(tx_in))?;
    // 40 = 36 (outpoint) + 4 (sequence)
    Ok(40 + script_sig_len.serialized_length() + script_sig_len.as_usize())
}

//
// Outputs
//

/// Determines the length of an output.
/// 5 types: WPKH, WSH, PKH, SH, and OP_RETURN.
///
/// # Arguments
///
/// * `tx_out` - The output
///
/// # Errors
///
/// * Errors if CompactInt represents a number larger than 253; large CompactInts are not supported.
pub fn determine_output_length(tx_out: &[u8]) -> Result<usize, SPVError> {
    if tx_out.len() < 9 {
        return Err(SPVError::MalformattedOutput);
    }
    let script_pubkey_len = parse_compact_int(&tx_out[8..]).map_err(|_| SPVError::BadCompactInt)?;

    Ok(8 + script_pubkey_len.serialized_length() + script_pubkey_len.as_usize())
}

//
// Transaction
//

/// Checks that the vin passed up is properly formatted;
/// Consider a vin with a valid vout in its scriptsig.
///
/// # Arguments
///
/// * `vin` - Raw bytes length-prefixed input vector
pub fn validate_vin(vin: &[u8]) -> bool {
    let n_ins = match parse_compact_int(vin) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let vin_length = vin.len();

    let mut offset = n_ins.serialized_length();
    if n_ins.as_usize() == 0usize {
        return false;
    }

    for _ in 0..n_ins.as_usize() {
        if offset >= vin_length {
            return false;
        }
        match determine_input_length(&vin[offset as usize..]) {
            Ok(v) => offset += v as usize,
            Err(_) => return false,
        };
    }

    offset == vin_length
}

/// Checks that the vout passed up is properly formatted;
/// Consider a vin with a valid vout in its scriptsig.
///
/// # Arguments
///
/// * `vout` - Raw bytes length-prefixed output vector
pub fn validate_vout(vout: &[u8]) -> bool {
    let n_outs = match parse_compact_int(vout) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let vout_length = vout.len();

    let mut offset = n_outs.serialized_length();
    if n_outs.as_usize() == 0usize {
        return false;
    }

    for _ in 0..n_outs.as_usize() {
        if offset >= vout_length {
            return false;
        }
        match determine_output_length(&vout[offset as usize..]) {
            Ok(v) => offset += v as usize,
            Err(_) => return false,
        };
    }

    offset == vout_length
}

/// Implements bitcoin's hash256 (double sha2).
/// Returns the digest.
///
/// # Arguments
///
/// * `preimage` - The pre-image
pub fn hash256(preimages: &[&[u8]]) -> H256 {
    let mut sha = Sha256::new();
    for preimage in preimages.iter() {
        sha.update(preimage);
    }
    let digest = sha.finalize();

    let mut second_sha = Sha256::new();
    second_sha.update(digest);
    let buf: [u8; 32] = second_sha.finalize().into();
    buf.into()
}

/// Concatenates and hashes two inputs for merkle proving.
///
/// # Arguments
///
/// * `a` - The first hash
/// * `b` - The second hash
pub fn hash256_merkle_step(a: &[u8], b: &[u8]) -> H256 { hash256(&[a, b]) }

/// Verifies a Bitcoin-style merkle tree.
/// Leaves are 0-indexed.
/// Note that `index` is not a reliable indicator of location within a block.
///
/// # Arguments
///
/// * `proof` - The proof. Tightly packed LE sha256 hashes.  The last hash is the root
/// * `index` - The index of the leaf
pub fn verify_hash256_merkle(txid: H256, merkle_root: H256, intermediate_nodes: &MerkleArray, index: u64) -> bool {
    let mut idx = index;
    let proof_len = intermediate_nodes.len();

    match proof_len {
        0 => return txid == merkle_root,
        1 => return true,
        _ => {},
    }

    let mut current = txid;

    for i in 0..proof_len {
        let next = intermediate_nodes.index(i);

        if idx % 2 == 1 {
            current = hash256_merkle_step(next.as_slice(), current.as_slice());
        } else {
            current = hash256_merkle_step(current.as_slice(), next.as_slice());
        }
        idx >>= 1;
    }

    current == merkle_root
}

/// Evaluates a Bitcoin merkle inclusion proof.
/// Note that `index` is not a reliable indicator of location within a block.
///
/// # Arguments
///
/// * `txid` - The txid (LE)
/// * `merkle_root` - The merkle root (as in the block header) (LE)
/// * `intermediate_nodes` - The proof's intermediate nodes (digests between leaf and root) (LE)
/// * `index` - The leaf's index in the tree (0-indexed)
///
/// # Notes
/// Wrapper around `bitcoin_spv::validatespv::prove`
pub fn merkle_prove(txid: H256, merkle_root: H256, intermediate_nodes: Vec<H256>, index: u64) -> Result<(), SPVError> {
    if txid == merkle_root && index == 0 && intermediate_nodes.is_empty() {
        return Ok(());
    }
    let vec: Vec<u8> = intermediate_nodes.into_iter().flat_map(|node| node.take()).collect();
    let nodes = MerkleArray::new(vec.as_slice())?;
    if !verify_hash256_merkle(txid.take().into(), merkle_root.take().into(), &nodes, index) {
        return Err(SPVError::BadMerkleProof);
    }
    Ok(())
}

fn validate_header_prev_hash(actual: &H256, to_compare_with: &H256) -> bool { actual == to_compare_with }

pub fn validate_header_work(digest: H256, target: &U256) -> bool {
    let empty = H256::default();

    if digest == empty {
        return false;
    }

    U256::from_little_endian(digest.as_slice()) < *target
}

/// Checks validity of header chain.
/// Compares the hash of each header to the prevHash in the next header.
///
/// # Arguments
///
/// * `headers` - Raw byte array of header chain
/// * `difficulty_check`: Rather the difficulty need to check or not, usefull for chain like Qtum (Pos)
/// or KMD/SmartChain (Difficulty change NN)
/// * `constant_difficulty`: If we do not expect difficulty change (BTC difficulty change every 2016 blocks)
/// use this variable to false when you do not have a chance to use a checkpoint
///
/// # Errors
///
/// * Errors if header chain is invalid, insufficient work, unexpected difficulty change or unable to get a target
///
/// # Notes
/// Wrapper inspired by `bitcoin_spv::validatespv::validate_header_chain`
pub fn validate_headers(
    headers: Vec<BlockHeader>,
    difficulty_check: bool,
    constant_difficulty: bool,
) -> Result<(), SPVError> {
    let mut previous_hash = H256::default();
    let mut target = U256::default();
    for (i, header) in headers.into_iter().enumerate() {
        let raw_header = RawBlockHeader::from(header.clone());
        if i == 0 {
            target = match header.target() {
                Ok(target) => target,
                Err(_) => return Err(SPVError::UnableToGetTarget),
            };
        }
        let cur_target = match header.target() {
            Ok(target) => target,
            Err(_) => return Err(SPVError::UnableToGetTarget),
        };
        if (!constant_difficulty && difficulty_check) && cur_target != target {
            return Err(SPVError::UnexpectedDifficultyChange);
        }
        if i != 0 && !validate_header_prev_hash(&raw_header.parent(), &previous_hash) {
            return Err(SPVError::InvalidChain);
        }
        if difficulty_check && !validate_header_work(raw_header.digest(), &target) {
            return Err(SPVError::InsufficientWork);
        }
        previous_hash = raw_header.digest();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;

    use std::{println, vec};
    use test_helpers::for_tests::force_deserialize_hex;

    use crate::test_utils::{self};
    use crate::types::{self};

    #[test]
    fn it_does_bitcoin_hash256() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("hash256", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let mut expected = H256::default();
                let output = force_deserialize_hex(case.output.as_str().unwrap());
                expected.as_mut().copy_from_slice(&output);
                assert_eq!(hash256(&[&input]), expected);
            }
        })
    }

    #[test]
    fn it_computes_hash256_merkle_steps() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("hash256MerkleStep", &fixtures);
            for case in test_cases {
                let inputs = case.input.as_array().unwrap();
                let a = force_deserialize_hex(inputs[0].as_str().unwrap());
                let b = force_deserialize_hex(inputs[1].as_str().unwrap());
                let mut expected = H256::default();
                let output = force_deserialize_hex(case.output.as_str().unwrap());
                expected.as_mut().copy_from_slice(&output);
                assert_eq!(hash256_merkle_step(&a, &b), expected);
            }
        })
    }

    #[test]
    fn it_extracts_script_sig_length_info() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("extractScriptSigLen", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());

                let outputs = case.output.as_array().unwrap();
                let expected_num = outputs[1].as_u64().unwrap() as usize;

                assert_eq!(extract_script_sig_len(&TxIn(&input)).unwrap().as_usize(), expected_num);
            }
        })
    }

    #[test]
    fn it_extracts_legacy_le_sequence_info() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("extractSequenceLELegacy", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let expected: &[u8] = &force_deserialize_hex(case.output.as_str().unwrap());
                assert_eq!(&types::extract_sequence_le(&TxIn(&input)).unwrap(), expected);
            }
        })
    }

    #[test]
    fn it_extracts_legacy_sequence_info() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("extractSequenceLegacy", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let expected = case.output.as_u64().unwrap() as u32;
                assert_eq!(types::extract_sequence(&TxIn(&input)).unwrap(), expected);
            }
        })
    }

    #[test]
    fn it_determines_input_length() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("determineInputLength", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let expected = case.output.as_u64().unwrap() as usize;
                assert_eq!(determine_input_length(&input).unwrap(), expected);
            }
        })
    }

    #[test]
    fn it_extracts_scipt_sigs() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("extractScriptSig", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let expected: &[u8] = &force_deserialize_hex(case.output.as_str().unwrap());
                assert_eq!(types::extract_script_sig(&TxIn(&input)).unwrap(), expected);
            }
        })
    }

    #[test]
    fn it_extracts_witness_le_sequence_numbers() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("extractSequenceLEWitness", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let expected: &[u8] = &force_deserialize_hex(case.output.as_str().unwrap());
                assert_eq!(types::extract_sequence_le(&TxIn(&input)).unwrap(), expected);
            }
        })
    }

    #[test]
    fn it_extracts_witness_sequence_numbers() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("extractSequenceWitness", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let expected = case.output.as_u64().unwrap() as u32;
                assert_eq!(types::extract_sequence(&TxIn(&input)).unwrap(), expected);
            }
        })
    }

    #[test]
    fn it_determines_output_length() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("determineOutputLength", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let expected = case.output.as_u64().unwrap() as usize;
                assert_eq!(determine_output_length(&input).unwrap(), expected);
            }
        })
    }

    #[test]
    fn it_validates_vin_syntax() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("validateVin", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let expected = case.output.as_bool().unwrap();
                assert_eq!(validate_vin(&input), expected);
            }
        })
    }

    #[test]
    fn it_validates_vout_syntax() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("validateVout", &fixtures);
            for case in test_cases {
                let input = force_deserialize_hex(case.input.as_str().unwrap());
                let expected = case.output.as_bool().unwrap();
                assert_eq!(validate_vout(&input), expected);
            }
        })
    }

    #[test]
    fn it_verifies_hash256_merkles() {
        test_utils::run_test(|fixtures| {
            let test_cases = test_utils::get_test_cases("verifyHash256Merkle", &fixtures);
            for case in test_cases {
                let inputs = case.input.as_object().unwrap();
                let extended_proof = force_deserialize_hex(inputs.get("proof").unwrap().as_str().unwrap());
                let proof_len = extended_proof.len();
                if proof_len < 32 {
                    continue;
                }

                let index = inputs.get("index").unwrap().as_u64().unwrap() as u64;
                let expected = case.output.as_bool().unwrap();

                // extract root and txid
                let mut root = H256::default();
                let mut txid = H256::default();
                println!("{:?}", extended_proof);
                root.as_mut().copy_from_slice(&extended_proof[proof_len - 32..]);
                txid.as_mut().copy_from_slice(&extended_proof[..32]);

                let proof = if proof_len > 64 {
                    extended_proof[32..proof_len - 32].to_vec()
                } else {
                    vec![]
                };

                println!("{:?} {:?} {:?} {:?}", root, txid, proof, proof.len());

                assert_eq!(
                    verify_hash256_merkle(txid, root, &MerkleArray::new(&proof).unwrap(), index),
                    expected
                );
            }
        })
    }

    #[test]
    fn test_merkle_prove_inclusion() {
        // https://rick.explorer.dexstats.info/tx/7e9797a05abafbc1542449766ef9a41838ebbf6d24cd3223d361aa07c51981df
        // merkle intermediate nodes 2 element
        let tx_id: H256 = H256::from_reversed_str("7e9797a05abafbc1542449766ef9a41838ebbf6d24cd3223d361aa07c51981df");
        let merkle_pos = 1;
        let merkle_root: H256 =
            H256::from_reversed_str("41f138275d13690e3c5d735e2f88eb6f1aaade1207eb09fa27a65b40711f3ae0").into();
        let merkle_nodes: Vec<H256> = vec![
            H256::from_reversed_str("73dfb53e6f49854b09d98500d4899d5c4e703c4fa3a2ddadc2cd7f12b72d4182"),
            H256::from_reversed_str("4274d707b2308d39a04f2940024d382fa80d994152a50d4258f5a7feead2a563"),
        ];
        let result = merkle_prove(tx_id, merkle_root, merkle_nodes, merkle_pos);
        result.unwrap()
    }

    #[test]
    fn test_merkle_prove_inclusion_single_element() {
        // https://www.blockchain.com/btc/tx/c06fbab289f723c6261d3030ddb6be121f7d2508d77862bb1e484f5cd7f92b25
        // merkle intermediate nodes single element
        let tx_id: H256 = H256::from_reversed_str("c06fbab289f723c6261d3030ddb6be121f7d2508d77862bb1e484f5cd7f92b25");
        let merkle_pos = 0;
        let merkle_root: H256 =
            H256::from_reversed_str("8fb300e3fdb6f30a4c67233b997f99fdd518b968b9a3fd65857bfe78b2600719").into();
        let merkle_nodes: Vec<H256> = vec![H256::from_reversed_str(
            "5a4ebf66822b0b2d56bd9dc64ece0bc38ee7844a23ff1d7320a88c5fdb2ad3e2",
        )];
        let result = merkle_prove(tx_id, merkle_root, merkle_nodes, merkle_pos);
        result.unwrap()
    }

    #[test]
    fn test_merkle_prove_inclusion_complex() {
        // https://www.blockchain.com/btc/tx/b36bced99cc459506ad2b3af6990920b12f6dc84f9c7ed0dd2c3703f94a4b692
        // merkle intermediate nodes complex merkle proof inclusion
        let tx_id: H256 = H256::from_reversed_str("b36bced99cc459506ad2b3af6990920b12f6dc84f9c7ed0dd2c3703f94a4b692");
        let merkle_pos = 680;
        let merkle_root: H256 =
            H256::from_reversed_str("def7a26d91789069dad448cb4b68658b7ba419f9fbd28dce7fe32ed0010e55df").into();
        let merkle_nodes: Vec<H256> = vec![
            H256::from_reversed_str("39141331f2b7133e72913460384927b421ffdef3e24b88521e7ac54d30019409"),
            H256::from_reversed_str("39aeb77571ee0b0cf9feb7e121938b862f3994ff1254b34559378f6f2ed8b1fb"),
            H256::from_reversed_str("5815f83f4eb2423c708127ea1f47feeabcf005d4aed18701d9692925f152d0b4"),
            H256::from_reversed_str("efbb90aae6875af1b05a17e53fabe79ca1655329d6e107269a190739bf9d9038"),
            H256::from_reversed_str("20eb7431ae5a185e89bd2ad89956fc660392ee9d231df58600ac675734013e82"),
            H256::from_reversed_str("1f1dd980e6196ec4de9037941076a6030debe466dfc177e54447171b64ea99e5"),
            H256::from_reversed_str("bbc4264359bec656298e31443034fc3ff9877752b765b9665b4da1eb8a32d1ff"),
            H256::from_reversed_str("71788bf5224f228f390243a2664d41d96bae97ae1e4cfbc39095448e4cd1addd"),
            H256::from_reversed_str("1b24a907c86e59eb698afeb4303c00fe3ecf8425270134ed3d0e62c6991621f2"),
            H256::from_reversed_str("7776b46bb148c573d5eabe1436a428f3dae484557fea6efef1da901009ca5f8f"),
            H256::from_reversed_str("623a90d6122a233b265aab497b13bb64b5d354d2e2112c3f554e51bfa4e6bbd3"),
            H256::from_reversed_str("3104295d99163e16405b80321238a97d02e2448bb634017e2e027281cc4af9e8"),
        ];
        let result = merkle_prove(tx_id, merkle_root, merkle_nodes, merkle_pos);
        result.unwrap()
    }

    #[test]
    fn test_block_headers_no_difficulty_check() {
        // morty: 1330480, 1330481, 1330482
        let headers: Vec<BlockHeader> = vec![
            "04000000bb496ba8d09f8f98b15cdaf5798163bdd70676eb1c8b538f53ab4f83da4a27000db352177c6b5ad2499a906cec33b843fb17fc1ec298cd06c7e7ceb7b62e144232d719d14c15e565c05e84ead95a2f101a1b658ee2f36eb7ca65206e27cfca473de614625be6071f09006c286bc5ec73dd27a09bf687700c06fb04d0b9a063c0aa0746c9db170000fd40050053b27dad1f5a858b78f3154039759e985ed57db10ecb772810d7f158c55083a14b9f2ba26ae9fcb82012186e2528f67c45b7b216a69fe26232ad2d179a141b1b10e4d5f108c7b920b49348f6eef2d70b7f02cb01d8d9992f8f2d7b6608806b10ff329846b188de200aa37c73ac03f6c9b79cf5613c71b7969b4abafdbc1165ad955a049269584c83b36f36a3e9becf2fe81f3b1917475eb13ecfed3813ecc32206078d8c1e2797013dfc6f6a55e06f1c06a07959ef94d53ca0fc81d03cb6f614761156ed4ff1a8e5c9f0b96f3c8c3eeb9a0720cf4ed10397330f49b83439c5083eea1d1785a10d86ca2866d0da4ca746c49118b780c55aa6cd5b4c0491cefa258ecf129307d15e001415b203e89c008f4444b236aa556dbf4f6d05e0c57642cfa142df2f8546f1d37a6b2feaf98496892b41caefbe7dc7bcbb2755752df3dbf00ac1fc558896f14541aea4cc78ec5d00bbe5398fac4a658b1ae3399777f15117c0f3de3c63bc5b3edf6543d172cfc66907f9cf8706e97b14281daeb427801dfb0910743873265ae6bae71dbf22353c321f726e68f747965858f488dd507b7e6adee42509e5720373dce5b111b420c906b0f2cb391cfb9d581e2509da3829d6718469f383e07043694db87db0ce1196449a6c9cd941a8bde507e553c0ca534238dcc93633631926102c87cd0f83720ccff60de8b05b103e086a2c2cb7943f21033a5658235fc52708907e1ea722e726808db0270bf898c51e9dd0745614857783dc11a6dcd7760d4a07ddbd83a2e02b23fa789b79eed22dc411b9b48f71c54f12387065e3ff0638701e0f6a0dd56d0ce395d150b237b60c166352e69b92173b884446d7660f5857458b97c6d4ee54f8a1f60113aff30e54c1f7c572b85dcb7a2419d2f736a9b0a6d99ea549bd74e546251c0b8be7975e9a6d96aa3467b1dc6b024745fdef43b37cf21a657a3247d9adf8c252ef210d9a4e9c7191f698ccc9b10103b8bb811cdcf1a62903786476db8195ffb3cd004c57ad07a7a3c41eee391f66a7697e69409d7a78558720f6a1b9804d72de820b7b6165b8e14a2b1316576022423f22bb82fab16127be7173ddcd43fa7ea5c4474f79321a8c4b792caf12320c3047d026b7d63216a022e83655c2d811d2bd2a559970e9155b979953f9801ce918f690f43f5e3f07f7ce27a6837bf33b2490d9add8549f1e603a750c114bb92740cc3987cb9f948a6229f175a7b577b0b60d885a0a7ef05debe921376a7acdb25eaa8bb72e120e529cd775175012efb454cf41d240a946bf140af20d9a5dbed2e196d91a7ff33c2769f140fa0bb968111e1602221deae8d162e7a471354c2051acb43ec31015aaefa0b08bf1bddbb282e86a1caf45f3b63e4c6427ba9e99aed28ef79711794511511c52daf13b735e02b9833d3467bfd16886606d5555b7cc95ff2fea3b03c82cfe60e8602d9f70a3870f5b755573b955bb300bd3733b5ddf9a61fd3cd281af39520d6dfd8b7e2b165ec91749614a3b5241e2ea12470f91b58cf6163e02dfe79392db70cd17db9497cf59c89ac8377dbd02042f6ed270c8c2bc717623b203b74676890f5f4cd905b25772a25292d76b6f42a094c27eed13793d189e395ed3f28c5731976a7b45184acee45b3cf05a9c62045644dfe39f79cd331e282edae99cea652eb82819415ac2a5c21539cdd636fb835063ace3b6befffaf50bf6866e9b1a2b35037a330faeb18ca1696693dafd26b5f5da8dcd3e50ff09249bdda695f576d25024560b643d873d07293a80fe71998ef6ccd88c0cf9f69326b463c26fe4906faaf454ae68accd7ef3edffefdd2ede23a822a2267332f0791f1c4e6d5ab4661f279f5039b36a4476e56fd5b0461e585ff30a7c661b93f1".into(),
            "04000000001f22e1bc88c53b1554f8fdcf261fdb09f4cae6ef5e5032b788515f4a60d30d67d1b35fda68abc05f5af39e5ade224a5312b8dcd1f3629a7ff33355bb7ca93e32d719d14c15e565c05e84ead95a2f101a1b658ee2f36eb7ca65206e27cfca478be6146220bb071f49000b055b22a7a4bbafd6b52efb90f963d5f80126c27e437005fb47720e0000fd4005004d9875d71c540f558813142e263f597243bdd8d8105ff3d1ffd62ae51ccf22729debe510f97ab0631701dbd34b73e570597dc8825be6bd669e693037fb701040c273b44745f4e850c2d8aeca7ccab6ef7f462206a16d75358f2e8fddf9d0dbc6333ff55b1813a37f0ba240bd2d897fbd6cfdb1989ac8f3ec93b15ae4360edf84088ac9a4ea7d3d71290532bb51675e7310be1210aa33c184d693f6f7c15c5be1e89356ae3d663d0c548fceac0974fe4cb6c6559f50643280df9508460fd04f9cde55521b4c6d61c644c6c7b7473f9e39b412e3776f5e47b6c466aaf1dc76ff2114e716eb6b9614d0c93cdc229ec13b07057a7f7446c1aac51ef0950d4361fa2d20f22f29ff490bf6d6a2a267c45d88d3152d9f5291695f2f4fba65ca9763cb4176506c73b8162611b6004af7ec8d1ea55a225cca2576e4ac84ac333b663693a2f19f7786340ad9d2212d576a0b4e7700bd7d60de88940dce1f01481f9c41350eefd7b496218bcf70c4c8922dfd18d666d37d10cb0f14dd38e1225ec179dcab5501a4434674d6f9ff9f23c4df5f445cc2accf43189fc99ac56693df373a4207b0dc991009fae4796fd7e49cea4dd139ee72264dfd47f4e1ad2420d635c7a1f37950d022ffdcccc7651b645db0ba0ce94c18dcc902279b4601806beefe05016f1f85411e6562b584da0854db2e36f602d8c4974d385aee4a01d1132082c8cd7c71443162f7d7487c73d8a46f830f72a0d352d957bef5afc33c4447ef33b2491e28000d1f4687e95ffc2b9532d28ae4c48f8551bf527dbe18c672204495f2bd546566fd5770189e28c2de0974130a492ccd8737a8c6e971d02a23c4f9f27410348d1f666f93385bdc81bad8e9a9d1dbffdfa2609ebae52740b457ecd67a3bf0db02a14f5bdf3e25b35b2d3d303094e46e0e3daef559d9f0e074e512bcaf9fcc9d035083eec16806af8a93d27b4ad46754a425b6a02b1ac22f682e48f214d66b379d7042aa39f2c5f3448d05ca4b6360e162f31f197225f4ad579d69207c666711fb3f6ca814efcf430899360cced1168cd69ec0e809a89cf2cf2015f9f895a3dadd4ced6d94793e98201b1da6a0a5d90be5d06925e3ad60b9227f84b9c3060a6db6e7857d8731f975d4a993abf10d84590da02b114625109d864de070813179b651d528f66036c30a0700ee84fc5e59757a509745b64e76fa3396f3c8b01a7724cd434e6d774dad36be8a73ad29f6859352aa15236e7825947396cb98e26b912b19ddc127590e59200c4334d1d96d7585a0e349b920f2e4e59cdedac911214c42c0894f72c8a7423d7aef3ea5ef9a5b650821f46537c65509ad8dcf6558c16c04f9877c737ff81875d9fbe01d23d37e937444cf257b0b57bc1c2a774f2e2bf5f3b0881be0e2282ba97ef6aad797f8fdb4053da4e478575805c7a93076c09847544a8e89f1cb3838df7870bcf61deb2144c6f6349c966b67545703058f9227965b97835b049538fb428431a8461586b022368626d20e9b6bfdd7232a5cc6a0aa214319cb440c45443a2446d1e17713c0e1049f0fd759d1dbff493302140376cfb153330ed455a043189260cb7d2d90333a37d3584f2d907d0a73dccee299ad14141d60d1409cda688464a13b5dab37476641741717d599a60c0ac84d85869ed449f83933ad30e2591157fd1f07b73ecf26f34e91bc00f1ca86ae34ca8231b372cdc2ed18d463ac42f92859d6f0e2c483dbb23d785f1233db2033458af9d7c1e7029ac5cc33ca7d25b2b49fd71b1ae5f5ce969b6e77333bf5fbb5e6645dd0a4d0c6e82eb534ac264ddbe28513e4b82b3578c1a6cbfaa2522aa50985fe2cce43cf3363eaacca0e09c721fd603d43c3a4fdf8dde0c9ff2c054910b16aeef7c4d86b31".into(),
            "04000000fcead9a1b425124f11aa97e0614120ce87bdddcad655672916f9c4564dc057002bd3df07a4602620282b276359529114ba89b59b16bec235d584c3cf5cc6b2d132d719d14c15e565c05e84ead95a2f101a1b658ee2f36eb7ca65206e27cfca47bfe61462d5b9071f1a001daf299c51afbd74fd75a98ba49a6e40ae8ad92b3afdc1cf215fd6190000fd40050044b5e035b02d138a9704f9513c0865f2733b7c09294ee504c155c283f4895559b6ac39828eac98ad393a642330589e8849040f55ce44f8f2197529d0b0ed57ccdda41f1971e153ec28ac5b4eba968741db374104d65ee234580a83bea1c0cdb67b8bc207057486eb1d90e21ba0cd4f5e9fd834821fafc1517c5d1fceb50ba6f6b102a9b4edac46f2359aec795a4e2458f51114a41289634b3b1cf250e3e38f3689f951278dfa7202a7dfe311cc098fd4a8d02c8f8a74e4a5010b18ee2e60578d5e9f1c094433a73f26e6546e20a574fc261baaa79e9910ab86ed607786a1cc88e7de51ff928d434e26eaef1437f7068c743f26d7c0eea6791e869b101fee8ab41b50af6174c5e6b731a1719f31ee3e6529efef49f31665baedc9382e9665278a84467d479f139fc7a8ef66fef9bd2fd17f7779ee315d458f691a290fa7c2179de8bb91a78458c5290d4aa45b163254006800ba2fce7479511f744fd7de96495c39be93413d8b0b187fe092537e1a7646a66a125b33333f6ecd10085e23ad168b24ee7be69d01ea021a39401e4bd41d818499e7174dd9b85542076c78cb89eeec1c190301b4709dbc963d47926e31bb0235ba6a7029d49458150f6491ac9c973b8a2c893258f907baf4bcb7c39f12b900ba2b2382cd5dd84314ee504ade835ad9a1cb13a7f5928a483ebc9415429810fd99893f2f8f83970b8b47143d617e6f9853e4d86ff378be664218f1c32531143e209f171590dd48216fec879a6b9cbf04432bf4f1a3734b69b6a9f1a358a259a0f9082cfb6c1f3d9d2d9e4522ad651ccce565f06b30c1c0b27252270c2f6608cf4f3288a7e7d4b174e646de05341f7db62b00b5ccb295f058d34b87201148828e9b3f7e08f60e100f810be27eb7f4c471cda7621106fe78bc69ec2bd27acabd55dc094b8626913b7d24d9b60939754700f32574a733a195f8b0220d56f6797de0bcd7b80d561896b816586593409f76e85a7a1035f821dee32a02fdbc26bc4cca375bed418b9d678ac589249a1a5a5b24447ee9b42e33f817066caf3d4e17d0347f6acf0cbf426d4df49413b3d12350edec2681ab9cfecd0825ccfb2649a57391d3f153050dfb4350d60e5e464229ddd6e49ece95557b8ef48c18cbffbe9fc8d7700f611a4b33a2a254afcec638c485e36daf0364da7d4302e488db7b6c41297571048cfea5452e324abb9f9e1043e625fd0853b7e03063d1c3a43aa1ee62d45d890b5e4d10640e775cff6852b6d1acd4a503b3ece3b319cbcf33ff9fdf17b8f852d748db1e05af80507f5d0e1bc44444b155d7da20f7f0b4d6d83368c3bb9e1321b39472a8677ea1d3aca43b453d35edca37b7536d19c26b764958b3c7c30f3211d7b7bb7f6a6d7fd7bf2dda6e7d7b1e533556863549bbe1394a3828596f25029b7e30495e1235f084e5edd133bc29fce4f1e5e514eb1d1cb19fd8dfbb0d130fbec4e288f23dae86311ffd6f4afbaacc2ffe1cc8811a455ba6f5659f82515b56c6ac84277bff5bef98fefc74e002e4a11866a417a429541f8a62df4108e4730d3045f92984bcf1ab2f7d03f8bb1767e91791530cd8eec412919e1f2e341e66a1588a8f485f7aa005787af946b9cb10f6685420b7e1663f66374fddc5e70720507ee2134f3b02df042fcf6db4a5bdd74cc5010793634816fe447cc68e076b225cc1ca872929ef246ce356dc8d8964ff6d7119d071eccb6dc37f75b932c44cdc30723b8357a2761c6de6ab2713e6f6a782538cb731b07950d3f459760a00cc0af406d6848014746b02653636f479d952b46fdeff976e1d159ba46ae7363d5b0042d3905a0bda12aaa6eaae1a5a0d55d4c1930aa1c004cd610866853a247239366aa20f8968ea9ca3d5d6d7321a5d0f2c".into()
        ];
        validate_headers(headers, false, false).unwrap()
    }

    #[test]
    fn test_block_headers_difficulty_check() {
        // BTC: 724609, 724610, 724611
        let headers: Vec<BlockHeader> = vec!["00200020eab6fa183da8f9e4c761b31a67a76fa6a7658eb84c760200000000000000000063cd9585d434ec0db25894ec4b1f03735f10e31709c4395ea67c50c8378f134b972f166278100a17bfd87203".into(), 
                                             "0000402045c698413fbe8b5bf10635658d2a1cec72062798e51200000000000000000000869617420a4c95b1d3d6d012419d2b6c199cff9b68dd9a790892a4da8466fb056033166278100a1743ac4d5b".into(), 
                                             "0400e02019d733c1fd76a1fa5950de7bee9d80f107276b93a67204000000000000000000a0d1dee718f5f732c041800e9aa2c25e92be3f6de28278545388db8a6ae27df64c37166278100a170a970c19".into()];
        validate_headers(headers, true, true).unwrap()
    }
}

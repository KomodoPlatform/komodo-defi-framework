use bitcoin_spv::btcspv::verify_hash256_merkle;
use chain::{BlockHeader, RawBlockHeader};
use primitives::hash::H256;
use primitives::U256;
use types::SPVError;

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
    let mut vec: Vec<u8> = vec![];
    for merkle_node in intermediate_nodes {
        vec.append(&mut merkle_node.as_slice().to_vec());
    }
    let nodes = bitcoin_spv::types::MerkleArray::new(vec.as_slice())?;
    if !verify_hash256_merkle(txid.take().into(), merkle_root.take().into(), &nodes, index) {
        return Err(SPVError::BadMerkleProof);
    }
    Ok(())
}

fn validate_header_prev_hash(actual: &H256, to_compare_with: &H256) -> bool { actual == to_compare_with }

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
            target = raw_header.target();
        }
        if !constant_difficulty && raw_header.target() != target {
            return Err(SPVError::UnexpectedDifficultyChange);
        }
        if i != 0 && !validate_header_prev_hash(&raw_header.parent(), &previous_hash) {
            return Err(SPVError::InvalidChain);
        }
        if difficulty_check {
            todo!()
        }
        previous_hash = raw_header.digest();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(result.is_err(), false);
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
        assert_eq!(result.is_err(), false);
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
        assert_eq!(result.is_err(), false);
    }
}

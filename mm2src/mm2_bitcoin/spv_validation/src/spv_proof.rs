use crate::helpers_validation::{merkle_prove, validate_vin, validate_vout, SPVError};
use chain::BlockHeader;
use primitives::hash::H256;

pub const TRY_SPV_PROOF_INTERVAL: u64 = 10;

#[derive(PartialEq, Clone)]
pub struct SPVProof {
    /// The tx id
    pub tx_id: H256,
    /// The vin serialized
    pub vin: Vec<u8>,
    /// The vout serialized
    pub vout: Vec<u8>,
    /// The transaction index in the merkle tree
    pub index: u64,
    /// The intermediate nodes (digests between leaf and root)
    pub intermediate_nodes: Vec<H256>,
}

/// Checks validity of an entire SPV Proof against a previously validated UTXO header retrieved from storage
///
/// # Arguments
///
/// * `self` - The SPV Proof
///
/// # Errors
///
/// * Errors if any of the SPV Proof elements are invalid.
///
/// # Notes
/// Re-write with our own types based on `bitcoin_spv::std_types::SPVProof::validate`
impl SPVProof {
    pub fn validate(&self, validated_header: &BlockHeader) -> Result<(), SPVError> {
        if !validate_vin(self.vin.as_slice()) {
            return Err(SPVError::InvalidVin);
        }
        if !validate_vout(self.vout.as_slice()) {
            return Err(SPVError::InvalidVout);
        }
        merkle_prove(
            self.tx_id,
            validated_header.merkle_root_hash,
            self.intermediate_nodes.clone(),
            self.index,
        )
    }
}

#[cfg(test)]
mod spv_proof_tests {
    use crate::spv_proof::SPVProof;
    use chain::{BlockHeader, Transaction};
    use hex::FromHex;
    use primitives::hash::H256;
    use serialization::{deserialize, serialize_list};

    #[test]
    fn test_validate() {
        // https://live.blockcypher.com/btc-testnet/block/000000000000004d36632fda8180ff16855d606e5515aab0750d9d4fe55fe7d6/
        let header_hex = "0000602002bf77bbb098f90f149430c314e71ef4e2671ea5e04a2503e0000000000000000406ffb54f2925360aae81bd3199f456928bbe6ae83a877902da9d9ffb08215da0ba3161ffff001a545a850b";
        let header_bytes: Vec<u8> = header_hex.from_hex().unwrap();
        let validated_header: BlockHeader = deserialize(header_bytes.as_slice()).unwrap();
        //https://live.blockcypher.com/btc-testnet/tx/eefbafa4006e77099db059eebe14687965813283e5754d317431d9984554735d/
        let tx: Transaction = "0200000000010146c398e70cceaf9d8f734e603bc53e4c4c0605ab46cb1b5807a62c90f5aed50d0100000000feffffff023c0fc10c010000001600145033f65b590f2065fe55414213f1d25ab20b6c4f487d1700000000001600144b812d5ef41fc433654d186463d41b458821ff740247304402202438dc18801919baa64eb18f7e925ab6acdedc3751ea58ea164a26723b79fd39022060b46c1d277714c640cdc8512c36c862ffc646e7ff62438ef5cc847a5990bbf801210247b49d9e6b0089a1663668829e573c629c936eb430c043af9634aa57cf97a33cbee81f00".into();
        let intermediate_nodes: Vec<H256> = vec![
            "434d6b93388ab077aa12d6257253cc036fd6122e9e88465a86f4fd682fc6e006".into(),
            "bd9af28e56cf6731e78ee1503a65d9cc9b15c148daa474e71e085176f48996ac".into(),
            "605f6f83423ef3b86623927ef2d9dcb0f8d9e40a8132217c2fa0910b84488ec7".into(),
            "10b7ef06ef0756823dbf39dea717be397e7ccb49bbefc5cfc45e6f9d58793baf".into(),
            "19183ceae11796a9b1d0893e0561870bbce4d060c9547b1e91ad8b34eb3d5001".into(),
            "1b16723739522955422b4286b4d8620d2a704b6997e6bbd809d151b8d8d64611".into(),
            "6f8496469b19dd35871684332dfd3fc0205d83d2c58c44ebdae068542bc951f6".into(),
            "e0d2733bd7bce4e5690b71bc8f7cedb1edbc49a5ff85c3678ecdec894ea1c023".into(),
        ];
        let intermediate_nodes = intermediate_nodes.into_iter().map(|hash| hash.reversed()).collect();
        let spv_proof = SPVProof {
            tx_id: tx.hash(),
            vin: serialize_list(&tx.inputs).take(),
            vout: serialize_list(&tx.outputs).take(),
            index: 1,
            intermediate_nodes,
        };
        spv_proof.validate(&validated_header).unwrap()
    }
}

use crate::helpers_validation::{merkle_prove, validate_vin, validate_vout, SPVError};
use chain::BlockHeader;
use chain::RawBlockHeader;
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
    /// The confirming UTXO header
    pub confirming_header: BlockHeader,
    /// The Raw confirming UTXO Header
    pub raw_header: RawBlockHeader,
    /// The intermediate nodes (digests between leaf and root)
    pub intermediate_nodes: Vec<H256>,
}

/// Checks validity of an entire SPV Proof
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
    pub fn validate_block_header(&self) -> Result<(), SPVError> {
        if self.confirming_header.hash() != self.raw_header.digest() {
            return Err(SPVError::WrongDigest);
        }
        if self.confirming_header.merkle_root_hash != self.raw_header.extract_merkle_root() {
            return Err(SPVError::WrongMerkleRoot);
        }
        if self.confirming_header.previous_header_hash != self.raw_header.parent() {
            return Err(SPVError::WrongPrevHash);
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), SPVError> {
        if !validate_vin(self.vin.as_slice()) {
            return Err(SPVError::InvalidVin);
        }
        if !validate_vout(self.vout.as_slice()) {
            return Err(SPVError::InvalidVout);
        }
        self.validate_block_header()?;
        merkle_prove(
            self.tx_id,
            self.confirming_header.merkle_root_hash,
            self.intermediate_nodes.clone(),
            self.index,
        )
    }
}

#[cfg(test)]
mod spv_proof_tests {
    use crate::spv_proof::SPVProof;
    use chain::BlockHeader;
    use chain::RawBlockHeader;
    use hex::FromHex;
    use serialization::deserialize;

    #[test]
    fn test_block_header() {
        let header_hex = "040000008e4e7283b71dd1572d220935db0a1654d1042e92378579f8abab67b143f93a02fa026610d2634b72ff729b9ea7850c0d2c25eeaf7a82878ca42a8e9912028863a2d8a734eb73a4dc734072dbfd12406f1e7121bfe0e3d6c10922495c44e5cc1c91185d5ee519011d0400b9caaf41d4b63a6ab55bb4e6925d46fc3adea7be37b713d3a615e7cf0000fd40050001a80fa65b9a46fdb1506a7a4d26f43e7995d69902489b9f6c4599c88f9c169605cc135258953da0d6299ada4ff81a76ad63c943261078d5dd1918f91cea68b65b7fc362e9df49ba57c2ea5c6dba91591c85eb0d59a1905ac66e2295b7a291a1695301489a3cc7310fd45f2b94e3b8d94f3051e9bbaada1e0641fcec6e0d6230e76753aa9574a3f3e28eaa085959beffd3231dbe1aeea3955328f3a973650a38e31632a4ffc7ec007a3345124c0b99114e2444b3ef0ada75adbd077b247bbf3229adcffbe95bc62daac88f96317d5768540b5db636f8c39a8529a736465ed830ab2c1bbddf523587abe14397a6f1835d248092c4b5b691a955572607093177a5911e317739187b41f4aa662aa6bca0401f1a0a77915ebb6947db686cff549c5f4e7b9dd93123b00a1ae8d411cfb13fa7674de21cbee8e9fc74e12aa6753b261eab3d9256c7c32cc9b16219dad73c61014e7d88d74d5e218f12e11bc47557347ff49a9ab4490647418d2a5c2da1df24d16dfb611173608fe4b10a357b0fa7a1918b9f2d7836c84bf05f384e1e678b2fdd47af0d8e66e739fe45209ede151a180aba1188058a0db093e30bc9851980cf6fbfa5adb612d1146905da662c3347d7e7e569a1041641049d951ab867bc0c6a3863c7667d43f596a849434958cee2b63dc8fa11bd0f38aa96df86ed66461993f64736345313053508c4e939506c08a766f5b6ed0950759f3901bbc4db3dc97e05bf20b9dda4ff242083db304a4e487ac2101b823998371542354e5d534b5b6ae6420cc19b11512108b61208f4d9a5a97263d2c060da893544dea6251bcadc682d2238af35f2b1c2f65a73b89a4e194f9e1eef6f0e5948ef8d0d2862f48fd3356126b00c6a2d3770ecd0d1a78fa34974b454f270b23d461e357c9356c19496522b59ff9d5b4608c542ff89e558798324021704b2cfe9f6c1a70906c43c7a690f16615f198d29fa647d84ce8461fa570b33e3eada2ed7d77e1f280a0d2e9f03c2e1db535d922b1759a191b417595f3c15d8e8b7f810527ff942e18443a3860e67ccba356809ecedc31c5d8db59c7e039dae4b53d126679e8ffa20cc26e8b9d229c8f6ee434ad053f5f4f5a94e249a13afb995aad82b4d90890187e516e114b168fc7c7e291b9738ea578a7bab0ba31030b14ba90b772b577806ea2d17856b0cb9e74254ba582a9f2638ea7ed2ca23be898c6108ff8f466b443537ed9ec56b8771bfbf0f2f6e1092a28a7fd182f111e1dbdd155ea82c6cb72d5f9e6518cc667b8226b5f5c6646125fc851e97cf125f48949f988ed37c4283072fc03dd1da3e35161e17f44c0e22c76f708bb66405737ef24176e291b4fc2eadab876115dc62d48e053a85f0ad132ef07ad5175b036fe39e1ad14fcdcdc6ac5b3daabe05161a72a50545dd812e0f9af133d061b726f491e904d89ee57811ef58d3bda151f577aed381963a30d91fb98dc49413300d132a7021a5e834e266b4ac982d76e00f43f5336b8e8028a0cacfa11813b01e50f71236a73a4c0d0757c1832b0680ada56c80edf070f438ab2bc587542f926ff8d3644b8b8a56c78576f127dec7aed9cb3e1bc2442f978a9df1dc3056a63e653132d0f419213d3cb86e7b61720de1aa3af4b3757a58156970da27560c6629257158452b9d5e4283dc6fe7df42d2fda3352d5b62ce5a984d912777c3b01837df8968a4d494db1b663e0e68197dbf196f21ea11a77095263dec548e2010460840231329d83978885ee2423e8b327785970e27c6c6d436157fb5b56119b19239edbb730ebae013d82c35df4a6e70818a74d1ef7a2e87c090ff90e32939f58ed24e85b492b5750fd2cd14b9b8517136b76b1cc6ccc6f6f027f65f1967a0eb4f32cd6e5d5315";
        let header_bytes: Vec<u8> = header_hex.from_hex().unwrap();
        let header: BlockHeader = deserialize(header_bytes.as_slice()).unwrap();
        let spv_proof = SPVProof {
            tx_id: Default::default(),
            vin: vec![],
            vout: vec![],
            index: 0,
            confirming_header: header,
            raw_header: RawBlockHeader::new(header_bytes).unwrap(),
            intermediate_nodes: vec![],
        };
        spv_proof.validate_block_header().unwrap()
    }
}

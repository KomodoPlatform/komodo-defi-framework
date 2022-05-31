use chain::RawHeaderError;
use primitives::hash::H256;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SPVError {
    /// Overran a checked read on a slice
    ReadOverrun,
    /// Attempted to parse a CompactInt without enough bytes
    BadCompactInt,
    /// `extract_hash` could not identify the output type.
    MalformattedOutput,
    /// Unable to get target from block header
    UnableToGetTarget,
    /// Unable to get block header from network or storage
    UnableToGetHeader,
    /// Header not exactly 80 bytes.
    WrongLengthHeader,
    /// Header chain changed difficulties unexpectedly
    UnexpectedDifficultyChange,
    /// Header does not meet its own difficulty target.
    InsufficientWork,
    /// Header in chain does not correctly reference parent header.
    InvalidChain,
    /// When validating a `BitcoinHeader`, the `hash` field is not the digest
    /// of the raw header.
    WrongDigest,
    /// When validating a `BitcoinHeader`, the `merkle_root` field does not
    /// match the root found in the raw header.
    WrongMerkleRoot,
    /// When validating a `BitcoinHeader`, the `prevhash` field does not
    /// match the parent hash found in the raw header.
    WrongPrevHash,
    /// A `vin` (transaction input vector) is malformatted.
    InvalidVin,
    /// A `vout` (transaction output vector) is malformatted or empty.
    InvalidVout,
    /// merkle proof connecting the `tx_id_le` to the `confirming_header`.
    BadMerkleProof,
    /// Unable to get merkle tree from network or storage
    UnableToGetMerkle,
    /// Unable to retrieve block height / block height is zero.
    InvalidHeight,
    /// Raises during validation loop
    Timeout,
}

impl From<RawHeaderError> for SPVError {
    fn from(e: RawHeaderError) -> Self {
        match e {
            RawHeaderError::WrongLengthHeader { .. } => SPVError::WrongLengthHeader,
        }
    }
}

/// A slice of `H256`s for use in a merkle array
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleArray<'a>(&'a [u8]);

impl<'a> MerkleArray<'a> {
    /// Return a new merkle array from a slice
    pub fn new(slice: &'a [u8]) -> Result<MerkleArray<'a>, SPVError> {
        if slice.len() % 32 == 0 {
            Ok(Self(slice))
        } else {
            Err(SPVError::BadMerkleProof)
        }
    }
}

impl MerkleArray<'_> {
    /// The length of the underlying slice
    pub fn len(&self) -> usize { self.0.len() / 32 }

    /// Whether the underlying slice is empty
    pub fn is_empty(&self) -> bool { self.0.is_empty() }

    /// Index into the merkle array
    pub fn index(&self, index: usize) -> H256 {
        let mut digest = H256::default();
        digest.as_mut().copy_from_slice(&self.0[index * 32..(index + 1) * 32]);
        digest
    }
}

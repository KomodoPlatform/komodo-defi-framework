use chain::RawHeaderError;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SPVError {
    /// Overran a checked read on a slice
    ReadOverrun,
    /// Attempted to parse a CompactInt without enough bytes
    BadCompactInt,
    /// Called `extract_op_return_data` on an output without an op_return.
    MalformattedOpReturnOutput,
    /// `extract_hash` identified a SH output prefix without a SH postfix.
    MalformattedP2SHOutput,
    /// `extract_hash` identified a PKH output prefix without a PKH postfix.
    MalformattedP2PKHOutput,
    /// `extract_hash` identified a Witness output with a bad length tag.
    MalformattedWitnessOutput,
    /// `extract_hash` could not identify the output type.
    MalformattedOutput,
    /// Unable to get target from block header
    UnableToGetTarget,
    /// Unable to get block header from network or storage
    UnableToGetHeader,
    /// Unable to deserialize raw block header from electrum to concrete type
    MalformattedHeader,
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
    /// When validating an `SPVProof`, the `tx_id` field is not the digest
    /// of the `version`, `vin`, `vout`, and `locktime`.
    WrongTxID,
    /// When validating an `SPVProof`, the `intermediate_nodes` is not a valid
    /// merkle proof connecting the `tx_id_le` to the `confirming_header`.
    BadMerkleProof,
    /// Unable to get merkle tree from network or storage
    UnableToGetMerkle,
    /// TxOut's reported length does not match passed-in byte slice's length
    OutputLengthMismatch,
    /// Unable to retrieve block height / block height is zero.
    InvalidHeight,
    /// Block Header Not Verified / Verification failed
    BlockHeaderNotVerified,
    /// Any other error
    UnknownError,
}

impl From<bitcoin_spv::types::SPVError> for SPVError {
    fn from(e: bitcoin_spv::types::SPVError) -> Self {
        match e {
            bitcoin_spv::types::SPVError::ReadOverrun => SPVError::ReadOverrun,
            bitcoin_spv::types::SPVError::BadCompactInt => SPVError::BadCompactInt,
            bitcoin_spv::types::SPVError::MalformattedOpReturnOutput => SPVError::MalformattedOpReturnOutput,
            bitcoin_spv::types::SPVError::MalformattedP2SHOutput => SPVError::MalformattedP2SHOutput,
            bitcoin_spv::types::SPVError::MalformattedP2PKHOutput => SPVError::MalformattedP2PKHOutput,
            bitcoin_spv::types::SPVError::MalformattedWitnessOutput => SPVError::MalformattedWitnessOutput,
            bitcoin_spv::types::SPVError::MalformattedOutput => SPVError::MalformattedOutput,
            bitcoin_spv::types::SPVError::WrongLengthHeader => SPVError::WrongLengthHeader,
            bitcoin_spv::types::SPVError::UnexpectedDifficultyChange => SPVError::UnexpectedDifficultyChange,
            bitcoin_spv::types::SPVError::InsufficientWork => SPVError::InsufficientWork,
            bitcoin_spv::types::SPVError::InvalidChain => SPVError::InvalidChain,
            bitcoin_spv::types::SPVError::WrongDigest => SPVError::WrongDigest,
            bitcoin_spv::types::SPVError::WrongMerkleRoot => SPVError::WrongMerkleRoot,
            bitcoin_spv::types::SPVError::WrongPrevHash => SPVError::WrongPrevHash,
            bitcoin_spv::types::SPVError::InvalidVin => SPVError::InvalidVin,
            bitcoin_spv::types::SPVError::InvalidVout => SPVError::InvalidVout,
            bitcoin_spv::types::SPVError::WrongTxID => SPVError::WrongTxID,
            bitcoin_spv::types::SPVError::BadMerkleProof => SPVError::BadMerkleProof,
            bitcoin_spv::types::SPVError::OutputLengthMismatch => SPVError::OutputLengthMismatch,
            bitcoin_spv::types::SPVError::UnknownError => SPVError::UnknownError,
        }
    }
}

impl From<RawHeaderError> for SPVError {
    fn from(e: RawHeaderError) -> Self {
        match e {
            RawHeaderError::WrongLengthHeader { .. } => SPVError::WrongLengthHeader,
        }
    }
}

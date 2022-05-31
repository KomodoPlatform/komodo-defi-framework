use chain::RawHeaderError;
use primitives::hash::H256;
use serialization::{parse_compact_int, CompactInteger};

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
    /// Raises during validation loop
    Timeout,
    /// Any other error
    UnknownError,
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

macro_rules! impl_view_type {
    (
        $(#[$outer:meta])*
        $name:ident
    ) => {
        $(#[$outer])*
        #[derive(Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub struct $name<'a>(pub(crate) &'a [u8]);

        impl $name<'_> {
            /// The length of the underlying slice
            pub fn len(&self) -> usize {
                self.0.len()
            }

            /// Whether the underlying slice is empty
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }

            /// The last item in the underlying slice, if any
            pub fn last(&self) -> Option<&u8> {
                self.0.last()
            }
        }

        impl<'a> AsRef<[u8]> for $name<'a> {
            fn as_ref(&self) -> &[u8] {
                self.0
            }
        }

        impl<I: core::slice::SliceIndex<[u8]>> core::ops::Index<I> for $name<'_> {
            type Output = I::Output;

            fn index(&self, index: I) -> &Self::Output {
                self.as_ref().index(index)
            }
        }

        impl PartialEq<[u8]> for $name<'_> {
            fn eq(&self, other: &[u8]) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&[u8]> for $name<'_> {
            fn eq(&self, other: &&[u8]) -> bool {
                &self.0 == other
            }
        }

        // For convenience while testing
        #[cfg(test)]
        impl<'a> From<&'a [u8]> for $name<'a> {
            fn from(slice: &'a [u8]) -> Self {
                Self(slice)
            }
        }
    }
}

impl_view_type!(
    /// A ScriptSig
    ScriptSig
);

/// Determines the length of a scriptSig in an input.
/// Will return 0 if passed a witness input.
///
/// # Arguments
///
/// * `tx_in` - The LEGACY input
pub fn extract_script_sig_len(tx_in: &TxIn) -> Result<CompactInteger, SPVError> {
    if tx_in.len() < 37 {
        return Err(SPVError::ReadOverrun);
    }
    parse_compact_int(&tx_in[36..]).map_err(|_| SPVError::BadCompactInt)
}

/// Extracts the CompactInt-prepended scriptSig from the input in a tx.
/// Will return `vec![0]` if passed a witness input.
///
/// # Arguments
///
/// * `tx_in` - The LEGACY input
pub fn extract_script_sig<'a>(tx_in: &'a TxIn) -> Result<ScriptSig<'a>, SPVError> {
    let script_sig_len = extract_script_sig_len(tx_in)?;
    let length = script_sig_len.serialized_length() + script_sig_len.as_usize();
    Ok(ScriptSig(&tx_in[36..36 + length]))
}

impl_view_type!(
    /// A TxIn
    TxIn
);

impl TxIn<'_> {
    /// Extract the script sig from the TxIn
    pub fn script_sig(&self) -> ScriptSig { extract_script_sig(self).expect("Not malformed") }
}

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

/// A bitoin double-sha256 digest
#[derive(Copy, Clone, PartialEq, Eq, Default, Hash)]
pub struct Hash256Digest([u8; 32]);

impl core::fmt::Debug for Hash256Digest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, "Hash256Digest: {:x?}", self.0) }
}

impl From<[u8; 32]> for Hash256Digest {
    fn from(buf: [u8; 32]) -> Self { Self(buf) }
}

impl AsRef<[u8; 32]> for Hash256Digest {
    fn as_ref(&self) -> &[u8; 32] { &self.0 }
}

impl AsMut<[u8; 32]> for Hash256Digest {
    fn as_mut(&mut self) -> &mut [u8; 32] { &mut self.0 }
}

/// A slice of `Hash256Digest`s for use in a merkle array
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
    pub fn index(&self, index: usize) -> Hash256Digest {
        let mut digest = Hash256Digest::default();
        digest.as_mut().copy_from_slice(&self.0[index * 32..(index + 1) * 32]);
        digest
    }
}

macro_rules! compact_int_conv {
    ($target:ty) => {
        impl From<$target> for CompactInt {
            fn from(number: $target) -> CompactInt { Self(number as u64) }
        }

        impl PartialEq<$target> for CompactInt {
            fn eq(&self, other: &$target) -> bool { self.0 == *other as u64 }
        }
    };
}

/// A Bitcoin-formatted `CompactInt`
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct CompactInt(u64);

compact_int_conv!(u8);
compact_int_conv!(u16);
compact_int_conv!(u32);
compact_int_conv!(u64);
compact_int_conv!(usize);

impl AsRef<u64> for CompactInt {
    fn as_ref(&self) -> &u64 { &self.0 }
}

impl CompactInt {
    /// Parse a compact int from a byte slice
    pub fn parse<T: AsRef<[u8]> + ?Sized>(t: &T) -> Result<CompactInt, SPVError> { parse_compact_int(t) }

    /// Return the underlying u64
    pub fn number(&self) -> u64 { self.0 }

    /// The underlying number as a usize
    pub fn as_usize(&self) -> usize { self.0 as usize }

    /// Determine the length of the compact int when serialized
    pub fn serialized_length(&self) -> usize {
        match self.0 {
            0..=0xfc => 1,
            0xfd..=0xffff => 3,
            0x10000..=0xffff_ffff => 5,
            _ => 9,
        }
    }

    /// Determines the length of a CompactInt in bytes.
    /// A CompactInt of > 1 byte is prefixed with a flag indicating its length.
    ///
    /// # Arguments
    ///
    /// * `flag` - The first byte of a compact_int
    pub fn data_length(flag: u8) -> u8 {
        let length: u8 = match flag {
            0xfd => 2,
            0xfe => 4,
            0xff => 8,
            _ => 0,
        };
        length
    }
}

/// Parse a CompactInt into its data length and the number it represents
/// Useful for Parsing Vins and Vouts. Returns `BadCompactInt` if insufficient bytes.
///
/// # Arguments
///
/// * `buf` - A byte-string starting with a CompactInt
///
/// # Returns
///
/// * (length, number) - the length of the data in bytes, and the number it represents
pub fn parse_compact_int<T: AsRef<[u8]> + ?Sized>(buf: &T) -> Result<CompactInt, SPVError> {
    let buf = buf.as_ref();
    let length = CompactInt::data_length(buf[0]) as usize;

    if length == 0 {
        return Ok(buf[0].into());
    }
    if buf.len() < 1 + length {
        return Err(SPVError::BadCompactInt);
    }

    let mut num_bytes = [0u8; 8];
    num_bytes[..length].copy_from_slice(&buf[1..=length]);

    Ok(u64::from_le_bytes(num_bytes).into())
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

/// Extracts the outpoint tx id from an input,
/// 32 byte tx id.
///
/// # Arguments
///
/// * `outpoint` - The outpoint extracted from the input
pub fn extract_input_tx_id_le(outpoint: &Outpoint) -> Hash256Digest {
    let mut txid = Hash256Digest::default();
    txid.as_mut().copy_from_slice(&outpoint[0..32]);
    txid
}

/// Extracts the LE tx input index from the input in a tx,
/// 4 byte tx index.
///
/// # Arguments
///
/// * `outpoint` - The outpoint extracted from the input
pub fn extract_tx_index_le(outpoint: &Outpoint) -> [u8; 4] {
    let mut idx = [0u8; 4];
    idx.copy_from_slice(&outpoint[32..36]);
    idx
}

/// Extracts the LE tx input index from the input in a tx,
/// 4 byte tx index.
///
/// # Arguments
///
/// * `outpoint` - The outpoint extracted from the input
pub fn extract_tx_index(outpoint: &Outpoint) -> u32 {
    let mut arr: [u8; 4] = [0u8; 4];
    let b = extract_tx_index_le(outpoint);
    arr.copy_from_slice(&b[..]);
    u32::from_le_bytes(arr)
}

impl_view_type!(
    /// A Outpoint
    Outpoint
);

impl Outpoint<'_> {
    /// Extract the LE txid from the outpoint
    pub fn txid_le(&self) -> Hash256Digest { extract_input_tx_id_le(self) }

    /// Extract the outpoint's index in the prevout tx's vout
    pub fn vout_index(&self) -> u32 { extract_tx_index(self) }
}

/// Extracts the outpoint from the input in a tx,
/// 32 byte tx id with 4 byte index.
///
/// # Arguments
///
/// * `tx_in` - The input
pub fn extract_outpoint<'a>(tx_in: &'a TxIn<'a>) -> Outpoint<'a> { Outpoint(&tx_in[0..36]) }

/// Extracts the LE sequence bytes from an input.
/// Sequence is used for relative time locks.
///
/// # Arguments
///
/// * `tx_in` - The LEGACY input
pub fn extract_sequence_le(tx_in: &TxIn) -> Result<[u8; 4], SPVError> {
    let script_sig_len = extract_script_sig_len(tx_in)?;
    let offset: usize = 36 + script_sig_len.serialized_length() + script_sig_len.as_usize();

    let mut sequence = [0u8; 4];
    sequence.copy_from_slice(&tx_in[offset..offset + 4]);
    Ok(sequence)
}

/// Extracts the sequence from the input.
/// Sequence is a 4-byte little-endian number.
///
/// # Arguments
///
/// * `tx_in` - The LEGACY input
pub fn extract_sequence(tx_in: &TxIn) -> Result<u32, SPVError> {
    let mut arr: [u8; 4] = [0u8; 4];
    let b = extract_sequence_le(tx_in)?;
    arr.copy_from_slice(&b[..]);
    Ok(u32::from_le_bytes(arr))
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
fn extract_script_sig_len(tx_in: &TxIn) -> Result<CompactInt, SPVError> {
    if tx_in.len() < 37 {
        return Err(SPVError::ReadOverrun);
    }
    parse_compact_int(&tx_in[36..])
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
    /// Extract the outpoint from the TxIn
    pub fn outpoint(&self) -> Outpoint { extract_outpoint(self) }

    /// Extract the sequence number from the TxIn
    pub fn sequence(&self) -> u32 { extract_sequence(self).expect("Not malformed") }

    /// Extract the script sig from the TxIn
    pub fn script_sig(&self) -> ScriptSig { extract_script_sig(self).expect("Not malformed") }
}

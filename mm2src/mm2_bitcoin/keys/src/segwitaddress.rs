use std::fmt;
use std::str::FromStr;

use bech32;
use AddressHash;

/// Address error.
#[derive(Debug, PartialEq)]
pub enum Error {
    /// Invalid address format
    InvalidSegwitAddressFormat,
    /// Bech32 encoding error
    Bech32(bech32::Error),
    /// The bech32 payload was empty
    EmptyBech32Payload,
    /// Script version must be 0 to 16 inclusive
    InvalidWitnessVersion(u8),
    /// The witness program must be between 2 and 40 bytes in length.
    InvalidWitnessProgramLength(usize),
    /// A v0 witness program must be either of length 20 or 32.
    InvalidSegwitV0ProgramLength(usize),
    /// An uncompressed pubkey was used where it is not allowed.
    UncompressedPubkey,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::InvalidSegwitAddressFormat => write!(f, "Invalid segwit address format"),
            Error::Bech32(ref e) => write!(f, "bech32: {}", e),
            Error::EmptyBech32Payload => write!(f, "the bech32 payload was empty"),
            Error::InvalidWitnessVersion(v) => write!(f, "invalid witness script version: {}", v),
            Error::InvalidWitnessProgramLength(l) => write!(
                f,
                "the witness program must be between 2 and 40 bytes in length: length={}",
                l,
            ),
            Error::InvalidSegwitV0ProgramLength(l) => write!(
                f,
                "a v0 witness program must be either of length 20 or 32 bytes: length={}",
                l,
            ),
            Error::UncompressedPubkey => write!(f, "an uncompressed pubkey was used where it is not allowed",),
        }
    }
}

#[doc(hidden)]
impl From<bech32::Error> for Error {
    fn from(e: bech32::Error) -> Error { Error::Bech32(e) }
}

/// The different types of segwit addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AddressType {
    P2wpkh,
    /// pay-to-witness-script-hash
    P2wsh,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// A Bitcoin segwit address
pub struct SegwitAddress {
    /// The human-readable part
    pub hrp: String,
    /// The witness program version
    version: bech32::u5,
    /// The witness program
    pub program: Vec<u8>,
}

impl SegwitAddress {
    pub fn new(hash: &AddressHash, hrp: String) -> SegwitAddress {
        SegwitAddress {
            hrp,
            version: bech32::u5::try_from_u8(0).expect("0<32"),
            program: hash.to_vec(),
        }
    }

    /// Get the address type of the address.
    /// None if unknown or non-standard.
    pub fn address_type(&self) -> Option<AddressType> {
        // BIP-141 p2wpkh or p2wsh addresses.
        match self.version.to_u8() {
            0 => match self.program.len() {
                20 => Some(AddressType::P2wpkh),
                32 => Some(AddressType::P2wsh),
                _ => None,
            },
            _ => None,
        }
    }

    /// Check whether or not the address is following Bitcoin
    /// standardness rules.
    ///
    /// Segwit addresses with unassigned witness versions or non-standard
    /// program sizes are considered non-standard.
    pub fn is_standard(&self) -> bool { self.address_type().is_some() }
}

struct UpperWriter<W: fmt::Write>(W);

impl<W: fmt::Write> fmt::Write for UpperWriter<W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.0.write_char(c.to_ascii_uppercase())?;
        }
        Ok(())
    }
}

// Alternate formatting `{:#}` is used to return uppercase version of bech32 addresses which should
// be used in QR codes, see [Address::to_qr_uri]
impl fmt::Display for SegwitAddress {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let mut upper_writer;
        let writer = if fmt.alternate() {
            upper_writer = UpperWriter(fmt);
            &mut upper_writer as &mut dyn fmt::Write
        } else {
            fmt as &mut dyn fmt::Write
        };
        let mut bech32_writer = bech32::Bech32Writer::new(self.hrp.as_str(), bech32::Variant::Bech32, writer)?;
        bech32::WriteBase32::write_u5(&mut bech32_writer, self.version)?;
        bech32::ToBase32::write_base32(&self.program, &mut bech32_writer)
    }
}

/// Extract the bech32 prefix.
/// Returns the same slice when no prefix is found.
fn find_bech32_prefix(bech32: &str) -> Option<&str> {
    // Split at the last occurrence of the separator character '1'.
    match bech32.rfind('1') {
        None => None,
        Some(sep) => Some(bech32.split_at(sep).0),
    }
}

impl FromStr for SegwitAddress {
    type Err = Error;

    fn from_str(s: &str) -> Result<SegwitAddress, Error> {
        // try bech32
        let hrp = match find_bech32_prefix(s) {
            // Todo: upper or lowercase is allowed but NOT mixed case
            Some(hrp) => hrp.to_string(),
            None => return Err(Error::InvalidSegwitAddressFormat),
        };
        // decode as bech32, should use Variant if Bech32m is used alongside Bech32
        // The improved Bech32m variant described in [BIP-0350](https://github.com/bitcoin/bips/blob/master/bip-0350.mediawiki)
        let (_, payload, _) = bech32::decode(s)?;
        if payload.is_empty() {
            return Err(Error::EmptyBech32Payload);
        }

        // Get the script version and program (converted from 5-bit to 8-bit)
        let (version, program): (bech32::u5, Vec<u8>) = {
            let (v, p5) = payload.split_at(1);
            (v[0], bech32::FromBase32::from_base32(p5)?)
        };

        // Generic segwit checks.
        if version.to_u8() > 16 {
            return Err(Error::InvalidWitnessVersion(version.to_u8()));
        }
        if program.len() < 2 || program.len() > 40 {
            return Err(Error::InvalidWitnessProgramLength(program.len()));
        }

        // Specific segwit v0 check.
        if version.to_u8() == 0 && (program.len() != 20 && program.len() != 32) {
            return Err(Error::InvalidSegwitV0ProgramLength(program.len()));
        }

        Ok(SegwitAddress { hrp, version, program })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Public;

    fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
        if s.len() % 2 == 0 {
            (0..s.len())
                .step_by(2)
                .map(|i| s.get(i..i + 2).and_then(|sub| u8::from_str_radix(sub, 16).ok()))
                .collect()
        } else {
            None
        }
    }

    #[test]
    fn test_p2wpkh_address() {
        // Bitcoin transaction: b3c8c2b6cfc335abbcb2c7823a8453f55d64b2b5125a9a61e8737230cdb8ce20
        let pk = "033bc8c83c52df5712229a2f72206d90192366c36428cb0c12b6af98324d97bfbc";
        let bytes = hex_to_bytes(pk).unwrap();
        let public_key = Public::from_slice(&bytes).unwrap();
        let hash = public_key.address_hash();
        let hrp = "bc";
        let addr = SegwitAddress::new(&hash, hrp.to_string());
        assert_eq!(&addr.to_string(), "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw");
        assert_eq!(addr.address_type(), Some(AddressType::P2wpkh));
    }
}

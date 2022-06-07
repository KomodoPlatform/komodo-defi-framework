/******************************************************************************
 * Copyright © 2022 Atomic Private Limited and its contributors               *
 *                                                                            *
 * See the CONTRIBUTOR-LICENSE-AGREEMENT, COPYING, LICENSE-COPYRIGHT-NOTICE   *
 * and DEVELOPER-CERTIFICATE-OF-ORIGIN files in the LEGAL directory in        *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * AtomicDEX software, including this file may be copied, modified, propagated*
 * or distributed except according to the terms contained in the              *
 * LICENSE-COPYRIGHT-NOTICE file.                                             *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  LP_utxos.c
//  marketmaker
//

use bitcrypto::{sha256, ChecksumType};
use derive_more::Display;
use keys::{Error as KeysError, KeyPair, Private};
use mm2_err_handle::prelude::*;
use primitives::hash::H256;
use rustc_hex::FromHexError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub type PrivKeyResult<T> = Result<T, MmError<PrivKeyError>>;

#[derive(Debug, Display, Serialize)]
pub enum PrivKeyError {
    #[display(fmt = "Provided WIF passphrase has invalid checksum!")]
    WifPassphraseInvalidChecksum,
    #[display(fmt = "Error parsing passphrase: {}", _0)]
    ErrorParsingPassphrase(String),
    #[display(fmt = "Invalid private key: {}", _0)]
    InvalidPrivKey(KeysError),
    #[display(fmt = "We only support compressed keys at the moment")]
    ExpectedCompressedKeys,
}

impl From<FromHexError> for PrivKeyError {
    fn from(e: FromHexError) -> Self { PrivKeyError::ErrorParsingPassphrase(e.to_string()) }
}

impl From<KeysError> for PrivKeyError {
    fn from(e: KeysError) -> Self { PrivKeyError::InvalidPrivKey(e) }
}

fn private_from_seed(seed: &str) -> PrivKeyResult<Private> {
    match seed.parse() {
        Ok(private) => return Ok(private),
        Err(e) => {
            if let KeysError::InvalidChecksum = e {
                return MmError::err(PrivKeyError::WifPassphraseInvalidChecksum);
            }
        }, // else ignore other errors, assume the passphrase is not WIF
    }

    match seed.strip_prefix("0x") {
        Some(stripped) => {
            let hash: H256 = stripped.parse()?;
            Ok(Private {
                prefix: 0,
                secret: hash,
                compressed: true,
                checksum_type: ChecksumType::DSHA256,
            })
        },
        None => {
            let hash = sha256(seed.as_bytes());

            Ok(Private {
                prefix: 0,
                secret: secp_privkey_from_hash(hash),
                compressed: true,
                checksum_type: ChecksumType::DSHA256,
            })
        },
    }
}

/// Mutates the arbitrary hash to become a valid secp256k1 private key
pub fn secp_privkey_from_hash(mut hash: H256) -> H256 {
    hash[0] &= 248;
    hash[31] &= 127;
    hash[31] |= 64;
    hash
}

pub fn key_pair_from_seed(seed: &str) -> PrivKeyResult<KeyPair> {
    let private = private_from_seed(seed)?;
    if !private.compressed {
        return MmError::err(PrivKeyError::ExpectedCompressedKeys);
    }
    let pair = KeyPair::from_private(private)?;
    // Just a sanity check. We rely on the public key being 33 bytes (aka compressed).
    assert_eq!(pair.public().len(), 33);
    Ok(pair)
}

pub fn key_pair_from_secret(secret: &[u8]) -> PrivKeyResult<KeyPair> {
    if secret.len() != 32 {
        return MmError::err(PrivKeyError::InvalidPrivKey(KeysError::InvalidPrivate));
    }

    let private = Private {
        prefix: 0,
        secret: secret.into(),
        compressed: true,
        checksum_type: Default::default(),
    };
    Ok(KeyPair::from_private(private)?)
}

#[derive(Clone, Copy, Debug)]
pub struct SerializableSecp256k1Keypair {
    inner: KeyPair,
}

impl PartialEq for SerializableSecp256k1Keypair {
    fn eq(&self, other: &Self) -> bool { self.inner.public() == other.inner.public() }
}

impl Eq for SerializableSecp256k1Keypair {}

impl SerializableSecp256k1Keypair {
    fn new(key: [u8; 32]) -> PrivKeyResult<Self> {
        Ok(SerializableSecp256k1Keypair {
            inner: key_pair_from_secret(&key)?,
        })
    }

    pub fn key_pair(&self) -> &KeyPair { &self.inner }

    pub fn public_slice(&self) -> &[u8] { self.inner.public_slice() }

    fn priv_key(&self) -> [u8; 32] { self.inner.private().secret.take() }

    pub fn random() -> Self {
        SerializableSecp256k1Keypair {
            inner: KeyPair::random_compressed(),
        }
    }

    pub fn into_inner(self) -> KeyPair { self.inner }
}

impl From<KeyPair> for SerializableSecp256k1Keypair {
    fn from(inner: KeyPair) -> Self { SerializableSecp256k1Keypair { inner } }
}

impl Serialize for SerializableSecp256k1Keypair {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.priv_key().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SerializableSecp256k1Keypair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let priv_key = <[u8; 32]>::deserialize(deserializer)?;
        SerializableSecp256k1Keypair::new(priv_key).map_err(serde::de::Error::custom)
    }
}

#[test]
fn serializable_secp256k1_keypair_test() {
    use serde_json::{self as json};

    let key_pair = KeyPair::random_compressed();
    let serializable = SerializableSecp256k1Keypair { inner: key_pair };
    let serialized = json::to_string(&serializable).unwrap();
    println!("{}", serialized);
    let deserialized = json::from_str(&serialized).unwrap();
    assert_eq!(serializable, deserialized);

    let invalid_privkey: [u8; 32] = [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xba, 0xae,
        0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
    ];
    let invalid_privkey_serialized = json::to_string(&invalid_privkey).unwrap();
    let err = json::from_str::<SerializableSecp256k1Keypair>(&invalid_privkey_serialized).unwrap_err();
    println!("{}", err);
}

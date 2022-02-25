/******************************************************************************
 * Copyright © 2014-2018 The SuperNET Developers.                             *
 *                                                                            *
 * See the AUTHORS, DEVELOPER-AGREEMENT and LICENSE files at                  *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * SuperNET software, including this file may be copied, modified, propagated *
 * or distributed except according to the terms contained in the LICENSE file *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  LP_utxos.c
//  marketmaker
//

use crate::mm_error::prelude::*;
use bitcrypto::{sha256, ChecksumType};
use derive_more::Display;
use keys::{Error as KeysError, KeyPair, Private};
use primitives::hash::H256;
use rustc_hex::FromHexError;

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
            let mut hash = sha256(seed.as_bytes());
            hash[0] &= 248;
            hash[31] &= 127;
            hash[31] |= 64;

            Ok(Private {
                prefix: 0,
                secret: hash,
                compressed: true,
                checksum_type: ChecksumType::DSHA256,
            })
        },
    }
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

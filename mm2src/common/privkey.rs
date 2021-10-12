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

use bitcrypto::{sha256, ChecksumType};
use keys::{Error as KeysError, KeyPair, Private};
use primitives::hash::H256;

fn private_from_seed(seed: &str) -> Result<Private, String> {
    match seed.parse() {
        Ok(private) => return Ok(private),
        Err(e) => {
            if let KeysError::InvalidChecksum = e {
                return ERR!("Provided WIF passphrase has invalid checksum!");
            }
        }, // else ignore other errors, assume the passphrase is not WIF
    }

    match seed.strip_prefix("0x") {
        Some(stripped) => {
            let hash: H256 = try_s!(stripped.parse());
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

pub fn key_pair_from_seed(seed: &str) -> Result<KeyPair, String> {
    let private = try_s!(private_from_seed(seed));
    if !private.compressed {
        return ERR!("We only support compressed keys at the moment");
    }
    let pair = try_s!(KeyPair::from_private(private));
    // Just a sanity check. We rely on the public key being 33 bytes (aka compressed).
    assert_eq!(pair.public().len(), 33);
    Ok(pair)
}

pub fn key_pair_from_secret(secret: [u8; 32]) -> Result<KeyPair, String> {
    let private = Private {
        prefix: 0,
        secret: secret.into(),
        compressed: true,
        checksum_type: Default::default(),
    };
    KeyPair::from_private(private).map_err(|e| format!("{}", e))
}

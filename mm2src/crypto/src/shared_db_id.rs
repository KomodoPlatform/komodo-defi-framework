use derive_more::Display;
use enum_from::EnumFromStringify;
use keys::{Error as KeysError, KeyPair};
use mm2_err_handle::prelude::*;
use primitives::hash::H160;

use crate::privkey::private_from_seed_hash;

/// This magic string is used to change the input mnemonic passphrase the way
/// so `sha256(mnemonic + SHARED_DB_MAGIC_SALT)`
const SHARED_DB_MAGIC_SALT: &str = "uVa*6pcnpc9ki+VBX.6_L.";

pub type SharedDbId = H160;

#[derive(Display, EnumFromStringify)]
pub enum SharedDbIdError {
    #[display(fmt = "Passphrase cannot be an empty string")]
    EmptyPassphrase,
    #[from_stringify("KeysError")]
    #[display(fmt = "Internal error: {_0}")]
    Internal(String),
}

pub fn shared_db_id_from_seed(passphrase: &str) -> MmResult<SharedDbId, SharedDbIdError> {
    let stripped_passphrase = passphrase.strip_prefix("0x").unwrap_or(passphrase);
    if stripped_passphrase.is_empty() {
        return MmError::err(SharedDbIdError::EmptyPassphrase);
    }

    let changed_passphrase = format!("{stripped_passphrase} {SHARED_DB_MAGIC_SALT}");
    let private = private_from_seed_hash(&changed_passphrase);
    let key_pair = KeyPair::from_private(private)?;
    Ok(key_pair.public().address_hash())
}

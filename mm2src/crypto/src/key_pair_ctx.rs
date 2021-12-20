use keys::{KeyPair, Private, Public as PublicKey};

pub struct KeyPairCtx {
    /// secp256k1 key pair derived from passphrase.
    /// cf. `key_pair_from_seed`.
    pub(crate) secp256k1_key_pair: KeyPair,
}

impl KeyPairCtx {
    pub fn secp256k1_pubkey(&self) -> PublicKey { *self.secp256k1_key_pair.public() }

    pub fn secp256k1_privkey(&self) -> &Private { self.secp256k1_key_pair.private() }

    pub fn secp256k1_privkey_bytes(&self) -> &[u8] { self.secp256k1_privkey().secret.as_slice() }
}

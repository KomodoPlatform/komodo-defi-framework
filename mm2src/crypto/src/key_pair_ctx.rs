use keys::{KeyPair, Private, Public as PublicKey};
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone)]
pub struct IguanaArc(Arc<IguanaCtx>);

impl Deref for IguanaArc {
    type Target = IguanaCtx;

    fn deref(&self) -> &Self::Target { &self.0 }
}

impl From<KeyPair> for IguanaArc {
    fn from(secp256k1_key_pair: KeyPair) -> Self { IguanaArc::new(IguanaCtx { secp256k1_key_pair }) }
}

impl IguanaArc {
    pub fn new(ctx: IguanaCtx) -> IguanaArc { IguanaArc(Arc::new(ctx)) }
}

pub struct IguanaCtx {
    /// secp256k1 key pair derived from passphrase.
    /// cf. `key_pair_from_seed`.
    pub(crate) secp256k1_key_pair: KeyPair,
}

impl IguanaCtx {
    pub fn secp256k1_pubkey(&self) -> PublicKey { *self.secp256k1_key_pair.public() }

    pub fn secp256k1_privkey(&self) -> &Private { self.secp256k1_key_pair.private() }

    pub fn secp256k1_privkey_bytes(&self) -> &[u8] { self.secp256k1_privkey().secret.as_slice() }
}

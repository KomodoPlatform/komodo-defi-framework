//! Bitcoin keys.

extern crate base58;
extern crate bech32;
extern crate bitcrypto as crypto;
extern crate derive_more;
extern crate lazy_static;
extern crate primitives;
extern crate rustc_hex as hex;
extern crate secp256k1;
extern crate serde;
extern crate serde_derive;

mod address;
mod cashaddress;
mod display;
mod error;
mod keypair;
mod network;
mod private;
mod public;
mod segwitaddress;
mod signature;

pub use primitives::{bytes, hash};

pub use address::{Address, AddressFormat, Type};
pub use cashaddress::{AddressType as CashAddrType, CashAddress};
pub use display::DisplayLayout;
pub use error::Error;
pub use keypair::KeyPair;
pub use network::Network;
pub use private::Private;
pub use public::Public;
pub use segwitaddress::SegwitAddress;
pub use signature::{CompactSignature, Signature};

use hash::{H160, H256};
use lazy_static::lazy_static;
use secp256k1::{Secp256k1, SignOnly, VerifyOnly};

/// 20 bytes long hash derived from public `ripemd160(sha256(public))`
pub type AddressHash = H160;
/// 32 bytes long secret key
pub type Secret = H256;
/// 32 bytes long signable message
pub type Message = H256;

lazy_static! {
    static ref SECP_VERIFY: Secp256k1<VerifyOnly> = Secp256k1::verification_only();
    static ref SECP_SIGN: Secp256k1<SignOnly> = Secp256k1::signing_only();
}

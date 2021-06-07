//! Bitcoin keys.

extern crate base58;
extern crate bech32;
extern crate bitcrypto as crypto;
extern crate primitives;
extern crate rustc_hex as hex;
extern crate secp256k1;
extern crate serde;

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

pub use address::{Address, Type};
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

/// 20 bytes long hash derived from public `ripemd160(sha256(public))`
pub type AddressHash = H160;
/// 32 bytes long secret key
pub type Secret = H256;
/// 32 bytes long signable message
pub type Message = H256;

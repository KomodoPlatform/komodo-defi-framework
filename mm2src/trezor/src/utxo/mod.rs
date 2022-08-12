mod prev_tx;
mod sign_utxo;
mod unsigned_tx;
mod utxo_command;

pub use prev_tx::{PrevTx, PrevTxInput, PrevTxOutput};
pub use sign_utxo::TxSignResult;
pub use unsigned_tx::{TrezorInputScriptType, TrezorOutputScriptType, TxOutput, UnsignedTxInput, UnsignedUtxoTx};
pub use utxo_command::{IGNORE_XPUB_MAGIC, KEEP_XPUB_MAGIC};

pub type TxHash = Vec<u8>;
pub type Signature = Vec<u8>;
pub type ScriptPubkey = Vec<u8>;

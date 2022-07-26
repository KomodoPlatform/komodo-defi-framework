use derive_more::Display;

mod prev_tx;
mod sign_utxo;
mod unsigned_tx;
mod utxo_command;

pub use prev_tx::{PrevTx, PrevTxInput, PrevTxOutput};
pub use sign_utxo::TxSignResult;
pub use unsigned_tx::{TrezorInputScriptType, TrezorOutputScriptType, TxOutput, UnsignedTxInput, UnsignedUtxoTx};

pub type TxHash = Vec<u8>;
pub type Signature = Vec<u8>;
pub type ScriptPubkey = Vec<u8>;

#[derive(Clone, Copy, Debug, Display, Deserialize, Serialize)]
pub enum TrezorUtxoCoin {
    Bitcoin,
    Komodo,
    Qtum,
}

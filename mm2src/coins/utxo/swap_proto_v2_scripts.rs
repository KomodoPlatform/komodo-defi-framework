/// This module contains functions building Bitcoins scripts for the "Swap protocol upgrade" feature
/// For more info, see https://github.com/KomodoPlatform/komodo-defi-framework/issues/1895
use bitcrypto::ripemd160;
use keys::Public;
use script::{Builder, Opcode, Script};

/// Builds a script for refundable dex_fee + premium taker transaction
pub fn taker_payment_script(time_lock: u32, secret_hash: &[u8], pub_0: &Public, pub_1: &Public) -> Script {
    let mut builder = Builder::default()
        // Dex fee refund path, same lock time as for taker payment
        .push_opcode(Opcode::OP_IF)
        .push_bytes(&time_lock.to_le_bytes())
        .push_opcode(Opcode::OP_CHECKLOCKTIMEVERIFY)
        .push_opcode(Opcode::OP_DROP)
        .push_bytes(pub_0)
        .push_opcode(Opcode::OP_CHECKSIG)
        // Dex fee redeem path, Maker needs to reveal the secret to prevent case of getting
        // the premium but not proceeding with spending the taker payment
        .push_opcode(Opcode::OP_ELSE)
        .push_opcode(Opcode::OP_SIZE)
        .push_bytes(&[32])
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_opcode(Opcode::OP_HASH160);

    if secret_hash.len() == 32 {
        builder = builder.push_bytes(ripemd160(secret_hash).as_slice());
    } else {
        builder = builder.push_bytes(secret_hash);
    }

    builder
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_opcode(Opcode::OP_2)
        .push_bytes(pub_0)
        .push_bytes(pub_1)
        .push_opcode(Opcode::OP_2)
        .push_opcode(Opcode::OP_CHECKMULTISIG)
        .push_opcode(Opcode::OP_ENDIF)
        .into_script()
}

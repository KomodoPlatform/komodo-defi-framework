/// This module contains functions building Bitcoins scripts for the "Trading protocol upgrade" feature
/// For more info, see https://github.com/KomodoPlatform/komodo-defi-framework/issues/1895
use bitcrypto::ripemd160;
use keys::Public;
use script::{Builder, Opcode, Script};

/// Builds a script for taker funding transaction
pub fn taker_funding_script(
    time_lock: u32,
    taker_secret_hash: &[u8],
    taker_pub: &Public,
    maker_pub: &Public,
) -> Script {
    let mut builder = Builder::default()
        .push_opcode(Opcode::OP_IF)
        .push_bytes(&time_lock.to_le_bytes())
        .push_opcode(Opcode::OP_CHECKLOCKTIMEVERIFY)
        .push_opcode(Opcode::OP_DROP)
        .push_bytes(taker_pub)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ELSE)
        .push_opcode(Opcode::OP_IF)
        .push_bytes(taker_pub)
        .push_opcode(Opcode::OP_CHECKSIGVERIFY)
        .push_bytes(maker_pub)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ELSE)
        .push_opcode(Opcode::OP_SIZE)
        .push_bytes(&[32])
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_opcode(Opcode::OP_HASH160);

    if taker_secret_hash.len() == 32 {
        builder = builder.push_bytes(ripemd160(taker_secret_hash).as_slice());
    } else {
        builder = builder.push_bytes(taker_secret_hash);
    }

    builder
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_bytes(taker_pub)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ENDIF)
        .push_opcode(Opcode::OP_ENDIF)
        .into_script()
}

/// Builds a script for combined trading_volume + dex_fee + premium taker transaction
pub fn taker_payment_script(
    time_lock: u32,
    maker_secret_hash: &[u8],
    taker_pub: &Public,
    maker_pub: &Public,
) -> Script {
    let mut builder = Builder::default()
        .push_opcode(Opcode::OP_IF)
        .push_bytes(&time_lock.to_le_bytes())
        .push_opcode(Opcode::OP_CHECKLOCKTIMEVERIFY)
        .push_opcode(Opcode::OP_DROP)
        .push_bytes(taker_pub)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ELSE)
        .push_opcode(Opcode::OP_SIZE)
        .push_bytes(&[32])
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_opcode(Opcode::OP_HASH160);

    if maker_secret_hash.len() == 32 {
        builder = builder.push_bytes(ripemd160(maker_secret_hash).as_slice());
    } else {
        builder = builder.push_bytes(maker_secret_hash);
    }

    builder
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_bytes(taker_pub)
        .push_opcode(Opcode::OP_CHECKSIGVERIFY)
        .push_bytes(maker_pub)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ENDIF)
        .into_script()
}

/// Builds a script for maker payment with immediate refund path
pub fn maker_payment_script(
    time_lock: u32,
    maker_secret_hash: &[u8],
    taker_secret_hash: &[u8],
    taker_pub: &Public,
    maker_pub: &Public,
) -> Script {
    let mut builder = Builder::default()
        .push_opcode(Opcode::OP_IF)
        .push_bytes(&time_lock.to_le_bytes())
        .push_opcode(Opcode::OP_CHECKLOCKTIMEVERIFY)
        .push_opcode(Opcode::OP_DROP)
        .push_bytes(maker_pub)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ELSE)
        .push_opcode(Opcode::OP_IF)
        .push_opcode(Opcode::OP_SIZE)
        .push_bytes(&[32])
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_opcode(Opcode::OP_HASH160);

    if maker_secret_hash.len() == 32 {
        builder = builder.push_bytes(ripemd160(maker_secret_hash).as_slice());
    } else {
        builder = builder.push_bytes(maker_secret_hash);
    }

    builder = builder
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_bytes(taker_pub)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ELSE)
        .push_opcode(Opcode::OP_SIZE)
        .push_bytes(&[32])
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_opcode(Opcode::OP_HASH160);

    if taker_secret_hash.len() == 32 {
        builder = builder.push_bytes(ripemd160(taker_secret_hash).as_slice());
    } else {
        builder = builder.push_bytes(taker_secret_hash);
    }

    builder
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_bytes(maker_pub)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ENDIF)
        .push_opcode(Opcode::OP_ENDIF)
        .into_script()
}

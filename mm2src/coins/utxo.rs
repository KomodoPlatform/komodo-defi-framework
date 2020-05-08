/******************************************************************************
 * Copyright © 2014-2019 The SuperNET Developers.                             *
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
//  utxo.rs
//  marketmaker
//
//  Copyright © 2017-2019 SuperNET. All rights reserved.
//

#![cfg_attr(not(feature = "native"), allow(unused_imports))]

pub mod rpc_clients;

use base64::{encode_config as base64_encode, URL_SAFE};
use bigdecimal::BigDecimal;
pub use bitcrypto::{dhash160, ChecksumType, sha256};
use chain::{TransactionOutput, TransactionInput, OutPoint};
use chain::constants::{SEQUENCE_FINAL};
use common::{first_char_to_upper, small_rng};
use common::executor::{spawn, Timer};
use common::jsonrpc_client::{JsonRpcError, JsonRpcErrorType};
use common::mm_ctx::MmArc;
use common::mm_number::MmNumber;
#[cfg(feature = "native")]
use dirs::home_dir;
use futures01::{Future};
use futures01::future::Either;
use futures::compat::Future01CompatExt;
use futures::future::{FutureExt, TryFutureExt};
use futures::lock::{Mutex as AsyncMutex};
use gstuff::{now_ms};
use keys::{KeyPair, Private, Public, Address, Secret, Type};
use keys::bytes::Bytes;
use num_traits::cast::ToPrimitive;
use primitives::hash::{H256, H264, H512};
use rand::seq::SliceRandom;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use script::{Opcode, Builder, Script, ScriptAddress, TransactionInputSigner, UnsignedTransactionInput, SignatureVersion};
use serde_json::{self as json, Value as Json};
use serialization::{serialize, deserialize};
use std::borrow::Cow;
use std::collections::hash_map::{HashMap, Entry};
use std::convert::TryInto;
use std::cmp::Ordering;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrderding};
use std::thread;
use std::time::Duration;

pub use chain::Transaction as UtxoTx;

use self::rpc_clients::{electrum_script_hash, ElectrumClient, ElectrumClientImpl, EstimateFeeMethod, NativeClient, UtxoRpcClientEnum, UnspentInfo };
use super::{CoinsContext, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin, SwapOps, TradeFee, TradeInfo,
            Transaction, TransactionEnum, TransactionFut, TransactionDetails, WithdrawFee, WithdrawRequest};
use crate::utxo::rpc_clients::{NativeClientImpl, UtxoRpcClientOps, ElectrumRpcRequest};

#[cfg(test)]
pub mod utxo_tests;

const SWAP_TX_SPEND_SIZE: u64 = 305;
const KILO_BYTE: u64 = 1000;
/// https://bitcoin.stackexchange.com/a/77192
const MAX_DER_SIGNATURE_LEN: usize = 72;
const COMPRESSED_PUBKEY_LEN: usize = 33;
const P2PKH_OUTPUT_LEN: u64 = 34;

#[cfg(windows)]
#[cfg(feature = "native")]
fn get_special_folder_path() -> PathBuf {
    use libc::c_char;
    use std::ffi::CStr;
    use std::mem::zeroed;
    use std::ptr::null_mut;
    use winapi::um::shlobj::SHGetSpecialFolderPathA;
    use winapi::shared::minwindef::MAX_PATH;
    use winapi::um::shlobj::CSIDL_APPDATA;

    let mut buf: [c_char; MAX_PATH + 1] = unsafe {zeroed()};
    // https://docs.microsoft.com/en-us/windows/desktop/api/shlobj_core/nf-shlobj_core-shgetspecialfolderpatha
    let rc = unsafe {SHGetSpecialFolderPathA (null_mut(), buf.as_mut_ptr(), CSIDL_APPDATA, 1)};
    if rc != 1 {panic! ("!SHGetSpecialFolderPathA")}
    Path::new (unwrap! (unsafe {CStr::from_ptr (buf.as_ptr())} .to_str())) .to_path_buf()
}

#[cfg(not(windows))]
#[cfg(feature = "native")]
fn get_special_folder_path() -> PathBuf {panic!("!windows")}

impl Transaction for UtxoTx {
    fn tx_hex(&self) -> Vec<u8> {
        serialize(self).into()
    }

    fn extract_secret(&self) -> Result<Vec<u8>, String> {
        let script: Script = self.inputs[0].script_sig.clone().into();
        for (i, instr) in script.iter().enumerate() {
            let instruction = instr.unwrap();
            if i == 1 {
                if instruction.opcode == Opcode::OP_PUSHBYTES_32 {
                    return Ok(instruction.data.unwrap().to_vec());
                }
            }
        }
        ERR!("Couldn't extract secret")
    }

    fn tx_hash(&self) -> BytesJson { self.hash().reversed().to_vec().into() }
}

/// Additional transaction data that can't be easily got from raw transaction without calling
/// additional RPC methods, e.g. to get input amount we need to request all previous transactions
/// and check output values
#[derive(Debug)]
pub struct AdditionalTxData {
    received_by_me: u64,
    spent_by_me: u64,
    fee_amount: u64,
}

/// The fee set from coins config
#[derive(Debug)]
enum TxFee {
    /// Tell the coin that it has fixed tx fee not depending on transaction size
    Fixed(u64),
    /// Tell the coin that it should request the fee from daemon RPC and calculate it relying on tx size
    Dynamic(EstimateFeeMethod),
}

/// The actual "runtime" fee that is received from RPC in case of dynamic calculation
#[derive(Debug)]
enum ActualTxFee {
    /// fixed tx fee not depending on transaction size
    Fixed(u64),
    /// fee amount per Kbyte received from coin RPC
    Dynamic(u64),
}

/// Fee policy applied on transaction creation
enum FeePolicy {
    /// Send the exact amount specified in output(s), fee is added to spent input amount
    SendExact,
    /// Contains the index of output from which fee should be deducted
    DeductFromOutput(usize),
}

#[derive(Debug)]
pub struct UtxoCoinImpl {  // pImpl idiom.
    ticker: String,
    /// https://en.bitcoin.it/wiki/List_of_address_prefixes
    /// https://github.com/jl777/coins/blob/master/coins
    pub_addr_prefix: u8,
    p2sh_addr_prefix: u8,
    wif_prefix: u8,
    pub_t_addr_prefix: u8,
    p2sh_t_addr_prefix: u8,
    /// True if coins uses Proof of Stake consensus algo
    /// Proof of Work is expected by default
    /// https://en.bitcoin.it/wiki/Proof_of_Stake
    /// https://en.bitcoin.it/wiki/Proof_of_work
    /// The actual meaning of this is nTime field is used in transaction
    is_pos: bool,
    /// Special field for Zcash and it's forks
    /// Defines if Overwinter network upgrade was activated
    /// https://z.cash/upgrade/overwinter/
    overwintered: bool,
    /// The tx version used to detect the transaction ser/de/signing algo
    /// For now it's mostly used for Zcash and forks because they changed the algo in
    /// Overwinter and then Sapling upgrades
    /// https://github.com/zcash/zips/blob/master/zip-0243.rst
    tx_version: i32,
    /// If true - allow coins withdraw to P2SH addresses (Segwit).
    /// the flag will also affect the address that MM2 generates by default in the future
    /// will be the Segwit (starting from 3 for BTC case) instead of legacy
    /// https://en.bitcoin.it/wiki/Segregated_Witness
    segwit: bool,
    /// Default decimals amount is 8 (BTC and almost all other UTXO coins)
    /// But there are forks which have different decimals:
    /// Peercoin has 6
    /// Emercoin has 6
    /// Bitcoin Diamond has 7
    decimals: u8,
    /// Does coin require transactions to be notarized to be considered as confirmed?
    /// https://komodoplatform.com/security-delayed-proof-of-work-dpow/
    requires_notarization: AtomicBool,
    /// RPC client
    rpc_client: UtxoRpcClientEnum,
    /// ECDSA key pair
    key_pair: KeyPair,
    /// Lock the mutex when we deal with address utxos
    my_address: Address,
    /// Is current coin KMD asset chain?
    /// https://komodoplatform.atlassian.net/wiki/spaces/KPSD/pages/71729160/What+is+a+Parallel+Chain+Asset+Chain
    asset_chain: bool,
    tx_fee: TxFee,
    /// Transaction version group id for Zcash transactions since Overwinter: https://github.com/zcash/zips/blob/master/zip-0202.rst
    version_group_id: u32,
    /// Consensus branch id for Zcash transactions since Overwinter: https://github.com/zcash/zcash/blob/master/src/consensus/upgrades.cpp#L11
    /// used in transaction sig hash calculation
    consensus_branch_id: u32,
    /// Defines if coin uses Zcash transaction format
    zcash: bool,
    /// Address and privkey checksum type
    checksum_type: ChecksumType,
    /// Fork id used in sighash
    fork_id: u32,
    /// Signature version
    signature_version: SignatureVersion,
    history_sync_state: Mutex<HistorySyncState>,
    required_confirmations: AtomicU64,
    /// if set to true MM2 will check whether calculated fee is lower than relay fee and use
    /// relay fee amount instead of calculated
    /// https://github.com/KomodoPlatform/atomicDEX-API/issues/617
    force_min_relay_fee: bool,
}

impl UtxoCoinImpl {
    async fn get_tx_fee(&self) -> Result<ActualTxFee, JsonRpcError> {
        match &self.tx_fee {
            TxFee::Fixed(fee) => Ok(ActualTxFee::Fixed(*fee)),
            TxFee::Dynamic(method) => {
                let fee = self.rpc_client.estimate_fee_sat(self.decimals, method).compat().await?;
                Ok(ActualTxFee::Dynamic(fee))
            },
        }
    }

    /// returns the fee required to be paid for HTLC spend transaction
    async fn get_htlc_spend_fee(&self) -> Result<u64, String> {
        let coin_fee = try_s!(self.get_tx_fee().await);
        let mut fee = match coin_fee {
            ActualTxFee::Fixed(fee) => fee,
            // atomic swap payment spend transaction is slightly more than 300 bytes in average as of now
            ActualTxFee::Dynamic(fee_per_kb) => (fee_per_kb * SWAP_TX_SPEND_SIZE) / KILO_BYTE,
        };
        if self.force_min_relay_fee {
            let relay_fee = try_s!(self.rpc_client.get_relay_fee().compat().await);
            let relay_fee_sat = try_s!(sat_from_big_decimal(&relay_fee, self.decimals));
            if fee < relay_fee_sat {
                fee = relay_fee_sat;
            }
        }
        Ok(fee)
    }

    fn addresses_from_script(&self, script: &Script) -> Result<Vec<Address>, String> {
        let destinations: Vec<ScriptAddress> = try_s!(script.extract_destinations());

        let addresses = destinations.into_iter().map(|dst| {
            let (prefix, t_addr_prefix) = match dst.kind {
                Type::P2PKH => (self.pub_addr_prefix, self.pub_t_addr_prefix),
                Type::P2SH => (self.p2sh_addr_prefix, self.p2sh_t_addr_prefix),
            };

            Address {
                hash: dst.hash,
                checksum_type: self.checksum_type,
                prefix,
                t_addr_prefix,
            }
        }).collect();

        Ok(addresses)
    }

    pub fn denominate_satoshis(&self, satoshi: i64) -> f64 {
        satoshi as f64 / 10f64.powf(self.decimals as f64)
    }

    fn search_for_swap_tx_spend(
        &self,
        time_lock: u32,
        first_pub: &Public,
        second_pub: &Public,
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let tx: UtxoTx = try_s!(deserialize(tx).map_err(|e| ERRL!("{:?}", e)));
        let script = payment_script(time_lock, secret_hash, first_pub, second_pub);
        let expected_script_pubkey = Builder::build_p2sh(&dhash160(&script)).to_bytes();
        if tx.outputs[0].script_pubkey != expected_script_pubkey {
            return ERR!("Transaction {:?} output 0 script_pubkey doesn't match expected {:?}", tx, expected_script_pubkey);
        }

        let spend = try_s!(self.rpc_client.find_output_spend(&tx, 0, search_from_block).wait());
        match spend {
            Some(tx) => {
                let script: Script = tx.inputs[0].script_sig.clone().into();
                match script.iter().nth(2) {
                    Some(instruction) => match instruction {
                        Ok(ref i) if i.opcode == Opcode::OP_0 => return Ok(Some(FoundSwapTxSpend::Spent(tx.into()))),
                        _ => (),
                    },
                    None => (),
                };

                match script.iter().nth(1) {
                    Some(instruction) => match instruction {
                        Ok(ref i) if i.opcode == Opcode::OP_1 => return Ok(Some(FoundSwapTxSpend::Refunded(tx.into()))),
                        _ => (),
                    },
                    None => (),
                };

                ERR!("Couldn't find required instruction in script_sig of input 0 of tx {:?}", tx)
            },
            None => Ok(None),
        }
    }

    pub fn my_public_key(&self) -> &Public {
        self.key_pair.public()
    }

    pub fn rpc_client(&self) -> &UtxoRpcClientEnum {
        &self.rpc_client
    }
}

fn payment_script(
    time_lock: u32,
    secret_hash: &[u8],
    pub_0: &Public,
    pub_1: &Public
) -> Script {
    let builder = Builder::default();
    builder
        .push_opcode(Opcode::OP_IF)
        .push_bytes(&time_lock.to_le_bytes())
        .push_opcode(Opcode::OP_CHECKLOCKTIMEVERIFY)
        .push_opcode(Opcode::OP_DROP)
        .push_bytes(pub_0)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ELSE)
        .push_opcode(Opcode::OP_SIZE)
        .push_bytes(&[32])
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_opcode(Opcode::OP_HASH160)
        .push_bytes(secret_hash)
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_bytes(pub_1)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ENDIF)
        .into_script()
}

fn script_sig(message: &H256, key_pair: &KeyPair, fork_id: u32) -> Result<Bytes, String> {
    let signature = try_s!(key_pair.private().sign(message));

    let mut sig_script = Bytes::default();
    sig_script.append(&mut Bytes::from((*signature).to_vec()));
    // Using SIGHASH_ALL only for now
    sig_script.append(&mut Bytes::from(vec![1 | fork_id as u8]));

    Ok(sig_script)
}

fn script_sig_with_pub(message: &H256, key_pair: &KeyPair, fork_id: u32) -> Result<Bytes, String> {
    let sig_script = try_s!(script_sig(message, key_pair, fork_id));

    let builder = Builder::default();

    Ok(builder
        .push_data(&sig_script)
        .push_data(&key_pair.public().to_vec())
        .into_bytes())
}

/// Creates signed input spending p2pkh output
fn p2pkh_spend(
    signer: &TransactionInputSigner,
    input_index: usize,
    key_pair: &KeyPair,
    prev_script: &Script,
    signature_version: SignatureVersion,
    fork_id: u32,
) -> Result<TransactionInput, String> {
    let script = Builder::build_p2pkh(&key_pair.public().address_hash());
    if script != *prev_script {
        return ERR!("p2pkh script {} built from input key pair doesn't match expected prev script {}", script, prev_script);
    }
    let sighash_type = 1 | fork_id;
    let sighash = signer.signature_hash(input_index, signer.inputs[input_index].amount, &script, signature_version, sighash_type);

    let script_sig = try_s!(script_sig_with_pub(&sighash, key_pair, fork_id));

    Ok(TransactionInput {
        script_sig,
        sequence: signer.inputs[input_index].sequence,
        script_witness: vec![],
        previous_output: signer.inputs[input_index].previous_output.clone()
    })
}

/// Creates signed input spending hash time locked p2sh output
fn p2sh_spend(
    signer: &TransactionInputSigner,
    input_index: usize,
    key_pair: &KeyPair,
    script_data: Script,
    redeem_script: Script,
    signature_version: SignatureVersion,
    fork_id: u32,
) -> Result<TransactionInput, String> {
    let sighash = signer.signature_hash(input_index, signer.inputs[input_index].amount, &redeem_script, signature_version, 1 | fork_id);

    let sig = try_s!(script_sig(&sighash, &key_pair, fork_id));

    let mut resulting_script = Builder::default().push_data(&sig).into_bytes();
    if !script_data.is_empty() {
        resulting_script.extend_from_slice(&script_data);
    }

    let redeem_part = Builder::default().push_data(&redeem_script).into_bytes();
    resulting_script.extend_from_slice(&redeem_part);

    Ok(TransactionInput {
        script_sig: resulting_script.into(),
        sequence: signer.inputs[input_index].sequence,
        script_witness: vec![],
        previous_output: signer.inputs[input_index].previous_output.clone()
    })
}

fn address_from_raw_pubkey(pub_key: &[u8], prefix: u8, t_addr_prefix: u8, checksum_type: ChecksumType) -> Result<Address, String> {
    Ok(Address {
        t_addr_prefix,
        prefix,
        hash: try_s!(Public::from_slice(pub_key)).address_hash(),
        checksum_type,
    })
}

fn sign_tx(
    unsigned: TransactionInputSigner,
    key_pair: &KeyPair,
    prev_script: Script,
    signature_version: SignatureVersion,
    fork_id: u32,
) -> Result<UtxoTx, String> {
    let mut signed_inputs = vec![];
    for (i, _) in unsigned.inputs.iter().enumerate() {
        signed_inputs.push(
            try_s!(p2pkh_spend(&unsigned, i, key_pair, &prev_script, signature_version, fork_id))
        );
    }
    Ok(UtxoTx {
        inputs: signed_inputs,
        n_time: unsigned.n_time,
        outputs: unsigned.outputs.clone(),
        version: unsigned.version,
        overwintered: unsigned.overwintered,
        lock_time: unsigned.lock_time,
        expiry_height: unsigned.expiry_height,
        join_splits: vec![],
        shielded_spends: vec![],
        shielded_outputs: vec![],
        value_balance: 0,
        version_group_id: unsigned.version_group_id,
        binding_sig: H512::default(),
        join_split_sig: H512::default(),
        join_split_pubkey: H256::default(),
        zcash: unsigned.zcash,
        str_d_zeel: unsigned.str_d_zeel,
    })
}

/// Denominate BigDecimal amount of coin units to satoshis
fn sat_from_big_decimal(amount: &BigDecimal, decimals: u8) -> Result<u64, String> {
    (amount * BigDecimal::from(10u64.pow(decimals as u32))).to_u64().ok_or(ERRL!("Could not get sat from amount {} with decimals {}", amount, decimals))
}

/// Convert satoshis to BigDecimal amount of coin units
fn big_decimal_from_sat(satoshis: i64, decimals: u8) -> BigDecimal {
    BigDecimal::from(satoshis) / BigDecimal::from(10u64.pow(decimals as u32))
}

#[derive(Clone, Debug)]
pub struct UtxoCoin(Arc<UtxoCoinImpl>);
impl Deref for UtxoCoin {type Target = UtxoCoinImpl; fn deref (&self) -> &UtxoCoinImpl {&*self.0}}

impl From<UtxoCoinImpl> for UtxoCoin {
    fn from(coin: UtxoCoinImpl) -> UtxoCoin {
        UtxoCoin(Arc::new(coin))
    }
}

// We can use a shared UTXO lock for all UTXO coins at 1 time.
// It's highly likely that we won't experience any issues with it as we won't need to send "a lot" of transactions concurrently.
lazy_static! {static ref UTXO_LOCK: AsyncMutex<()> = AsyncMutex::new(());}

macro_rules! true_or_err {
    ($cond: expr, $msg: expr $(, $args:expr)*) => {
        if !$cond {
            return ERR!($msg $(, $args)*);
        }
    };
}

async fn send_outputs_from_my_address_impl(coin: UtxoCoin, outputs: Vec<TransactionOutput>)
    -> Result<UtxoTx, String> {
    let _utxo_lock = UTXO_LOCK.lock().await;
    let unspents = try_s!(coin.rpc_client.list_unspent_ordered(&coin.my_address).map_err(|e| ERRL!("{}", e)).compat().await);
    let (unsigned, _) = try_s!(coin.generate_transaction(unspents, outputs, FeePolicy::SendExact, None).await);
    let prev_script = Builder::build_p2pkh(&coin.my_address.hash);
    let signed = try_s!(sign_tx(unsigned, &coin.key_pair, prev_script, coin.signature_version, coin.fork_id));
    try_s!(coin.rpc_client.send_transaction(&signed, coin.my_address.clone()).map_err(|e| ERRL!("{}", e)).compat().await);
    Ok(signed)
}

impl UtxoCoin {
    fn send_outputs_from_my_address(&self, outputs: Vec<TransactionOutput>) -> TransactionFut {
        let fut = send_outputs_from_my_address_impl(self.clone(), outputs);
        Box::new(fut.boxed().compat().map(|tx| tx.into()))
    }

    fn validate_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        first_pub0: &Public,
        second_pub0: &Public,
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send> {
        let tx: UtxoTx = try_fus!(deserialize(payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let amount = try_fus!(sat_from_big_decimal(&amount, self.decimals));
        let selfi = self.clone();

        let expected_redeem = payment_script(
            time_lock,
            priv_bn_hash,
            &try_fus!(Public::from_slice(first_pub0)),
            &try_fus!(Public::from_slice(second_pub0)),
        );
        let fut = async move {
            let mut attempts = 0;
            loop {
                let tx_from_rpc = match selfi.rpc_client.get_transaction_bytes(tx.hash().reversed().into()).compat().await {
                    Ok(t) => t,
                    Err(e) => {
                        if attempts > 2 {
                            return ERR!("Got error {:?} after 3 attempts of getting tx {:?} from RPC", e, tx.tx_hash());
                        };
                        attempts += 1;
                        log!("Error " [e] " getting the tx " [tx.tx_hash()] " from rpc");
                        Timer::sleep(10.).await;
                        continue;
                    }
                };
                if serialize(&tx).take() != tx_from_rpc.0 {
                    return ERR!("Provided payment tx {:?} doesn't match tx data from rpc {:?}", tx, tx_from_rpc);
                }

                let expected_output = TransactionOutput {
                    value: amount,
                    script_pubkey: Builder::build_p2sh(&dhash160(&expected_redeem)).into(),
                };

                if tx.outputs[0] != expected_output {
                    return ERR!("Provided payment tx output doesn't match expected {:?} {:?}", tx.outputs[0], expected_output);
                }
                return Ok(());
            }
        };
        Box::new(fut.boxed().compat())
    }

    /// Generates unsigned transaction (TransactionInputSigner) from specified utxos and outputs.
    /// This function expects that utxos are sorted by amounts in ascending order
    /// Consider sorting before calling this function
    /// Sends the change (inputs amount - outputs amount) to "my_address"
    /// Also returns additional transaction data
    async fn generate_transaction(
        &self,
        utxos: Vec<UnspentInfo>,
        outputs: Vec<TransactionOutput>,
        fee_policy: FeePolicy,
        fee: Option<ActualTxFee>,
    ) -> Result<(TransactionInputSigner, AdditionalTxData), String> {
        const DUST: u64 = 1000;
        let lock_time = (now_ms() / 1000) as u32;
        let change_script_pubkey = Builder::build_p2pkh(&self.my_address.hash).to_bytes();
        let coin_tx_fee = match fee {
            Some(f) => f,
            None => try_s!(self.get_tx_fee().await),
        };
        true_or_err!(!utxos.is_empty(), "Couldn't generate tx from empty utxos set");
        true_or_err!(!outputs.is_empty(), "Couldn't generate tx from empty outputs set");

        let mut sum_outputs_value = 0;
        let mut received_by_me = 0;
        for output in outputs.iter() {
            true_or_err!(output.value >= DUST, "Output value {} is less than dust amount {}", output.value, DUST);
            sum_outputs_value += output.value;
            if output.script_pubkey == change_script_pubkey {
                received_by_me += output.value;
            }
        }

        true_or_err!(sum_outputs_value > 0, "Sum of outputs {:?} is zero", outputs);
        let str_d_zeel = if self.ticker == "NAV" {
            Some("".into())
        } else {
            None
        };
        let mut tx = TransactionInputSigner {
            inputs: vec![],
            outputs,
            lock_time,
            version: self.tx_version,
            n_time: if self.is_pos { Some((now_ms() / 1000) as u32) } else { None },
            overwintered: self.overwintered,
            expiry_height: 0,
            join_splits: vec![],
            shielded_spends: vec![],
            shielded_outputs: vec![],
            value_balance: 0,
            version_group_id: self.version_group_id,
            consensus_branch_id: self.consensus_branch_id,
            zcash: self.zcash,
            str_d_zeel,
        };
        let mut sum_inputs = 0;
        let mut tx_fee = 0;
        let min_relay_fee = if self.force_min_relay_fee {
            let fee_dec = try_s!(self.rpc_client.get_relay_fee().compat().await);
            Some(try_s!(sat_from_big_decimal(&fee_dec, self.decimals)))
        } else {
            None
        };
        for utxo in utxos.iter() {
            sum_inputs += utxo.value;
            tx.inputs.push(UnsignedTransactionInput {
                previous_output: utxo.outpoint.clone(),
                sequence: SEQUENCE_FINAL,
                amount: utxo.value,
            });
            tx_fee = match &coin_tx_fee {
                ActualTxFee::Fixed(f) => *f,
                ActualTxFee::Dynamic(f) => {
                    let transaction = UtxoTx::from(tx.clone());
                    let transaction_bytes = serialize(&transaction);
                    // 2 bytes are used to indicate the length of signature and pubkey
                    // total is 107
                    let additional_len = 2 + MAX_DER_SIGNATURE_LEN + COMPRESSED_PUBKEY_LEN;
                    let tx_size = transaction_bytes.len() + transaction.inputs().len() * additional_len;
                    (f * tx_size as u64) / KILO_BYTE
                },
            };
            match fee_policy {
                FeePolicy::SendExact => {
                    let mut outputs_plus_fee = sum_outputs_value + tx_fee;
                    if sum_inputs >= outputs_plus_fee {
                        if sum_inputs - outputs_plus_fee > DUST {
                            // there will be change output if sum_inputs - outputs_plus_fee > DUST
                            if let ActualTxFee::Dynamic(ref f) = coin_tx_fee {
                                tx_fee += (f * P2PKH_OUTPUT_LEN) / KILO_BYTE;
                                outputs_plus_fee += (f * P2PKH_OUTPUT_LEN) / KILO_BYTE;
                            }
                        }
                        if let Some(min_relay) = min_relay_fee {
                            if tx_fee < min_relay {
                                outputs_plus_fee -= tx_fee;
                                outputs_plus_fee += min_relay;
                                tx_fee = min_relay;
                            }
                        }
                        if sum_inputs >= outputs_plus_fee {
                            break;
                        }
                    }
                    ()
                },
                FeePolicy::DeductFromOutput(_) => {
                    if sum_inputs >= sum_outputs_value {
                        if sum_inputs - sum_outputs_value > DUST {
                            if let ActualTxFee::Dynamic(ref f) = coin_tx_fee {
                                tx_fee += (f * P2PKH_OUTPUT_LEN) / KILO_BYTE;
                            }
                            if let Some(min_relay) = min_relay_fee {
                                if tx_fee < min_relay {
                                    tx_fee = min_relay;
                                }
                            }
                        }
                        break;
                    }
                },
            };
        }
        match fee_policy {
            FeePolicy::SendExact => sum_outputs_value += tx_fee,
            FeePolicy::DeductFromOutput(i) => {
                let min_output = tx_fee + DUST;
                let val = tx.outputs[i].value;
                true_or_err!(val >= min_output, "Output {} value {} is too small, required no less than {}", i, val, min_output);
                tx.outputs[i].value -= tx_fee;
                if tx.outputs[i].script_pubkey == change_script_pubkey {
                    received_by_me -= tx_fee;
                }
            },
        };
        true_or_err!(sum_inputs >= sum_outputs_value, "Not sufficient balance. Couldn't collect enough value from utxos {:?} to create tx with outputs {:?}", utxos, tx.outputs);

        let change = sum_inputs - sum_outputs_value;
        if change >= DUST {
            tx.outputs.push({
                TransactionOutput {
                    value: change,
                    script_pubkey: change_script_pubkey.clone()
                }
            });
            received_by_me += change;
        } else {
            tx_fee += change;
        }

        let data = AdditionalTxData {
            fee_amount: tx_fee,
            received_by_me,
            spent_by_me: sum_inputs,
        };
        self.calc_interest_if_required(tx.into(), data, change_script_pubkey).await
    }

    /// Calculates interest if the coin is KMD
    /// Adds the value to existing output to my_script_pub or creates additional interest output
    /// returns transaction and data as is if the coin is not KMD
    async fn calc_interest_if_required(
        &self,
        mut unsigned: TransactionInputSigner,
        mut data: AdditionalTxData,
        my_script_pub: Bytes,
    ) -> Result<(TransactionInputSigner, AdditionalTxData), String> {
        if self.ticker != "KMD" {
            return Ok((unsigned, data));
        }
        unsigned.lock_time = (now_ms() / 1000) as u32 - 777;
        let mut interest = 0;
        for input in unsigned.inputs.iter() {
            let prev_hash = input.previous_output.hash.reversed().into();
            let tx = try_s!(self.rpc_client.get_verbose_transaction(prev_hash).compat().await);
            interest += kmd_interest(tx.height, input.amount, tx.locktime as u64, unsigned.lock_time as u64);
        }
        if interest > 0 {
            data.received_by_me += interest;
            let mut output_to_me = unsigned.outputs.iter_mut().find(|out| out.script_pubkey == my_script_pub);
            // add calculated interest to existing output to my address
            // or create the new one if it's not found
            match output_to_me {
                Some(ref mut output) => output.value += interest,
                None => {
                    let interest_output = TransactionOutput {
                        script_pubkey: my_script_pub,
                        value: interest,
                    };
                    unsigned.outputs.push(interest_output);
                }
            };
        } else {
            // if interest is zero attempt to set the lowest possible lock_time to claim it later
            unsigned.lock_time = (now_ms() / 1000) as u32 - 3600 + 777 * 2;
        }
        Ok((unsigned, data))
    }

    fn p2sh_spending_tx(
        &self,
        prev_transaction: UtxoTx,
        redeem_script: Bytes,
        outputs: Vec<TransactionOutput>,
        script_data: Script,
        sequence: u32,
    ) -> Result<UtxoTx, String> {
        // https://github.com/bitcoin/bitcoin/blob/master/doc/release-notes/release-notes-0.11.2.md#bip113-mempool-only-locktime-enforcement-using-getmediantimepast
        // Implication for users: GetMedianTimePast() always trails behind the current time,
        // so a transaction locktime set to the present time will be rejected by nodes running this
        // release until the median time moves forward.
        // To compensate, subtract one hour (3,600 seconds) from your locktimes to allow those
        // transactions to be included in mempools at approximately the expected time.
        let lock_time = if self.ticker == "KMD" {
            (now_ms() / 1000) as u32 - 3600 + 2 * 777
        } else {
            (now_ms() / 1000) as u32 - 3600
        };
        let n_time = if self.is_pos { Some((now_ms() / 1000) as u32) } else { None };
        let str_d_zeel = if self.ticker == "NAV" { Some("".into()) } else { None };
        let unsigned = TransactionInputSigner {
            lock_time,
            version: self.tx_version,
            n_time,
            overwintered: self.overwintered,
            inputs: vec![UnsignedTransactionInput {
                sequence,
                previous_output: OutPoint {
                    hash: prev_transaction.hash(),
                    index: 0,
                },
                amount: prev_transaction.outputs[0].value,
            }],
            outputs: outputs.clone(),
            expiry_height: 0,
            join_splits: vec![],
            shielded_spends: vec![],
            shielded_outputs: vec![],
            value_balance: 0,
            version_group_id: self.version_group_id,
            consensus_branch_id: self.consensus_branch_id,
            zcash: self.zcash,
            str_d_zeel,
        };
        let signed_input = try_s!(
            p2sh_spend(&unsigned, 0, &self.key_pair, script_data, redeem_script.into(), self.signature_version, self.fork_id)
        );
        Ok(UtxoTx {
            version: unsigned.version,
            n_time: unsigned.n_time,
            overwintered: unsigned.overwintered,
            lock_time: unsigned.lock_time,
            inputs: vec![signed_input],
            outputs,
            expiry_height: unsigned.expiry_height,
            join_splits: vec![],
            shielded_spends: vec![],
            shielded_outputs: vec![],
            value_balance: 0,
            version_group_id: self.version_group_id,
            binding_sig: H512::default(),
            join_split_sig: H512::default(),
            join_split_pubkey: H256::default(),
            zcash: self.zcash,
            str_d_zeel: unsigned.str_d_zeel,
        })
    }
}

pub fn compressed_key_pair_from_bytes(raw: &[u8], prefix: u8, checksum_type: ChecksumType) -> Result<KeyPair, String> {
    if raw.len() != 32 {
        return ERR!("Invalid raw priv key len {}", raw.len());
    }

    let private = Private {
        prefix,
        compressed: true,
        secret: Secret::from(raw),
        checksum_type,
    };
    Ok(try_s!(KeyPair::from_private(private)))
}

pub fn compressed_pub_key_from_priv_raw(raw_priv: &[u8], sum_type: ChecksumType) -> Result<H264, String> {
    let key_pair: KeyPair = try_s!(compressed_key_pair_from_bytes(raw_priv, 0, sum_type));
    Ok(H264::from(&**key_pair.public()))
}

impl SwapOps for UtxoCoin {
    fn send_taker_fee(&self, fee_pub_key: &[u8], amount: BigDecimal) -> TransactionFut {
        let address = try_fus!(address_from_raw_pubkey(fee_pub_key, self.pub_addr_prefix, self.pub_t_addr_prefix, self.checksum_type));
        let amount = try_fus!(sat_from_big_decimal(&amount, self.decimals));
        let output = TransactionOutput {
            value: amount,
            script_pubkey: Builder::build_p2pkh(&address.hash).to_bytes()
        };
        self.send_outputs_from_my_address(vec![output])
    }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
    ) -> TransactionFut {
        let redeem_script = payment_script(
            time_lock,
            secret_hash,
            self.key_pair.public(),
            &try_fus!(Public::from_slice(taker_pub)),
        );
        let amount = try_fus!(sat_from_big_decimal(&amount, self.decimals));
        let output = TransactionOutput {
            value: amount,
            script_pubkey: Builder::build_p2sh(&dhash160(&redeem_script)).into(),
        };
        let send_fut = match &self.rpc_client {
            UtxoRpcClientEnum::Electrum(_) => Either::A(self.send_outputs_from_my_address(vec![output]).map_err(|e| ERRL!("{}", e))),
            UtxoRpcClientEnum::Native(client) => {
                let payment_addr = Address {
                    checksum_type: self.checksum_type,
                    hash: dhash160(&redeem_script),
                    prefix: self.p2sh_addr_prefix,
                    t_addr_prefix: self.p2sh_t_addr_prefix,
                };
                let arc = self.clone();
                let addr_string = payment_addr.to_string();
                Either::B(client.import_address(&addr_string, &addr_string, false).map_err(|e| ERRL!("{}", e)).and_then(move |_|
                    arc.send_outputs_from_my_address(vec![output]).map_err(|e| ERRL!("{}", e))
                ))
            }
        };
        Box::new(send_fut)
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> TransactionFut {
        let redeem_script = payment_script(
            time_lock,
            priv_bn_hash,
            self.key_pair.public(),
            &try_fus!(Public::from_slice(maker_pub)),
        );

        let amount = try_fus!(sat_from_big_decimal(&amount, self.decimals));

        let output = TransactionOutput {
            value: amount,
            script_pubkey: Builder::build_p2sh(&dhash160(&redeem_script)).into(),
        };
        let send_fut = match &self.rpc_client {
            UtxoRpcClientEnum::Electrum(_) => Either::A(self.send_outputs_from_my_address(vec![output])),
            UtxoRpcClientEnum::Native(client) => {
                let payment_addr = Address {
                    checksum_type: self.checksum_type,
                    hash: dhash160(&redeem_script),
                    prefix: self.p2sh_addr_prefix,
                    t_addr_prefix: self.p2sh_t_addr_prefix,
                };
                let arc = self.clone();
                let addr_string = payment_addr.to_string();
                Either::B(client.import_address(&addr_string, &addr_string, false).map_err(|e| ERRL!("{}", e)).and_then(move |_|
                    arc.send_outputs_from_my_address(vec![output])
                ))
            }
        };
        Box::new(send_fut)
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut {
        let prev_tx: UtxoTx = try_fus!(deserialize(taker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let script_data = Builder::default()
            .push_data(secret)
            .push_opcode(Opcode::OP_0)
            .into_script();
        let redeem_script = payment_script(time_lock, &*dhash160(secret), &try_fus!(Public::from_slice(taker_pub)), self.key_pair.public());
        let arc = self.clone();
        let fut = async move {
            let fee = try_s!(arc.get_htlc_spend_fee().await);
            let output = TransactionOutput {
                value: prev_tx.outputs[0].value - fee,
                script_pubkey: Builder::build_p2pkh(&arc.key_pair.public().address_hash()).to_bytes()
            };
            let transaction = try_s!(arc.p2sh_spending_tx(
                prev_tx,
                redeem_script.into(),
                vec![output],
                script_data,
                SEQUENCE_FINAL,
            ));
            let tx_fut = arc.rpc_client.send_transaction(&transaction, arc.my_address.clone()).compat();
            try_s!(tx_fut.await);
            Ok(transaction.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut {
        let prev_tx: UtxoTx = try_fus!(deserialize(maker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let script_data = Builder::default()
            .push_data(secret)
            .push_opcode(Opcode::OP_0)
            .into_script();
        let redeem_script = payment_script(time_lock, &*dhash160(secret), &try_fus!(Public::from_slice(maker_pub)), self.key_pair.public());
        let arc = self.clone();
        let fut = async move {
            let fee = try_s!(arc.get_htlc_spend_fee().await);
            let output = TransactionOutput {
                value: prev_tx.outputs[0].value - fee,
                script_pubkey: Builder::build_p2pkh(&arc.key_pair.public().address_hash()).to_bytes()
            };
            let transaction = try_s!(arc.p2sh_spending_tx(
                prev_tx,
                redeem_script.into(),
                vec![output],
                script_data,
                SEQUENCE_FINAL,
            ));
            let tx_fut = arc.rpc_client.send_transaction(&transaction, arc.my_address.clone()).compat();
            try_s!(tx_fut.await);
            Ok(transaction.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
    ) -> TransactionFut {
        let prev_tx: UtxoTx = try_fus!(deserialize(taker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let script_data = Builder::default()
            .push_opcode(Opcode::OP_1)
            .into_script();
        let redeem_script = payment_script(time_lock, secret_hash, self.key_pair.public(), &try_fus!(Public::from_slice(maker_pub)));
        let arc = self.clone();
        let fut = async move {
            let fee = try_s!(arc.get_htlc_spend_fee().await);
            let output = TransactionOutput {
                value: prev_tx.outputs[0].value - fee,
                script_pubkey: Builder::build_p2pkh(&arc.key_pair.public().address_hash()).to_bytes()
            };
            let transaction = try_s!(arc.p2sh_spending_tx(
                prev_tx,
                redeem_script.into(),
                vec![output],
                script_data,
                SEQUENCE_FINAL - 1,
            ));
            let tx_fut = arc.rpc_client.send_transaction(&transaction, arc.my_address.clone()).compat();
            try_s!(tx_fut.await);
            Ok(transaction.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
    ) -> TransactionFut {
        let prev_tx: UtxoTx = try_fus!(deserialize(maker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let script_data = Builder::default()
            .push_opcode(Opcode::OP_1)
            .into_script();
        let redeem_script = payment_script(
            time_lock,
            secret_hash,
            self.key_pair.public(),
            &try_fus!(Public::from_slice(taker_pub)),
        );
        let arc = self.clone();
        let fut = async move {
            let fee = try_s!(arc.get_htlc_spend_fee().await);
            let output = TransactionOutput {
                value: prev_tx.outputs[0].value - fee,
                script_pubkey: Builder::build_p2pkh(&arc.key_pair.public().address_hash()).to_bytes()
            };
            let transaction = try_s!(arc.p2sh_spending_tx(
                prev_tx,
                redeem_script.into(),
                vec![output],
                script_data,
                SEQUENCE_FINAL - 1,
            ));
            let tx_fut = arc.rpc_client.send_transaction(&transaction, arc.my_address.clone()).compat();
            try_s!(tx_fut.await);
            Ok(transaction.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        fee_addr: &[u8],
        amount: &BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send> {
        let selfi = self.clone();
        let tx = match fee_tx {
            TransactionEnum::UtxoTx(tx) => tx.clone(),
            _ => panic!(),
        };
        let amount = amount.clone();
        let address = try_fus!(address_from_raw_pubkey(fee_addr, selfi.pub_addr_prefix, selfi.pub_t_addr_prefix, selfi.checksum_type));

        let fut = async move {
            let amount = try_s!(sat_from_big_decimal(&amount, selfi.decimals));
            let tx_from_rpc = try_s!(selfi.rpc_client.get_transaction_bytes(tx.hash().reversed().into()).compat().await);

            if tx_from_rpc.0 != serialize(&tx).take() {
                return ERR!("Provided dex fee tx {:?} doesn't match tx data from rpc {:?}", tx, tx_from_rpc);
            }

            match tx.outputs.first() {
                Some(out) => {
                    let expected_script_pubkey = Builder::build_p2pkh(&address.hash).to_bytes();
                    if out.script_pubkey != expected_script_pubkey {
                        return ERR!("Provided dex fee tx output script_pubkey doesn't match expected {:?} {:?}", out.script_pubkey, expected_script_pubkey);
                    }
                    if out.value < amount {
                        return ERR!("Provided dex fee tx output value is less than expected {:?} {:?}", out.value, amount);
                    }
                },
                None => {
                    return ERR!("Provided dex fee tx {:?} has no outputs", tx);
                }
            }
            Ok(())
        };
        Box::new(fut.boxed().compat())
    }

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send> {
        self.validate_payment(
            payment_tx,
            time_lock,
            &try_fus!(Public::from_slice(maker_pub)),
            self.key_pair.public(),
            priv_bn_hash,
            amount
        )
    }

    fn validate_taker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send> {
        self.validate_payment(
            payment_tx,
            time_lock,
            &try_fus!(Public::from_slice(taker_pub)),
            self.key_pair.public(),
            priv_bn_hash,
            amount
        )
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        _from_block: u64,
    ) -> Box<dyn Future<Item=Option<TransactionEnum>, Error=String> + Send> {
        let script = payment_script(
            time_lock,
            secret_hash,
            self.key_pair.public(),
            &try_fus!(Public::from_slice(other_pub)),
        );
        let hash = dhash160(&script);
        let p2sh = Builder::build_p2sh(&hash);
        let script_hash = electrum_script_hash(&p2sh);
        let selfi = self.clone();
        let fut = async move {
            match &selfi.rpc_client {
                UtxoRpcClientEnum::Electrum(client) => {
                    let history = try_s!(client.scripthash_get_history(&hex::encode(script_hash)).compat().await);
                    match history.first() {
                        Some(item) => {
                            let tx_bytes = try_s!(client.get_transaction_bytes(item.tx_hash.clone()).compat().await);
                            let tx: UtxoTx = try_s!(deserialize(tx_bytes.0.as_slice()).map_err(|e| ERRL!("{:?}", e)));
                            Ok(Some(tx.into()))
                        },
                        None => Ok(None),
                    }
                },
                UtxoRpcClientEnum::Native(client) => {
                    let target_addr = Address {
                        t_addr_prefix: selfi.p2sh_t_addr_prefix,
                        prefix: selfi.p2sh_addr_prefix,
                        hash,
                        checksum_type: selfi.checksum_type,
                    }.to_string();
                    let received_by_addr = try_s!(client.list_received_by_address(0, true, true).compat().await);
                    for item in received_by_addr {
                        if item.address == target_addr && !item.txids.is_empty() {
                            let tx_bytes = try_s!(client.get_transaction_bytes(item.txids[0].clone()).compat().await);
                            let tx: UtxoTx = try_s!(deserialize(tx_bytes.0.as_slice()).map_err(|e| ERRL!("{:?}", e)));
                            return Ok(Some(tx.into()))
                        }
                    }
                    Ok(None)
                },
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn search_for_swap_tx_spend_my(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        self.search_for_swap_tx_spend(
            time_lock,
            self.key_pair.public(),
            &try_s!(Public::from_slice(other_pub)),
            secret_hash,
            tx,
            search_from_block
        )
    }

    fn search_for_swap_tx_spend_other(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        self.search_for_swap_tx_spend(
            time_lock,
            &try_s!(Public::from_slice(other_pub)),
            self.key_pair.public(),
            secret_hash,
            tx,
            search_from_block
        )
    }
}

impl MarketCoinOps for UtxoCoin {
    fn ticker (&self) -> &str {&self.ticker[..]}

    fn my_address(&self) -> Cow<str> {
        self.0.my_address.to_string().into()
    }

    fn my_balance(&self) -> Box<dyn Future<Item=BigDecimal, Error=String> + Send> {
        Box::new(self.rpc_client.display_balance(self.my_address.clone(), self.decimals).map_err(|e| ERRL!("{}", e)))
    }

    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item=String, Error=String> + Send> {
        let bytes = try_fus!(hex::decode(tx));
        Box::new(self.rpc_client.send_raw_transaction(bytes.into()).map_err(|e| ERRL!("{}", e)).map(|hash| format!("{:?}", hash)))
    }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item=(), Error=String> + Send> {
        let tx: UtxoTx = try_fus!(deserialize(tx).map_err(|e| ERRL!("{:?}", e)));
        self.rpc_client.wait_for_confirmations(
            &tx,
            confirmations as u32,
            requires_nota,
            wait_until,
            check_every,
        )
    }

    fn wait_for_tx_spend(&self, tx_bytes: &[u8], wait_until: u64, from_block: u64) -> TransactionFut {
        let tx: UtxoTx = try_fus!(deserialize(tx_bytes).map_err(|e| ERRL!("{:?}", e)));
        let vout = 0;
        let client = self.rpc_client.clone();
        let fut = async move {
            loop {
                match client.find_output_spend(&tx, vout, from_block).compat().await {
                    Ok(Some(tx)) => return Ok(tx.into()),
                    Ok(None) => (),
                    Err(e) => {
                        log!("Error " (e) " on find_output_spend of tx " [e]);
                        ()
                    },
                };

                if now_ms() / 1000 > wait_until {
                    return ERR!("Waited too long until {} for transaction {:?} {} to be spent ", wait_until, tx, vout);
                }
                Timer::sleep(10.).await;
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> {
        let transaction: UtxoTx = try_s!(deserialize(bytes).map_err(|err| format!("{:?}", err)));
        Ok(transaction.into())
    }

    fn current_block(&self) -> Box<dyn Future<Item=u64, Error=String> + Send> {
        Box::new(self.rpc_client.get_block_count().map_err(|e| ERRL!("{}", e)))
    }

    fn address_from_pubkey_str(&self, pubkey: &str) -> Result<String, String> {
        let pubkey_bytes = try_s!(hex::decode(pubkey));
        let addr = try_s!(address_from_raw_pubkey(&pubkey_bytes, self.pub_addr_prefix, self.pub_t_addr_prefix, self.checksum_type));
        Ok(addr.to_string())
    }

    fn display_priv_key(&self) -> String {
        format!("{}", self.key_pair.private())
    }
}

async fn withdraw_impl(coin: UtxoCoin, req: WithdrawRequest) -> Result<TransactionDetails, String> {
    let to = try_s!(Address::from_str(&req.to));
    if to.checksum_type != coin.checksum_type {
        return ERR!("Address {} has invalid checksum type, it must be {:?}", to, coin.checksum_type);
    }
    let script_pubkey = if to.prefix == coin.pub_addr_prefix && to.t_addr_prefix == coin.pub_t_addr_prefix {
        Builder::build_p2pkh(&to.hash).to_bytes()
    } else if to.prefix == coin.p2sh_addr_prefix && to.t_addr_prefix == coin.p2sh_t_addr_prefix && coin.segwit {
        Builder::build_p2sh(&to.hash).to_bytes()
    } else {
        return ERR!("Address {} has invalid format", to);
    };
    let _utxo_lock = UTXO_LOCK.lock().await;
    let unspents = try_s!(coin.rpc_client.list_unspent_ordered(&coin.my_address).map_err(|e| ERRL!("{}", e)).compat().await);
    let (value, fee_policy) = if req.max {
        (unspents.iter().fold(0, |sum, unspent| sum + unspent.value), FeePolicy::DeductFromOutput(0))
    } else {
        (try_s!(sat_from_big_decimal(&req.amount, coin.decimals)), FeePolicy::SendExact)
    };
    let outputs = vec![TransactionOutput {
        value,
        script_pubkey,
    }];
    let fee = match req.fee {
        Some(WithdrawFee::UtxoFixed { amount }) => Some(ActualTxFee::Fixed(try_s!(sat_from_big_decimal(&amount, coin.decimals)))),
        Some(WithdrawFee::UtxoPerKbyte { amount }) => Some(ActualTxFee::Dynamic(try_s!(sat_from_big_decimal(&amount, coin.decimals)))),
        Some(_) => return ERR!("Unsupported input fee type"),
        None => None,
    };
    let (unsigned, data) = try_s!(coin.generate_transaction(unspents, outputs, fee_policy, fee).await);
    let prev_script = Builder::build_p2pkh(&coin.my_address.hash);
    let signed = try_s!(sign_tx(unsigned, &coin.key_pair, prev_script, coin.signature_version, coin.fork_id));
    let fee_details = UtxoFeeDetails {
        amount: big_decimal_from_sat(data.fee_amount as i64, coin.decimals),
    };
    Ok(TransactionDetails {
        from: vec![coin.my_address().into()],
        to: vec![format!("{}", to)],
        total_amount: big_decimal_from_sat(data.spent_by_me as i64, coin.decimals),
        spent_by_me: big_decimal_from_sat(data.spent_by_me as i64, coin.decimals),
        received_by_me: big_decimal_from_sat(data.received_by_me as i64, coin.decimals),
        my_balance_change: big_decimal_from_sat(data.received_by_me as i64 - data.spent_by_me as i64, coin.decimals),
        tx_hash: signed.hash().reversed().to_vec().into(),
        tx_hex: serialize(&signed).into(),
        fee_details: Some(fee_details.into()),
        block_height: 0,
        coin: coin.ticker.clone(),
        internal_id: vec![].into(),
        timestamp: now_ms() / 1000,
    })
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct UtxoFeeDetails {
    amount: BigDecimal,
}

impl MmCoin for UtxoCoin {
    fn is_asset_chain(&self) -> bool { self.asset_chain }

    fn check_i_have_enough_to_trade(&self, amount: &MmNumber, balance: &MmNumber, trade_info: TradeInfo) -> Box<dyn Future<Item=(), Error=String> + Send> {
        let arc = self.clone();
        let amount = amount.clone();
        let balance = balance.clone();
        let fee_fut = async move {
            let coin_fee = try_s!(arc.get_tx_fee().await);
            let fee = match coin_fee {
                ActualTxFee::Fixed(f) => f,
                ActualTxFee::Dynamic(f) => f,
            };
            let fee_decimal = MmNumber::from(fee) / MmNumber::from(10u64.pow(arc.decimals as u32));
            if &amount < &fee_decimal {
                return ERR!("Amount {} is too low, it'll result to dust error, at least {} is required", amount, fee_decimal);
            }
            let required = match trade_info {
                TradeInfo::Maker => amount + fee_decimal,
                TradeInfo::Taker(dex_fee) => &amount + &MmNumber::from(dex_fee.clone()) + MmNumber::from(2) * fee_decimal,
            };
            if balance < required {
                return ERR!("{} balance {} is too low, required {}", arc.ticker(), balance, required);
            }
            Ok(())
        };
        Box::new(fee_fut.boxed().compat())
    }

    fn can_i_spend_other_payment(&self) -> Box<dyn Future<Item=(), Error=String> + Send> {
        Box::new(futures01::future::ok(()))
    }

    fn withdraw(&self, req: WithdrawRequest) -> Box<dyn Future<Item=TransactionDetails, Error=String> + Send> {
        let fut = withdraw_impl(self.clone(), req);
        Box::new(fut.boxed().compat())
    }

    fn decimals(&self) -> u8 {
        self.decimals
    }

    fn process_history_loop(&self, ctx: MmArc) {
        const HISTORY_TOO_LARGE_ERR_CODE: i64 = -1;
        let history_too_large = json!({
            "code": 1,
            "message": "history too large"
        });
        let history = self.load_history_from_file(&ctx);
        let mut history_map: HashMap<H256Json, TransactionDetails> = history.into_iter().map(|tx| (H256Json::from(tx.tx_hash.as_slice()), tx)).collect();
        loop {
            if ctx.is_stopping() { break };
            {
                let coins_ctx = unwrap!(CoinsContext::from_ctx(&ctx));
                let coins = match coins_ctx.coins.spinlock (22) {
                    Ok (guard) => guard,
                    Err (_err) => {thread::sleep (Duration::from_millis (99)); continue}
                };
                if !coins.contains_key(&self.ticker) {
                    ctx.log.log("", &[&"tx_history", &self.ticker], "Loop stopped");
                    break
                };
            }

            let tx_ids: Vec<(H256Json, u64)> = match &self.rpc_client {
                UtxoRpcClientEnum::Native(client) => {
                    let mut from = 0;
                    let mut all_transactions = vec![];
                    loop {
                        let transactions = match client.list_transactions(100, from).wait() {
                            Ok(value) => value,
                            Err(e) => {
                                ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on list transactions, retrying", e));
                                thread::sleep(Duration::from_secs(10));
                                continue;
                            }
                        };
                        if transactions.is_empty() {
                            break;
                        }
                        from += 100;
                        all_transactions.extend(transactions);
                    }
                    all_transactions.into_iter().filter_map(|item| {
                        if item.address == self.my_address() {
                            Some((item.txid, item.blockindex))
                        } else {
                            None
                        }
                    }).collect()
                },
                UtxoRpcClientEnum::Electrum(client) => {
                    let script = Builder::build_p2pkh(&self.my_address.hash);
                    let script_hash = electrum_script_hash(&script);
                    let electrum_history = match client.scripthash_get_history(&hex::encode(script_hash)).wait() {
                        Ok(value) => value,
                        Err(e) => {
                            match &e.error {
                                JsonRpcErrorType::Transport(e) | JsonRpcErrorType::Parse(_, e) => {
                                    ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on scripthash_get_history, retrying", e));
                                    thread::sleep(Duration::from_secs(10));
                                    continue;
                                },
                                JsonRpcErrorType::Response(_addr, err) => {
                                    if *err == history_too_large {
                                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Got `history too large`, stopping further attempts to retrieve it"));
                                        *unwrap!(self.history_sync_state.lock()) = HistorySyncState::Error(json!({
                                            "code": HISTORY_TOO_LARGE_ERR_CODE,
                                            "message": "Got `history too large` error from Electrum server. History is not available",
                                        }));
                                        break;
                                    } else {
                                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {:?} on scripthash_get_history, retrying", e));
                                        thread::sleep(Duration::from_secs(10));
                                        continue;
                                    }
                                }
                            }
                        }
                    };
                    // electrum returns the most recent transactions in the end but we need to
                    // process them first so rev is required
                    electrum_history.into_iter().rev().map(|item| {
                        let height = if item.height < 0 {
                            0
                        } else {
                            item.height as u64
                        };
                        (item.tx_hash, height)
                    }).collect()
                }
            };
            let mut transactions_left = if tx_ids.len() > history_map.len() {
                *unwrap!(self.history_sync_state.lock()) = HistorySyncState::InProgress(json!({
                    "transactions_left": tx_ids.len() - history_map.len()
                }));
                tx_ids.len() - history_map.len()
            } else {
                *unwrap!(self.history_sync_state.lock()) = HistorySyncState::InProgress(json!({
                    "transactions_left": 0
                }));
                0
            };

            for (txid, height) in tx_ids {
                let mut updated = false;
                match history_map.entry(txid.clone()) {
                    Entry::Vacant(e) => {
                        match self.tx_details_by_hash(&txid.0).wait() {
                            Ok(mut tx_details) => {
                                if tx_details.block_height == 0 && height > 0 {
                                    tx_details.block_height = height;
                                }

                                e.insert(tx_details);
                                if transactions_left > 0 {
                                    transactions_left -= 1;
                                    *unwrap!(self.history_sync_state.lock()) = HistorySyncState::InProgress(json!({
                                    "transactions_left": transactions_left
                                }));
                                }
                                updated = true;
                            },
                            Err(e) => ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {:?} on getting the details of {:?}, skipping the tx", e, txid)),
                        }
                    },
                    Entry::Occupied(mut e) => {
                        // update block height for previously unconfirmed transaction
                        if (e.get().block_height == 0 || e.get().block_height == std::u64::MAX) && height > 0 {
                            e.get_mut().block_height = height;
                            updated = true;
                        }
                        if e.get().timestamp == 0 {
                            if let Ok(tx_details) = self.tx_details_by_hash(&txid.0).wait() {
                                e.get_mut().timestamp = tx_details.timestamp;
                                updated = true;
                            }
                        }
                    }
                }
                if updated {
                    let mut to_write: Vec<&TransactionDetails> = history_map.iter().map(|(_, value)| value).collect();
                    // the transactions with block_height == 0 are the most recent so we need to separately handle them while sorting
                    to_write.sort_unstable_by(|a, b| if a.block_height == 0 {
                        Ordering::Less
                    } else if b.block_height == 0 {
                        Ordering::Greater
                    } else {
                        b.block_height.cmp(&a.block_height)
                    });
                    self.save_history_to_file(&unwrap!(json::to_vec(&to_write)), &ctx);
                }
            }
            *unwrap!(self.history_sync_state.lock()) = HistorySyncState::Finished;
            thread::sleep(Duration::from_secs(30));
        }
    }

    fn tx_details_by_hash(&self, hash: &[u8]) -> Box<dyn Future<Item=TransactionDetails, Error=String> + Send> {
        let hash = H256Json::from(hash);
        let selfi = self.clone();
        let fut = async move {
            let verbose_tx = try_s!(selfi.rpc_client.get_verbose_transaction(hash).compat().await);
            let tx: UtxoTx = try_s!(deserialize(verbose_tx.hex.as_slice()).map_err(|e| ERRL!("{:?}", e)));
            let mut input_transactions: HashMap<&H256, UtxoTx> = HashMap::new();
            let mut input_amount = 0;
            let mut output_amount = 0;
            let mut from_addresses = vec![];
            let mut to_addresses = vec![];
            let mut spent_by_me = 0;
            let mut received_by_me = 0;
            for input in tx.inputs.iter() {
                // input transaction is zero if the tx is the coinbase transaction
                if input.previous_output.hash.is_zero() {
                    continue;
                }

                let input_tx = match input_transactions.entry(&input.previous_output.hash) {
                    Entry::Vacant(e) => {
                        let prev_hash = input.previous_output.hash.reversed();
                        let prev: BytesJson = try_s!(selfi.rpc_client.get_transaction_bytes(prev_hash.clone().into()).compat().await);
                        let prev_tx: UtxoTx = try_s!(deserialize(prev.as_slice()).map_err(|e| ERRL!("{:?}, tx: {:?}", e, prev_hash)));
                        e.insert(prev_tx)
                    },
                    Entry::Occupied(e) => e.into_mut(),
                };
                input_amount += input_tx.outputs[input.previous_output.index as usize].value;
                let from: Vec<Address> = try_s!(selfi.addresses_from_script(&input_tx.outputs[input.previous_output.index as usize].script_pubkey.clone().into()));
                if from.contains(&selfi.my_address) {
                    spent_by_me += input_tx.outputs[input.previous_output.index as usize].value;
                }
                from_addresses.push(from);
            };

            for output in tx.outputs.iter() {
                output_amount += output.value;
                let to = try_s!(selfi.addresses_from_script(&output.script_pubkey.clone().into()));
                if to.contains(&selfi.my_address) {
                    received_by_me += output.value;
                }
                to_addresses.push(to);
            }
            // remove address duplicates in case several inputs were spent from same address
            // or several outputs are sent to same address
            let mut from_addresses: Vec<String> = from_addresses.into_iter().flatten().map(|addr| addr.to_string()).collect();
            from_addresses.sort();
            from_addresses.dedup();
            let mut to_addresses: Vec<String> = to_addresses.into_iter().flatten().map(|addr| addr.to_string()).collect();
            to_addresses.sort();
            to_addresses.dedup();

            let fee = big_decimal_from_sat(input_amount as i64 - output_amount as i64, selfi.decimals);
            Ok(TransactionDetails {
                from: from_addresses,
                to: to_addresses,
                received_by_me: big_decimal_from_sat(received_by_me as i64, selfi.decimals),
                spent_by_me: big_decimal_from_sat(spent_by_me as i64, selfi.decimals),
                my_balance_change: big_decimal_from_sat(received_by_me as i64 - spent_by_me as i64, selfi.decimals),
                total_amount: big_decimal_from_sat(input_amount as i64, selfi.decimals),
                tx_hash: tx.hash().reversed().to_vec().into(),
                tx_hex: verbose_tx.hex,
                fee_details: Some(UtxoFeeDetails {
                    amount: fee,
                }.into()),
                block_height: verbose_tx.height,
                coin: selfi.ticker.clone(),
                internal_id: tx.hash().reversed().to_vec().into(),
                timestamp: verbose_tx.time.into(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn history_sync_status(&self) -> HistorySyncState {
        unwrap!(self.history_sync_state.lock()).clone()
    }

    fn get_trade_fee(&self) -> Box<dyn Future<Item=TradeFee, Error=String> + Send> {
        let ticker = self.ticker.clone();
        let decimals = self.decimals;
        let arc = self.clone();
        let fut = async move {
            let fee = try_s!(arc.get_tx_fee().await);
            let amount = match fee {
                ActualTxFee::Fixed(f) => f,
                ActualTxFee::Dynamic(f) => f,
            };
            Ok(TradeFee {
                coin: ticker,
                amount: big_decimal_from_sat(amount as i64, decimals),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn required_confirmations(&self) -> u64 {
        self.required_confirmations.load(AtomicOrderding::Relaxed)
    }

    fn requires_notarization(&self) -> bool { self.requires_notarization.load(AtomicOrderding::Relaxed) }

    fn set_required_confirmations(&self, confirmations: u64) {
        self.required_confirmations.store(confirmations, AtomicOrderding::Relaxed);
    }

    fn set_requires_notarization(&self, requires_nota: bool) {
        self.requires_notarization.store(requires_nota, AtomicOrderding::Relaxed);
    }
}

#[cfg(feature = "native")]
// https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.sh#L5
// https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.bat#L4
pub fn zcash_params_path() -> PathBuf {
    if cfg! (windows) {
        // >= Vista: c:\Users\$username\AppData\Roaming
        get_special_folder_path().join("ZcashParams")
    } else if cfg! (target_os = "macos") {
        unwrap!(home_dir()).join("Library").join("Application Support").join("ZcashParams")
    } else {
        unwrap!(home_dir()).join(".zcash-params")
    }
}

#[cfg(feature = "native")]
pub fn coin_daemon_data_dir(name: &str, is_asset_chain: bool) -> PathBuf {
    // komodo/util.cpp/GetDefaultDataDir
    let mut data_dir = match dirs::home_dir() {
        Some (hd) => hd,
        None => Path::new ("/") .to_path_buf()
    };

    if cfg! (windows) {
        // >= Vista: c:\Users\$username\AppData\Roaming
        data_dir = get_special_folder_path();
        if is_asset_chain {
            data_dir.push ("Komodo");
        } else {
            data_dir.push (first_char_to_upper(name));
        }
    } else if cfg! (target_os = "macos") {
        data_dir.push ("Library");
        data_dir.push ("Application Support");
        if is_asset_chain {
            data_dir.push ("Komodo");
        } else {
            data_dir.push (first_char_to_upper(name));
        }
    } else {
        if is_asset_chain {
            data_dir.push (".komodo");
        } else {
            data_dir.push (format!(".{}", name));
        }
    }

    if is_asset_chain {data_dir.push (name)};
    data_dir
}

#[cfg(not(feature = "native"))]
pub fn coin_daemon_data_dir(_name: &str, _is_asset_chain: bool) -> PathBuf {
    unimplemented!()
}

#[cfg(feature = "native")]
/// Returns a path to the native coin wallet configuration.
/// (This path is used in `LP_userpassfp` to read the wallet credentials).
/// cf. https://github.com/artemii235/SuperNET/issues/346
fn confpath (coins_en: &Json) -> Result<PathBuf, String> {
    // Documented at https://github.com/jl777/coins#bitcoin-protocol-specific-json
    // "USERHOME/" prefix should be replaced with the user's home folder.
    let confpathˢ = coins_en["confpath"].as_str().unwrap_or ("") .trim();
    if confpathˢ.is_empty() {
        let (name, is_asset_chain) = {
            match coins_en["asset"].as_str() {
                Some(a) => (a, true),
                None => (try_s!(coins_en["name"].as_str().ok_or("'name' field is not found in config")), false),
            }
        };

        let data_dir = coin_daemon_data_dir(name, is_asset_chain);

        let confname = format! ("{}.conf", name);

        return Ok (data_dir.join (&confname[..]))
    }
    let (confpathˢ, rel_to_home) =
        if confpathˢ.starts_with ("~/") {(&confpathˢ[2..], true)}
        else if confpathˢ.starts_with ("USERHOME/") {(&confpathˢ[9..], true)}
        else {(confpathˢ, false)};

    if rel_to_home {
        let home = try_s! (home_dir().ok_or ("Can not detect the user home directory"));
        Ok (home.join (confpathˢ))
    } else {
        Ok (confpathˢ.into())
    }
}

#[cfg(not(feature = "native"))]
fn confpath (_coins_en: &Json) -> Result<PathBuf, String> {unimplemented!()}

/// Attempts to parse native daemon conf file and return rpcport, rpcuser and rpcpassword
#[cfg(feature = "native")]
fn read_native_mode_conf(filename: &dyn AsRef<Path>) -> Result<(Option<u16>, String, String), String> {
    use ini::Ini;

    let conf: Ini = match Ini::load_from_file(&filename) {
        Ok(ini) => ini,
        Err(err) => return ERR!("Error parsing the native wallet configuration '{}': {}", filename.as_ref().display(), err)
    };
    let section = conf.general_section();
    let rpc_port = match section.get("rpcport") {
        Some(port) => port.parse::<u16>().ok(),
        None => None,
    };
    let rpc_user = try_s!(section.get("rpcuser").ok_or(ERRL!("Conf file {} doesn't have the rpcuser key", filename.as_ref().display())));
    let rpc_password = try_s!(section.get("rpcpassword").ok_or(ERRL!("Conf file {} doesn't have the rpcpassword key", filename.as_ref().display())));
    Ok((rpc_port, rpc_user.clone(), rpc_password.clone()))
}

#[cfg(not(feature = "native"))]
fn read_native_mode_conf(_filename: &dyn AsRef<Path>) -> Result<(Option<u16>, String, String), String> {
    unimplemented!()
}

pub async fn utxo_coin_from_conf_and_request(
    ticker: &str,
    conf: &Json,
    req: &Json,
    priv_key: &[u8],
) -> Result<UtxoCoin, String> {
    let checksum_type = if ticker == "GRS" {
        ChecksumType::DGROESTL512
    } else if ticker == "SMART" {
        ChecksumType::KECCAK256
    } else {
        ChecksumType::DSHA256
    };

    let pub_addr_prefix = conf["pubtype"].as_u64().unwrap_or (if ticker == "BTC" {0} else {60}) as u8;
    let wif_prefix = conf["wiftype"].as_u64().unwrap_or (if ticker == "BTC" {128} else {188}) as u8;

    let private = Private {
        prefix: wif_prefix,
        secret: H256::from(priv_key),
        compressed: true,
        checksum_type,
    };

    let key_pair = try_s!(KeyPair::from_private(private));
    let my_address = Address {
        prefix: pub_addr_prefix,
        t_addr_prefix: conf["taddr"].as_u64().unwrap_or (0) as u8,
        hash: key_pair.public().address_hash(),
        checksum_type,
    };

    let rpc_client = match req["method"].as_str() {
        Some("enable") => {
            if cfg!(feature = "native") {
                let native_conf_path = try_s!(confpath(conf));
                let (rpc_port, rpc_user, rpc_password) = try_s!(read_native_mode_conf(&native_conf_path));
                let auth_str = fomat!((rpc_user)":"(rpc_password));
                let rpc_port = match rpc_port {
                    Some(p) => p,
                    None => try_s!(conf["rpcport"].as_u64().ok_or(ERRL!("Rpc port is not set neither in `coins` file nor in native daemon config"))) as u16,
                };
                let client = Arc::new(NativeClientImpl {
                    coin_ticker: ticker.to_string(),
                    uri: fomat!("http://127.0.0.1:"(rpc_port)),
                    auth: format!("Basic {}", base64_encode(&auth_str, URL_SAFE)),
                });

                UtxoRpcClientEnum::Native(NativeClient(client))
            } else {
                return ERR!("Native UTXO mode is not available in non-native build");
            }
        },
        Some("electrum") => {
            let mut servers: Vec<ElectrumRpcRequest> = try_s!(json::from_value(req["servers"].clone()));
            let mut rng = small_rng();
            servers.as_mut_slice().shuffle(&mut rng);
            let mut client = ElectrumClientImpl::new(ticker.to_string());
            for server in servers.iter() {
                match client.add_server(server) {
                    Ok(_) => (),
                    Err(e) => log!("Error " (e) " connecting to " [server] ". Address won't be used")
                };
            }

            let mut attempts = 0;
            while !client.is_connected().await {
                if attempts >= 10 {
                    return ERR!("Failed to connect to at least 1 of {:?} in 5 seconds.", servers);
                }

                Timer::sleep(0.5).await;
                attempts += 1;
            }

            let client = Arc::new(client);
            // ping the electrum servers every 30 seconds to prevent them from disconnecting us.
            // according to docs server can do it if there are no messages in ~10 minutes.
            // https://electrumx.readthedocs.io/en/latest/protocol-methods.html?highlight=keep#server-ping
            // weak reference will allow to stop the thread if client is dropped
            let weak_client = Arc::downgrade(&client);
            spawn(async move {
                loop {
                    if let Some(client) = weak_client.upgrade() {
                        if let Err(e) = ElectrumClient(client).server_ping().compat().await {
                            log!("Electrum servers " [servers] " ping error " [e]);
                        }
                    } else {
                        log!("Electrum servers " [servers] " ping loop stopped");
                        break;
                    }
                    Timer::sleep(30.).await
                }
            });
            UtxoRpcClientEnum::Electrum(ElectrumClient(client))
        },
        _ => return ERR!("utxo_coin_from_conf_and_request should be called only by enable or electrum requests"),
    };
    let asset_chain = conf["asset"].as_str().is_some();
    let tx_version = conf["txversion"].as_i64().unwrap_or (1) as i32;
    let overwintered = conf["overwintered"].as_u64().unwrap_or (0) == 1;

    let tx_fee = match conf["txfee"].as_u64() {
        None => TxFee::Fixed(1000),
        Some (0) => {
            let fee_method = match &rpc_client {
                UtxoRpcClientEnum::Electrum(_) => EstimateFeeMethod::Standard,
                UtxoRpcClientEnum::Native(client) => try_s!(client.detect_fee_method().compat().await)
            };
            TxFee::Dynamic(fee_method)
        },
        Some (fee) => TxFee::Fixed(fee),
    };
    let version_group_id = match conf["version_group_id"].as_str() {
        Some(mut s) => {
            if s.starts_with("0x") {
                s = &s[2..];
            }
            let bytes = try_s!(hex::decode(s));
            u32::from_be_bytes(try_s!(bytes.as_slice().try_into()))
        },
        None => if tx_version == 3 && overwintered {
            0x03c48270
        } else if tx_version == 4 && overwintered {
            0x892f2085
        } else {
            0
        }
    };

    let consensus_branch_id = match conf["consensus_branch_id"].as_str() {
        Some(mut s) => {
            if s.starts_with("0x") {
                s = &s[2..];
            }
            let bytes = try_s!(hex::decode(s));
            u32::from_be_bytes(try_s!(bytes.as_slice().try_into()))
        },
        None => {
            match tx_version {
                3 => 0x5ba81b19,
                4 => 0x76b809bb,
                _ => 0,
            }
        },
    };

    let decimals = conf["decimals"].as_u64().unwrap_or (8) as u8;

    let (signature_version, fork_id) = if ticker == "BCH" {
        (SignatureVersion::ForkId, 0x40)
    } else {
        (SignatureVersion::Base, 0)
    };
    // should be sufficient to detect zcash by overwintered flag
    let zcash = overwintered;

    let initial_history_state = if req["tx_history"].as_bool().unwrap_or(false) {
        HistorySyncState::NotStarted
    } else {
        HistorySyncState::NotEnabled
    };

    // param from request should override the config
    let required_confirmations = req["required_confirmations"].as_u64().unwrap_or(
        conf["required_confirmations"].as_u64().unwrap_or(1)
    );
    let requires_notarization = req["requires_notarization"].as_bool().unwrap_or(
        conf["requires_notarization"].as_bool().unwrap_or(false)
    ).into();

    let coin = UtxoCoinImpl {
        ticker: ticker.into(),
        decimals,
        rpc_client,
        key_pair,
        is_pos: conf["isPoS"].as_u64() == Some(1),
        requires_notarization,
        overwintered,
        pub_addr_prefix,
        p2sh_addr_prefix: conf["p2shtype"].as_u64().unwrap_or (if ticker == "BTC" {5} else {85}) as u8,
        pub_t_addr_prefix: conf["taddr"].as_u64().unwrap_or (0) as u8,
        p2sh_t_addr_prefix: conf["taddr"].as_u64().unwrap_or (0) as u8,
        segwit: conf["segwit"].as_bool().unwrap_or (false),
        wif_prefix,
        tx_version,
        my_address: my_address.clone(),
        asset_chain,
        tx_fee,
        version_group_id,
        consensus_branch_id,
        zcash,
        checksum_type,
        signature_version,
        fork_id,
        history_sync_state: Mutex::new(initial_history_state),
        required_confirmations: required_confirmations.into(),
        force_min_relay_fee: conf["force_min_relay_fee"].as_bool().unwrap_or (false),
    };
    Ok(UtxoCoin(Arc::new(coin)))
}

/// Function calculating KMD interest
/// https://komodoplatform.atlassian.net/wiki/spaces/KPSD/pages/71729215/What+is+the+5+Komodo+Stake+Reward
/// https://github.com/KomodoPlatform/komodo/blob/master/src/komodo_interest.h
fn kmd_interest(height: u64, value: u64, lock_time: u64, current_time: u64) -> u64 {
    const KOMODO_ENDOFERA: u64 = 7777777;
    const LOCKTIME_THRESHOLD: u64 = 500000000;
    // value must be at least 10 KMD
    if value < 1000000000 { return 0; }
    // interest will stop accrue after block 7777777
    if height >= KOMODO_ENDOFERA { return 0 };
    // interest doesn't accrue for lock_time < 500000000
    if lock_time < LOCKTIME_THRESHOLD { return 0; }
    // current time must be greater than tx lock_time
    if current_time < lock_time { return 0; }

    let mut minutes = (current_time - lock_time) / 60;
    // at least 1 hour should pass
    if minutes < 60 { return 0; }
    // interest stop accruing after 1 year before block 1000000
    if minutes > 365 * 24 * 60 { minutes = 365 * 24 * 60 };
    // interest stop accruing after 1 month past 1000000 block
    if height >= 1000000 && minutes > 31 * 24 * 60 { minutes = 31 * 24 * 60; }
    // next 2 lines ported as is from Komodo codebase
    minutes -= 59;
    (value / 10512000) * minutes
}

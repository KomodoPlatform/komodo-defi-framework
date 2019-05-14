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
pub mod rpc_clients;

use base64::{encode_config as base64_encode, URL_SAFE};
pub use bitcrypto::{dhash160, ChecksumType};
use byteorder::{LittleEndian, WriteBytesExt};
use chain::{TransactionOutput, TransactionInput, OutPoint};
use chain::constants::{SEQUENCE_FINAL};
use common::{dstr, lp, MutexGuardWrapper};
use common::custom_futures::join_all_sequential;
use common::mm_ctx::MmArc;
use futures::{Future};
use gstuff::{now_ms};
use hashbrown::hash_map::{HashMap, Entry};
use keys::{Error as KeysError, KeyPair, Private, Public, Address, Secret, Type};
use keys::bytes::Bytes;
use keys::generator::{Random, Generator};
use primitives::hash::{H256, H264, H512};
use rand::{thread_rng};
use rand::seq::SliceRandom;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use script::{Opcode, Builder, Script, ScriptAddress, TransactionInputSigner, UnsignedTransactionInput, SignatureVersion};
use serde_json::{self as json, Value as Json};
use serialization::{serialize, deserialize};
use std::borrow::Cow;
use std::ffi::CStr;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub use chain::Transaction as UtxoTx;

use self::rpc_clients::{UtxoRpcClientEnum, UnspentInfo, ElectrumClient, ElectrumClientImpl, NativeClient, electrum_script_hash};
use super::{IguanaInfo, MarketCoinOps, MmCoin, MmCoinEnum, SwapOps, Transaction, TransactionEnum, TransactionFut, TransactionDetails};
use crate::utxo::rpc_clients::{NativeClientImpl, UtxoRpcClientOps};
use std::cmp::Ordering;
use bitcrypto::sha256;

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

    fn amount(&self, decimals: u8) -> Result<f64, String> { Ok(0.) }

    fn to(&self) -> Vec<String> { vec!["".into()] }

    fn from(&self) -> Vec<String> { vec!["".into()] }

    fn fee_details(&self) -> Result<Json, String> { Ok(Json::Null) }
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
    Dynamic,
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
    /// If true - use Segwit protocol
    /// https://en.bitcoin.it/wiki/Segregated_Witness
    segwit: bool,
    /// Default decimals amount is 8 (BTC and almost all other UTXO coins)
    /// But there are forks which have different decimals:
    /// Peercoin has 6
    /// Emercoin has 6
    /// Bitcoin Diamond has 7
    decimals: u8,
    /// Is coin protected by Komodo dPoW?
    /// https://komodoplatform.com/security-delayed-proof-of-work-dpow/
    notarized: bool,
    /// The local RPC port of the coin wallet.  
    /// Fetched from the wallet config when we can find it.
    rpc_port: u16,
    /// RPC username
    rpc_user: String,
    /// RPC password
    rpc_password: String,
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
    /// Version group id for Zcash transactions since Overwinter: https://github.com/zcash/zips/blob/master/zip-0202.rst
    version_group_id: u32,
    /// Defines if coin uses Zcash transaction format
    zcash: bool,
    /// Address and privkey checksum type
    checksum_type: ChecksumType,
}

impl UtxoCoinImpl {
    fn get_tx_fee(&self) -> Box<Future<Item=ActualTxFee, Error=String> + Send> {
        match self.tx_fee {
            TxFee::Fixed(fee) => Box::new(futures::future::ok(ActualTxFee::Fixed(fee))),
            TxFee::Dynamic => Box::new(self.rpc_client.estimate_fee_sat(self.decimals).map(|fee| ActualTxFee::Dynamic(fee))),
        }
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
}

fn payment_script(
    time_lock: u32,
    secret_hash: &[u8],
    pub_0: &Public,
    pub_1: &Public
) -> Result<Script, String> {
    let builder = Builder::default();
    let mut wtr = vec![];
    try_s!(wtr.write_u32::<LittleEndian>(time_lock));
    Ok(builder
        .push_opcode(Opcode::OP_IF)
        .push_bytes(&wtr)
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
        .into_script())
}

fn script_sig(message: &H256, key_pair: &KeyPair) -> Result<Bytes, String> {
    let signature = try_s!(key_pair.private().sign(message));

    let mut sig_script = Bytes::default();
    sig_script.append(&mut Bytes::from((*signature).to_vec()));
    // Using SIGHASH_ALL only for now
    sig_script.append(&mut Bytes::from(vec![1]));

    Ok(sig_script)
}

fn script_sig_with_pub(message: &H256, key_pair: &KeyPair) -> Result<Bytes, String> {
    let sig_script = try_s!(script_sig(message, key_pair));

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
    prev_script: &Script
) -> Result<TransactionInput, String> {
    let script = Builder::build_p2pkh(&key_pair.public().address_hash());
    if script != *prev_script {
        return ERR!("p2pkh script {} built from input key pair doesn't match expected prev script {}", script, prev_script);
    }

    let sighash = signer.signature_hash(input_index, 0, &script, SignatureVersion::Base, 1);

    let script_sig = try_s!(script_sig_with_pub(&sighash, key_pair));

    Ok(TransactionInput {
        script_sig,
        sequence: signer.inputs[input_index].sequence,
        script_witness: vec![],
        previous_output: signer.inputs[input_index].previous_output.clone()
    })
}

/// Creates signed input spending p2sh output
fn p2sh_spend(
    signer: &TransactionInputSigner,
    input_index: usize,
    key_pair: &KeyPair,
    script_data: Script,
    redeem_script: Script,
) -> Result<TransactionInput, String> {
    let sighash = signer.signature_hash(input_index, 0, &redeem_script, SignatureVersion::Base, 1);

    let sig = try_s!(script_sig(&sighash, &key_pair));

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

fn p2sh_spending_tx(
    prev_transaction: UtxoTx,
    redeem_script: Bytes,
    outputs: Vec<TransactionOutput>,
    script_data: Script,
    key_pair: &KeyPair,
    version: i32,
    overwintered: bool,
    sequence: u32,
    version_group_id: u32,
    zcash: bool,
) -> Result<UtxoTx, String> {
    let lock_time = (now_ms() / 1000) as u32;
    let unsigned = TransactionInputSigner {
        lock_time,
        version,
        overwintered,
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
        version_group_id,
        zcash,
    };
    let signed_input = try_s!(
        p2sh_spend(&unsigned, 0, key_pair, script_data, redeem_script.into())
    );
    Ok(UtxoTx {
        version: unsigned.version,
        overwintered: unsigned.overwintered,
        lock_time: unsigned.lock_time,
        inputs: vec![signed_input],
        outputs,
        expiry_height: unsigned.expiry_height,
        join_splits: vec![],
        shielded_spends: vec![],
        shielded_outputs: vec![],
        value_balance: 0,
        version_group_id: unsigned.version_group_id,
        binding_sig: H512::default(),
        join_split_sig: H512::default(),
        join_split_pubkey: H256::default(),
        zcash,
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
    prev_script: Script
) -> Result<UtxoTx, String> {
    let mut signed_inputs = vec![];
    for (i, _) in unsigned.inputs.iter().enumerate() {
        signed_inputs.push(
            try_s!(p2pkh_spend(&unsigned, i, key_pair, &prev_script))
        );
    }
    Ok(UtxoTx {
        inputs: signed_inputs,
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
    })
}

/// MM2 uses satoshis with 8 decimals as amounts, but some UTXO coins have less than 8 decimals.
/// Have not seen UTXO coins with more than 8 decimals but it's ok to handle this too just in case.
fn adjust_sat_by_decimals(satoshis: u64, decimals: u8) -> u64 {
    if decimals < 8 {
        satoshis / 10_u64.pow(8 - decimals as u32)
    } else if decimals > 8 {
        satoshis * 10_u64.pow(decimals as u32 - 8)
    } else {
        satoshis
    }
}

#[derive(Clone, Debug)]
pub struct UtxoCoin(Arc<UtxoCoinImpl>);
impl Deref for UtxoCoin {type Target = UtxoCoinImpl; fn deref (&self) -> &UtxoCoinImpl {&*self.0}}

// We can use a shared UTXO lock for all UTXO coins at 1 time.
// It's highly likely that we won't experience any issues with it as we won't need to send "a lot" of transactions concurrently.
lazy_static! {static ref UTXO_LOCK: Mutex<()> = Mutex::new(());}

macro_rules! true_or_err {
    ($cond: expr, $msg: expr $(, $args:ident)*) => {
        if !$cond {
            return Box::new(futures::future::err(ERRL!($msg $(, $args)*)));
        }
    };
}

impl UtxoCoin {
    fn send_outputs_from_my_address(&self, outputs: Vec<TransactionOutput>) -> TransactionFut {
        let arc = self.clone();
        let utxo_lock = MutexGuardWrapper(try_fus!(UTXO_LOCK.lock()));
        let unspent_fut = self.rpc_client.list_unspent_ordered(&self.my_address);
        Box::new(unspent_fut.and_then(move |unspents| {
            arc.generate_transaction(
                unspents,
                outputs,
                FeePolicy::SendExact,
            ).and_then(move |(unsigned, _)| -> TransactionFut {
                let prev_script = Builder::build_p2pkh(&arc.my_address.hash);
                let signed = try_fus!(sign_tx(unsigned, &arc.key_pair, prev_script));
                Box::new(arc.rpc_client.send_transaction(&signed, arc.my_address.clone()).then(move |res| {
                    // Drop the UTXO lock only when the transaction send result is known.
                    drop(utxo_lock);
                    try_s!(res);
                    Ok(signed.into())
                }))
            })
        }))
    }

    fn validate_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        first_pub0: &Public,
        second_pub0: &Public,
        priv_bn_hash: &[u8],
        amount: u64,
    ) -> Result<(), String> {
        let tx: UtxoTx = try_s!(deserialize(payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let amount = adjust_sat_by_decimals(amount, self.decimals);

        let mut attempts = 0;
        loop {
            let tx_from_rpc = match self.rpc_client.get_transaction_bytes(tx.hash().reversed().into()).wait() {
                Ok(t) => t,
                Err(e) => {
                    if attempts > 2 {
                        return ERR!("Got error {:?} after 3 attempts of getting tx {:?} from RPC", e, tx.tx_hash());
                    };
                    attempts += 1;
                    log!("Error " [e] " getting the tx " [tx.tx_hash()] " from rpc");
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };
            if serialize(&tx).take() != tx_from_rpc.0 {
                return ERR!("Provided payment tx {:?} doesn't match tx data from rpc {:?}", tx, tx_from_rpc);
            }

            let expected_redeem = try_s!(payment_script(
                time_lock,
                priv_bn_hash,
                &try_s!(Public::from_slice(first_pub0)),
                &try_s!(Public::from_slice(second_pub0)),
            ));

            let expected_output = TransactionOutput {
                value: amount,
                script_pubkey: Builder::build_p2sh(&dhash160(&expected_redeem)).into(),
            };

            if tx.outputs[0] != expected_output {
                return ERR!("Provided payment tx output doesn't match expected {:?} {:?}", tx.outputs[0], expected_output);
            }
            return Ok(());
        }
    }

    /// Generates unsigned transaction (TransactionInputSigner) from specified utxos and outputs.
    /// This function expects that utxos are sorted by amounts in ascending order
    /// Consider sorting before calling this function
    /// Sends the change (inputs amount - outputs amount) to "my_address"
    /// Also returns the resulting transaction fee in satoshis
    fn generate_transaction(
        &self,
        utxos: Vec<UnspentInfo>,
        mut outputs: Vec<TransactionOutput>,
        fee_policy: FeePolicy
    ) -> Box<Future<Item=(TransactionInputSigner, AdditionalTxData), Error=String> + Send> {
        const DUST: u64 = 1000;
        let lock_time = (now_ms() / 1000) as u32;
        let change_script_pubkey = Builder::build_p2pkh(&self.my_address.hash).to_bytes();
        let arc = self.clone();
        Box::new(self.get_tx_fee().and_then(move |coin_tx_fee| -> Box<Future<Item=(TransactionInputSigner, AdditionalTxData), Error=String> + Send> {
            true_or_err!(!utxos.is_empty(), "Couldn't generate tx from empty utxos set");
            true_or_err!(!outputs.is_empty(), "Couldn't generate tx from empty outputs set");
            let mut tx_fee = match &coin_tx_fee {
                ActualTxFee::Fixed(f) => *f,
                ActualTxFee::Dynamic(f) => (f * 20) / 1024, // every tx has version, locktime and maybe other fields depending on coin, using 20 bytes as default value for this
            };

            let mut target_value = 0;
            let mut received_by_me = 0;
            for output in outputs.iter() {
                let value = output.value;
                true_or_err!(value >= DUST, "Output value {} is less than dust amount {}", value, DUST);
                target_value += value;
                if output.script_pubkey == change_script_pubkey {
                    received_by_me += value;
                }
                if let ActualTxFee::Dynamic(ref fee_per_kb) = coin_tx_fee {
                    // output size: 8 byte (value) + 1 byte (script_pubkey length) + script_pubkey length
                    tx_fee += (fee_per_kb * (8 + 1 + output.script_pubkey.len() as u64)) / 1024;
                }
            }

            true_or_err!(target_value > 0, "Total target value calculated from outputs {:?} is zero", outputs);
            let mut value_to_spend = 0;
            let mut inputs = vec![];
            for utxo in utxos.iter() {
                value_to_spend += utxo.value;
                inputs.push(UnsignedTransactionInput {
                    previous_output: utxo.outpoint.clone(),
                    sequence: SEQUENCE_FINAL,
                    amount: utxo.value,
                });
                if let ActualTxFee::Dynamic(ref fee_per_kb) = coin_tx_fee {
                    // final input size: 1 byte (signature length) + 72 bytes (signature + sighash with length)
                    // + 34 bytes (pubkey with length) + 32 bytes (prev_out hash) + 4 bytes (prev_out index)
                    tx_fee += (fee_per_kb * (1 + 72 + 34 + 32 + 4)) / 1024;
                }
                let target = match fee_policy {
                    FeePolicy::SendExact => target_value + tx_fee,
                    FeePolicy::DeductFromOutput(_) => target_value,
                };
                if value_to_spend >= target { break; }
            }
            match fee_policy {
                FeePolicy::SendExact => target_value += tx_fee,
                FeePolicy::DeductFromOutput(i) => {
                    let min_output = tx_fee + DUST;
                    let val = outputs[i].value;
                    true_or_err!(val >= min_output, "Output {} value {} is too small, required no less than {}", i, val, min_output);
                    outputs[i].value -= tx_fee;
                    if outputs[i].script_pubkey == change_script_pubkey {
                        received_by_me -= tx_fee;
                    }
                },
            };
            true_or_err!(value_to_spend >= target_value, "Not sufficient balance. Couldn't collect enough value from utxos {:?} to create tx with outputs {:?}", utxos, outputs);

            let change = value_to_spend - target_value;
            if change >= DUST {
                outputs.push({
                    TransactionOutput {
                        value: change,
                        script_pubkey: change_script_pubkey.clone()
                    }
                });
                received_by_me += change;
            } else {
                tx_fee += change;
            }

            let tx = TransactionInputSigner {
                inputs,
                outputs,
                lock_time,
                version: arc.tx_version,
                overwintered: arc.overwintered,
                expiry_height: 0,
                join_splits: vec![],
                shielded_spends: vec![],
                shielded_outputs: vec![],
                value_balance: 0,
                version_group_id: arc.version_group_id,
                zcash: arc.zcash,
            };

            let data = AdditionalTxData {
                fee_amount: tx_fee,
                received_by_me,
                spent_by_me: value_to_spend,
            };
            arc.calc_interest_if_required(tx, data, change_script_pubkey)
        }))
    }

    /// Calculates interest if the coin is KMD
    /// Adds the value to existing output to my_script_pub or creates additional interest output
    /// returns transaction and data as is if the coin is not KMD
    fn calc_interest_if_required(
        &self,
        mut unsigned: TransactionInputSigner,
        mut data: AdditionalTxData,
        my_script_pub: Bytes,
    ) -> Box<Future<Item=(TransactionInputSigner, AdditionalTxData), Error=String> + Send> {
        if self.ticker != "KMD" {
            return Box::new(futures::future::ok((unsigned, data)));
        }
        unsigned.lock_time = (now_ms() / 1000) as u32 - 777;
        let prev_tx_futures: Vec<_> = unsigned.inputs
            .iter()
            .map(|input| self.rpc_client.get_verbose_transaction(input.previous_output.hash.reversed().into()))
            .collect();

        Box::new(join_all_sequential(prev_tx_futures).map(move |prev_transactions| {
            let mut interest = 0;
            let inputs_and_tx = unsigned.inputs.iter().zip(prev_transactions.iter());
            inputs_and_tx.for_each(|(input, tx)| {
                interest += kmd_interest(tx.height, input.amount, tx.locktime as u64, unsigned.lock_time as u64);
            });
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
                unsigned.lock_time = (now_ms() / 1000) as u32 - 3000;
            }
            (unsigned, data)
        }))
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
    fn send_taker_fee(&self, fee_pub_key: &[u8], amount: u64) -> TransactionFut {
        let address = try_fus!(address_from_raw_pubkey(fee_pub_key, self.pub_addr_prefix, self.pub_t_addr_prefix, self.checksum_type));
        let amount = adjust_sat_by_decimals(amount, self.decimals);
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
        amount: u64,
    ) -> TransactionFut {
        let redeem_script = try_fus!(payment_script(
            time_lock,
            secret_hash,
            self.key_pair.public(),
            &try_fus!(Public::from_slice(taker_pub)),
        ));
        let amount = adjust_sat_by_decimals(amount, self.decimals);
        let output = TransactionOutput {
            value: amount,
            script_pubkey: Builder::build_p2sh(&dhash160(&redeem_script)).into(),
        };
        self.send_outputs_from_my_address(vec![output])
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: u64,
    ) -> TransactionFut {
        let redeem_script = try_fus!(payment_script(
            time_lock,
            priv_bn_hash,
            self.key_pair.public(),
            &try_fus!(Public::from_slice(maker_pub)),
        ));

        let amount = adjust_sat_by_decimals(amount, self.decimals);

        let output = TransactionOutput {
            value: amount,
            script_pubkey: Builder::build_p2sh(&dhash160(&redeem_script)).into(),
        };
        self.send_outputs_from_my_address(vec![output])
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
        let redeem_script = try_fus!(
            payment_script(time_lock, &*dhash160(secret), &try_fus!(Public::from_slice(taker_pub)), self.key_pair.public())
        );
        let arc = self.clone();
        Box::new(self.get_tx_fee().and_then(move |coin_fee| -> TransactionFut {
            let fee = match coin_fee {
                ActualTxFee::Fixed(fee) => fee,
                // atomic swap payment spend transaction is ~300 bytes in average as of now
                ActualTxFee::Dynamic(fee_per_kb) => (fee_per_kb * 300) / 1024,
            };

            let output = TransactionOutput {
                value: prev_tx.outputs[0].value - fee,
                script_pubkey: Builder::build_p2pkh(&arc.key_pair.public().address_hash()).to_bytes()
            };
            let transaction = try_fus!(p2sh_spending_tx(
                prev_tx,
                redeem_script.into(),
                vec![output],
                script_data,
                &arc.key_pair,
                arc.tx_version,
                arc.overwintered,
                SEQUENCE_FINAL,
                arc.version_group_id,
                arc.zcash,
            ));
            Box::new(arc.rpc_client.send_transaction(&transaction, arc.my_address.clone()).map(move |_res|
                transaction.into()
            ))
        }))
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
        let redeem_script = try_fus!(
            payment_script(time_lock, &*dhash160(secret), &try_fus!(Public::from_slice(maker_pub)), self.key_pair.public())
        );
        let arc = self.clone();
        Box::new(self.get_tx_fee().and_then(move |coin_fee| -> TransactionFut {
            let fee = match coin_fee {
                ActualTxFee::Fixed(fee) => fee,
                // atomic swap payment spend transaction is ~300 bytes in average as of now
                ActualTxFee::Dynamic(fee_per_kb) => (fee_per_kb * 300) / 1024,
            };

            let output = TransactionOutput {
                value: prev_tx.outputs[0].value - fee,
                script_pubkey: Builder::build_p2pkh(&arc.key_pair.public().address_hash()).to_bytes()
            };
            let transaction = try_fus!(p2sh_spending_tx(
                prev_tx,
                redeem_script.into(),
                vec![output],
                script_data,
                &arc.key_pair,
                arc.tx_version,
                arc.overwintered,
                SEQUENCE_FINAL,
                arc.version_group_id,
                arc.zcash,
            ));
            Box::new(arc.rpc_client.send_transaction(&transaction, arc.my_address.clone()).map(move |_res|
                transaction.into()
            ))
        }))
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
        let redeem_script = try_fus!(
            payment_script(time_lock, secret_hash, self.key_pair.public(), &try_fus!(Public::from_slice(maker_pub)))
        );
        let arc = self.clone();
        Box::new(self.get_tx_fee().and_then(move |coin_fee| -> TransactionFut {
            let fee = match coin_fee {
                ActualTxFee::Fixed(fee) => fee,
                // atomic swap payment spend transaction is ~300 bytes in average as of now
                ActualTxFee::Dynamic(fee_per_kb) => (fee_per_kb * 300) / 1024,
            };

            let output = TransactionOutput {
                value: prev_tx.outputs[0].value - fee,
                script_pubkey: Builder::build_p2pkh(&arc.key_pair.public().address_hash()).to_bytes()
            };
            let transaction = try_fus!(p2sh_spending_tx(
                prev_tx,
                redeem_script.into(),
                vec![output],
                script_data,
                &arc.key_pair,
                arc.tx_version,
                arc.overwintered,
                SEQUENCE_FINAL - 1,
                arc.version_group_id,
                arc.zcash,
            ));
            Box::new(arc.rpc_client.send_transaction(&transaction, arc.my_address.clone()).map(move |_res|
                transaction.into()
            ))
        }))
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
        let redeem_script = try_fus!(payment_script(
            time_lock,
            secret_hash,
            self.key_pair.public(),
            &try_fus!(Public::from_slice(taker_pub)),
        ));
        let arc = self.clone();
        Box::new(self.get_tx_fee().and_then(move |coin_fee| -> TransactionFut {
            let fee = match coin_fee {
                ActualTxFee::Fixed(fee) => fee,
                // atomic swap payment spend transaction is ~300 bytes in average as of now
                ActualTxFee::Dynamic(fee_per_kb) => (fee_per_kb * 300) / 1024,
            };

            let output = TransactionOutput {
                value: prev_tx.outputs[0].value - fee,
                script_pubkey: Builder::build_p2pkh(&arc.key_pair.public().address_hash()).to_bytes()
            };
            let transaction = try_fus!(p2sh_spending_tx(
                prev_tx,
                redeem_script.into(),
                vec![output],
                script_data,
                &arc.key_pair,
                arc.tx_version,
                arc.overwintered,
                SEQUENCE_FINAL - 1,
                arc.version_group_id,
                arc.zcash,
            ));
            Box::new(arc.rpc_client.send_transaction(&transaction, arc.my_address.clone()).map(move |_res|
                transaction.into()
            ))
        }))
    }

    fn validate_fee(
        &self,
        fee_tx: TransactionEnum,
        fee_addr: &[u8],
        amount: u64
    ) -> Result<(), String> {
        let tx = match fee_tx {
            TransactionEnum::UtxoTx(tx) => tx,
            _ => panic!(),
        };
        let amount = adjust_sat_by_decimals(amount, self.decimals);
        let tx_from_rpc = try_s!(self.rpc_client.get_transaction_bytes(tx.hash().reversed().into()).wait());

        if tx_from_rpc.0 != serialize(&tx).take() {
            return ERR!("Provided dex fee tx {:?} doesn't match tx data from rpc {:?}", tx, tx_from_rpc);
        }

        let address = try_s!(address_from_raw_pubkey(fee_addr, self.pub_addr_prefix, self.pub_t_addr_prefix, self.checksum_type));
        let expected_output = TransactionOutput {
            value: amount,
            script_pubkey: Builder::build_p2pkh(&address.hash).to_bytes()
        };

        if tx.outputs[0] != expected_output {
            return ERR!("Provided dex fee tx output doesn't match expected {:?} {:?}", tx.outputs[0], expected_output);
        }
        Ok(())
    }

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: u64,
    ) -> Result<(), String> {
        self.validate_payment(
            payment_tx,
            time_lock,
            &try_s!(Public::from_slice(maker_pub)),
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
        amount: u64,
    ) -> Result<(), String> {
        self.validate_payment(
            payment_tx,
            time_lock,
            &try_s!(Public::from_slice(taker_pub)),
            self.key_pair.public(),
            priv_bn_hash,
            amount
        )
    }
}

impl MarketCoinOps for UtxoCoin {
    fn my_address(&self) -> Cow<str> {
        self.0.my_address.to_string().into()
    }

    fn my_balance(&self) -> Box<Future<Item=f64, Error=String> + Send> {
        self.rpc_client.display_balance(self.my_address.clone(), self.decimals)
    }

    fn send_raw_tx(&self, tx: &str) -> Box<Future<Item=String, Error=String> + Send> {
        let bytes = try_fus!(hex::decode(tx));
        Box::new(self.rpc_client.send_raw_transaction(bytes.into()).map(|hash| format!("{:?}", hash)))
    }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u32,
        wait_until: u64,
    ) -> Result<(), String> {
        let tx: UtxoTx = try_s!(deserialize(tx).map_err(|e| ERRL!("{:?}", e)));
        self.rpc_client.wait_for_confirmations(
            &tx,
            confirmations as u32,
            wait_until,
        )
    }

    fn wait_for_tx_spend(&self, tx_bytes: &[u8], wait_until: u64) -> Result<TransactionEnum, String> {
        let tx: UtxoTx = try_s!(deserialize(tx_bytes).map_err(|e| ERRL!("{:?}", e)));

        let res = try_s!(self.rpc_client.wait_for_payment_spend(&tx, 0, wait_until));

        Ok(res.into())
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> {
        let transaction: UtxoTx = try_s!(deserialize(bytes).map_err(|err| format!("{:?}", err)));
        Ok(transaction.into())
    }

    fn current_block(&self) -> Box<Future<Item=u64, Error=String> + Send> {
        self.rpc_client.get_block_count()
    }
}

impl IguanaInfo for UtxoCoin {
    fn ticker<'a> (&'a self) -> &'a str {&self.ticker[..]}
}

#[derive(Serialize)]
struct UtxoFeeDetails {
    amount: f64,
}

impl MmCoin for UtxoCoin {
    fn is_asset_chain(&self) -> bool { self.asset_chain }

    fn check_i_have_enough_to_trade(&self, amount: f64, maker: bool) -> Box<Future<Item=(), Error=String> + Send> {
        const DUST_F64: f64 = 0.00001;
        if amount / 777.0 < DUST_F64 {
            return Box::new(futures::future::err(ERRL!("Amount {} is too low, it'll result to dust error, at least {} is required", amount, 0.00777)));
        }
        let fee_fut = self.get_tx_fee();
        let arc = self.clone();
        Box::new(
            fee_fut.and_then(move |coin_fee| {
                let fee = match coin_fee {
                    ActualTxFee::Fixed(f) => f,
                    ActualTxFee::Dynamic(f) => f,
                };
                let fee_f64 = dstr(fee as i64, arc.decimals);
                arc.my_balance().and_then(move |balance| {
                    let required = if maker {
                        amount + fee_f64
                    } else {
                        amount + amount / 777.0 + 2.0 * fee_f64
                    };
                    if balance < required {
                        return ERR!("{} balance {} is too low, required {:.8}", arc.ticker(), balance, required);
                    }
                    Ok(())
                })
            })
        )
    }

    fn can_i_spend_other_payment(&self) -> Box<Future<Item=(), Error=String> + Send> {
        Box::new(futures::future::ok(()))
    }

    fn withdraw(&self, to: &str, amount: f64, max: bool) -> Box<Future<Item=TransactionDetails, Error=String> + Send> {
        let to: Address = try_fus!(Address::from_str(to));
        let script_pubkey = Builder::build_p2pkh(&to.hash).to_bytes();
        let utxo_lock = MutexGuardWrapper(try_fus!(UTXO_LOCK.lock()));
        let unspent_fut = self.rpc_client.list_unspent_ordered(&self.my_address);
        let arc = self.clone();
        Box::new(unspent_fut.and_then(move |unspents| {
            let (value, fee_policy) = if max {
                (unspents.iter().fold(0, |sum, unspent| sum + unspent.value), FeePolicy::DeductFromOutput(0))
            } else {
                ((amount * 10.0_f64.powf(arc.decimals as f64)) as u64, FeePolicy::SendExact)
            };
            let outputs = vec![TransactionOutput {
                value,
                script_pubkey,
            }];

            arc.generate_transaction(
                unspents,
                outputs,
                fee_policy,
            ).and_then(move |(unsigned, data)| {
                drop(utxo_lock);
                let prev_script = Builder::build_p2pkh(&arc.my_address.hash);
                let signed = try_s!(sign_tx(unsigned, &arc.key_pair, prev_script));
                let fee_details = UtxoFeeDetails {
                    amount: dstr(data.fee_amount as i64, arc.decimals),
                };
                Ok(TransactionDetails {
                    from: vec![arc.my_address().into()],
                    to: vec![format!("{}", to)],
                    total_amount: dstr(data.spent_by_me as i64, arc.decimals),
                    spent_by_me: dstr(data.spent_by_me as i64, arc.decimals),
                    received_by_me: dstr(data.received_by_me as i64, arc.decimals),
                    my_balance_change: dstr(data.received_by_me as i64 - data.spent_by_me as i64, arc.decimals),
                    tx_hash: signed.hash().reversed().to_vec().into(),
                    tx_hex: serialize(&signed).into(),
                    fee_details: try_s!(json::to_value(fee_details)),
                    block_height: 0,
                    coin: arc.ticker.clone(),
                    internal_id: vec![].into(),
                    timestamp: now_ms() / 1000,
                })
            })
        }))
    }

    fn decimals(&self) -> u8 {
        self.decimals
    }

    fn process_history_loop(&self, ctx: MmArc) {
        let history = self.load_history_from_file(&ctx);
        let mut history_map: HashMap<H256Json, TransactionDetails> = history.into_iter().map(|tx| (H256Json::from(tx.tx_hash.as_slice()), tx)).collect();
        loop {
            let tx_ids: Vec<H256Json> = match &self.rpc_client {
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
                            Some(item.txid)
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
                            ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on scripthash_get_history, retrying", e));
                            thread::sleep(Duration::from_secs(10));
                            continue;
                        }
                    };
                    // electrum returns the most recent transactions in the end but we need to
                    // process them first so rev is required
                    electrum_history.into_iter().rev().map(|item| item.tx_hash).collect()
                }
            };
            for txid in tx_ids {
                let mut updated = false;
                match history_map.entry(txid.clone()) {
                    Entry::Vacant(e) => {
                        if let Ok(tx_details) = self.tx_details_by_hash(&txid.0) {
                            e.insert(tx_details);
                            updated = true;
                        }
                    },
                    Entry::Occupied(mut e) => {
                        // request tx details again for unconfirmed transaction
                        if e.get().block_height == 0 {
                            if let Ok(tx_details) = self.tx_details_by_hash(&txid.0) {
                                e.insert(tx_details);
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
            thread::sleep(Duration::from_secs(30));
        }
    }

    fn tx_details_by_hash(&self, hash: &[u8]) -> Result<TransactionDetails, String> {
        let hash = H256Json::from(hash);
        let verbose_tx = try_s!(self.rpc_client.get_verbose_transaction(hash).wait());
        let tx: UtxoTx = try_s!(deserialize(verbose_tx.hex.as_slice()).map_err(|e| ERRL!("{:?}", e)));
        let mut input_transactions: HashMap<&H256, UtxoTx> = HashMap::new();
        let mut input_amount = 0;
        let mut output_amount = 0;
        let mut from_addresses = vec![];
        let mut to_addresses = vec![];
        let mut spent_by_me = 0;
        let mut received_by_me = 0;
        for input in tx.inputs.iter() {
            let input_tx = match input_transactions.entry(&input.previous_output.hash) {
                Entry::Vacant(e) => {
                    let prev: BytesJson = try_s!(self.rpc_client.get_transaction_bytes(input.previous_output.hash.reversed().into()).wait());
                    let prev_tx: UtxoTx = try_s!(deserialize(prev.as_slice()).map_err(|e| ERRL!("{:?}", e)));
                    e.insert(prev_tx)
                },
                Entry::Occupied(e) => e.into_mut(),
            };
            input_amount += input_tx.outputs[input.previous_output.index as usize].value;
            let from: Vec<Address> = try_s!(self.addresses_from_script(&input_tx.outputs[input.previous_output.index as usize].script_pubkey.clone().into()));
            if from.contains(&self.my_address) {
                spent_by_me += input_tx.outputs[input.previous_output.index as usize].value;
            }
            from_addresses.push(from);
        };

        for output in tx.outputs.iter() {
            output_amount += output.value;
            let to = try_s!(self.addresses_from_script(&output.script_pubkey.clone().into()));
            if to.contains(&self.my_address) {
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

        let fee = self.denominate_satoshis(input_amount as i64 - output_amount as i64);
        Ok(TransactionDetails {
            from: from_addresses,
            to: to_addresses,
            received_by_me: self.denominate_satoshis(received_by_me as i64),
            spent_by_me: self.denominate_satoshis(spent_by_me as i64),
            my_balance_change: self.denominate_satoshis(received_by_me as i64 - spent_by_me as i64),
            total_amount: self.denominate_satoshis(input_amount as i64),
            tx_hash: tx.hash().reversed().to_vec().into(),
            tx_hex: verbose_tx.hex,
            fee_details: json!({
                "amount": fee
            }),
            block_height: verbose_tx.height,
            coin: self.ticker.clone(),
            internal_id: tx.hash().reversed().to_vec().into(),
            timestamp: verbose_tx.time.into(),
        })
    }
}

pub fn random_compressed_key_pair(prefix: u8, checksum_type: ChecksumType) -> Result<KeyPair, String> {
    let random_key = try_s!(Random::new(prefix).generate());

    Ok(try_s!(KeyPair::from_private(Private {
        prefix,
        secret: random_key.private().secret.clone(),
        compressed: true,
        checksum_type,
    })))
}

fn private_from_seed(seed: &str) -> Result<Private, String> {
    match seed.parse() {
        Ok(private) => return Ok(private),
        Err(e) => match e {
            KeysError::InvalidChecksum => return ERR!("Provided WIF passphrase has invalid checksum!"),
            _ => (), // ignore other errors, assume the passphrase is not WIF
        },
    }

    if seed.starts_with("0x") {
        let hash = try_s!(H256::from_str(&seed[2..]));
        Ok(Private {
            prefix: 0,
            secret: hash,
            compressed: true,
            checksum_type: ChecksumType::DSHA256,
        })
    } else {
        let mut hash = sha256(seed.as_bytes());
        hash[0] &= 248;
        hash[31] &= 127;
        hash[31] |= 64;

        Ok(Private {
            prefix: 0,
            secret: hash,
            compressed: true,
            checksum_type: ChecksumType::DSHA256,
        })
    }
}

pub fn key_pair_from_seed(seed: &str) -> Result<KeyPair, String> {
    let private = try_s!(private_from_seed(seed));
    Ok(try_s!(KeyPair::from_private(private)))
}

pub enum UtxoInitMode {
    Native,
    Electrum(Vec<String>),
}

pub fn utxo_coin_from_iguana_info(
    info: *mut lp::iguana_info,
    mode: UtxoInitMode,
    rpc_port: u16,
) -> Result<MmCoinEnum, String> {
    let info = unsafe { *info };
    let ticker = try_s! (unsafe {CStr::from_ptr (info.symbol.as_ptr())} .to_str()) .into();

    let checksum_type = if ticker == "GRS" {
        ChecksumType::DGROESTL512
    } else if ticker == "SMART" {
        ChecksumType::KECCAK256
    } else {
        ChecksumType::DSHA256
    };

    let private = Private {
        prefix: info.wiftype,
        secret: H256::from(unsafe { lp::G.LP_privkey.bytes }),
        compressed: true,
        checksum_type,
    };

    let key_pair = try_s!(KeyPair::from_private(private));
    let my_address = Address {
        prefix: info.pubtype,
        t_addr_prefix: info.taddr,
        hash: key_pair.public().address_hash(),
        checksum_type,
    };

    let rpc_client = match mode {
        UtxoInitMode::Native => {
            let auth_str = unsafe { try_s!(CStr::from_ptr(info.userpass.as_ptr()).to_str()) };
            let uri = unsafe { try_s!(CStr::from_ptr(info.serverport.as_ptr()).to_str()) };
            let client = Arc::new(NativeClientImpl {
                // Similar to `fomat!("http://127.0.0.1:"(rpc_port))`.
                uri: format!("http://{}", uri),
                auth: format!("Basic {}", base64_encode(auth_str, URL_SAFE)),
            });

            UtxoRpcClientEnum::Native(NativeClient(client))
        },
        UtxoInitMode::Electrum(mut urls) => {
            let mut rng = thread_rng();
            urls.as_mut_slice().shuffle(&mut rng);
            let mut client = ElectrumClientImpl::new();
            for url in urls.iter() {
                try_s!(client.add_server(url));
            }

            let client = Arc::new(client);
            try_s!(client.blockchain_headers_subscribe().wait());
            // ping the electrum servers every 30 seconds to prevent them from disconnecting us.
            // according to docs server can do it if there are no messages in ~10 minutes.
            // https://electrumx.readthedocs.io/en/latest/protocol-methods.html?highlight=keep#server-ping
            // weak reference will allow to stop the thread if client is dropped
            let weak_client = Arc::downgrade(&client);
            try_s!(thread::Builder::new().name(format!("electrum_ping_{}", ticker)).spawn(move || {
                loop {
                    if let Some(client) = weak_client.upgrade() {
                        if let Err(e) = client.server_ping().wait() {
                            log!("Electrum servers " [urls] " ping error " [e]);
                        }

                        if let Err(e) = client.blockchain_headers_subscribe().wait() {
                            log!("Electrum servers " [urls] " subscribe error " [e]);
                        }
                    } else {
                        break;
                    }
                    thread::sleep(Duration::from_secs(30));
                }
            }));
            UtxoRpcClientEnum::Electrum(ElectrumClient(client))
        }
    };
    let (tx_version, overwintered) = if info.isassetchain == 1 || ticker == "KMD" || ticker == "BEER" || ticker == "PIZZA" {
        (4, true)
    } else {
        (info.txversion, info.overwintered == 1)
    };
    let tx_fee = if info.txfee > 0 {
        TxFee::Fixed(info.txfee)
    } else {
        TxFee::Dynamic
    };
    let version_group_id = if tx_version == 3 && overwintered {
        0x03c48270
    } else if tx_version == 4 && overwintered {
        0x892f2085
    } else {
        0
    };

    let decimals = if info.decimals > 0 {
        info.decimals
    } else {
        8
    };
    // should be sufficient to detect zcash by overwintered flag
    let zcash = overwintered;
    let coin = UtxoCoinImpl {
        ticker,
        decimals,
        rpc_client,
        key_pair,
        is_pos: false,
        notarized: false,
        overwintered,
        pub_addr_prefix: info.pubtype,
        p2sh_addr_prefix: info.p2shtype,
        pub_t_addr_prefix: info.taddr,
        p2sh_t_addr_prefix: info.taddr,
        rpc_password: "".to_owned(),
        rpc_port,
        rpc_user: "".to_owned(),
        segwit: false,
        wif_prefix: info.wiftype,
        tx_version,
        my_address: my_address.clone(),
        asset_chain: info.isassetchain == 1,
        tx_fee,
        version_group_id,
        zcash,
        checksum_type,
    };
    Ok(UtxoCoin(Arc::new(coin)).into())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn utxo_coin_for_test() -> UtxoCoin {
        let checksum_type = ChecksumType::DSHA256;
        let key_pair = key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid").unwrap();
        let my_address = Address {
            prefix: 60,
            hash: key_pair.public().address_hash(),
            t_addr_prefix: 0,
            checksum_type,
        };

        let mut client = ElectrumClientImpl::new();
        log!("My address "(my_address));
        client.add_server("electrum1.cipig.net:10025").unwrap();

        let coin = UtxoCoinImpl {
            decimals: 8,
            rpc_client: UtxoRpcClientEnum::Electrum(ElectrumClient(Arc::new(client))),
            key_pair,
            is_pos: false,
            notarized: false,
            overwintered: true,
            rpc_password: "".to_owned(),
            rpc_port: 0,
            rpc_user: "".to_owned(),
            segwit: false,
            tx_version: 4,
            my_address,
            asset_chain: true,
            p2sh_addr_prefix: 85,
            p2sh_t_addr_prefix: 0,
            pub_addr_prefix: 60,
            pub_t_addr_prefix: 0,
            ticker: "ETOMIC".into(),
            wif_prefix: 0,
            tx_fee: TxFee::Fixed(1000),
            version_group_id: 0x892f2085,
            zcash: true,
            checksum_type,
        };

        UtxoCoin(Arc::new(coin))
    }

    #[test]
    fn test_extract_secret() {
        let tx: UtxoTx = "0100000001de7aa8d29524906b2b54ee2e0281f3607f75662cbc9080df81d1047b78e21dbc00000000d7473044022079b6c50820040b1fbbe9251ced32ab334d33830f6f8d0bf0a40c7f1336b67d5b0220142ccf723ddabb34e542ed65c395abc1fbf5b6c3e730396f15d25c49b668a1a401209da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365004c6b6304f62b0e5cb175210270e75970bb20029b3879ec76c4acd320a8d0589e003636264d01a7d566504bfbac6782012088a9142fb610d856c19fd57f2d0cffe8dff689074b3d8a882103f368228456c940ac113e53dad5c104cf209f2f102a409207269383b6ab9b03deac68ffffffff01d0dc9800000000001976a9146d9d2b554d768232320587df75c4338ecc8bf37d88ac40280e5c".into();
        let secret = tx.extract_secret().unwrap();
        let expected_secret = hex::decode("9da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365").unwrap();
        assert_eq!(expected_secret, secret);
    }

    #[test]
    fn test_generate_transaction() {
        let coin = utxo_coin_for_test();
        let unspents = vec![UnspentInfo {
            value: 10000000000,
            outpoint: OutPoint::default(),
        }];

        let outputs = vec![TransactionOutput {
            script_pubkey: vec![].into(),
            value: 999,
        }];

        let generated = coin.generate_transaction(unspents, outputs, FeePolicy::SendExact).wait();
        // must not allow to use output with value < dust
        unwrap_err!(generated);

        let unspents = vec![UnspentInfo {
            value: 100000,
            outpoint: OutPoint::default(),
        }];

        let outputs = vec![TransactionOutput {
            script_pubkey: vec![].into(),
            value: 98001,
        }];

        let generated = unwrap!(coin.generate_transaction(unspents, outputs, FeePolicy::SendExact).wait());
        // the change that is less than dust must be included to miner fee
        // so no extra outputs should appear in generated transaction
        assert_eq!(generated.0.outputs.len(), 1);

        assert_eq!(generated.1.fee_amount, 1999);
        assert_eq!(generated.1.received_by_me, 0);
        assert_eq!(generated.1.spent_by_me, 100000);

        let unspents = vec![UnspentInfo {
            value: 100000,
            outpoint: OutPoint::default(),
        }];

        let outputs = vec![TransactionOutput {
            script_pubkey: "76a91405aab5342166f8594baf17a7d9bef5d56744332788ac".into(),
            value: 100000,
        }];

        // test that fee is properly deducted from output amount equal to input amount (max withdraw case)
        let generated = unwrap!(coin.generate_transaction(unspents, outputs, FeePolicy::DeductFromOutput(0)).wait());
        assert_eq!(generated.0.outputs.len(), 1);

        assert_eq!(generated.1.fee_amount, 1000);
        assert_eq!(generated.1.received_by_me, 99000);
        assert_eq!(generated.1.spent_by_me, 100000);
        assert_eq!(generated.0.outputs[0].value, 99000);

        let unspents = vec![UnspentInfo {
            value: 100000,
            outpoint: OutPoint::default(),
        }];

        let outputs = vec![TransactionOutput {
            script_pubkey: vec![].into(),
            value: 100000,
        }];

        // test that generate_transaction returns an error when input amount is not sufficient to cover output + fee
        unwrap_err!(coin.generate_transaction(unspents, outputs, FeePolicy::SendExact).wait());
    }

    #[test]
    fn test_addresses_from_script() {
        let coin = utxo_coin_for_test();
        // P2PKH
        let script: Script = "76a91405aab5342166f8594baf17a7d9bef5d56744332788ac".into();
        let expected_addr: Vec<Address> = vec!["R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW".into()];
        let actual_addr = unwrap!(coin.addresses_from_script(&script));
        assert_eq!(expected_addr, actual_addr);

        // P2SH
        let script: Script = "a914e71a6120653ebd526e0f9d7a29cde5969db362d487".into();
        let expected_addr: Vec<Address> = vec!["bZoEPR7DjTqSDiQTeRFNDJuQPTRY2335LD".into()];
        let actual_addr = unwrap!(coin.addresses_from_script(&script));
        assert_eq!(expected_addr, actual_addr);
    }

    #[test]
    fn test_kmd_interest() {
        let value = 64605500822;
        let lock_time = 1556623906;
        let current_time = 1556623906 + 3600 + 300;
        let expected = 36870;
        let actual = kmd_interest(1000001, value, lock_time, current_time);
        assert_eq!(expected, actual);

        // UTXO amount must be at least 10 KMD to be eligible for interest
        let value = 999999999;
        let lock_time = 1556623906;
        let current_time = 1556623906 + 3600 + 300;
        let expected = 0;
        let actual = kmd_interest(1000001, value, lock_time, current_time);
        assert_eq!(expected, actual);
    }
}

//! Transaction signer

use blake2b_simd::Params as Blake2b;
use bytes::Bytes;
use chain::{JoinSplit, OutPoint, ShieldedOutput, ShieldedSpend, Transaction, TransactionInput, TransactionOutput,
            TxHashAlgo};
use crypto::{dhash256, sha256};
use hash::{H256, H512};
use keys::KeyPair;
use ser::Stream;
use serde::Deserialize;
use {Builder, Script};

const ZCASH_PREVOUTS_HASH_PERSONALIZATION: &[u8] = b"ZcashPrevoutHash";
const ZCASH_SEQUENCE_HASH_PERSONALIZATION: &[u8] = b"ZcashSequencHash";
const ZCASH_OUTPUTS_HASH_PERSONALIZATION: &[u8] = b"ZcashOutputsHash";
const ZCASH_JOIN_SPLITS_HASH_PERSONALIZATION: &[u8] = b"ZcashJSplitsHash";
const ZCASH_SHIELDED_SPENDS_HASH_PERSONALIZATION: &[u8] = b"ZcashSSpendsHash";
const ZCASH_SHIELDED_OUTPUTS_HASH_PERSONALIZATION: &[u8] = b"ZcashSOutputHash";
const ZCASH_SIG_HASH_PERSONALIZATION: &[u8] = b"ZcashSigHash";

#[derive(Debug, PartialEq, Clone, Copy, Deserialize)]
pub enum SignatureVersion {
    #[serde(rename = "base")]
    Base,
    #[serde(rename = "witness_v0")]
    WitnessV0,
    #[serde(rename = "fork_id")]
    ForkId,
}

#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum SighashBase {
    All = 1,
    None = 2,
    Single = 3,
}

impl From<SighashBase> for u32 {
    fn from(s: SighashBase) -> Self { s as u32 }
}

#[cfg_attr(feature = "cargo-clippy", allow(clippy::doc_markdown))]
/// Signature hash type. [Documentation](https://en.bitcoin.it/wiki/OP_CHECKSIG#Procedure_for_Hashtype_SIGHASH_SINGLE)
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Sighash {
    pub base: SighashBase,
    pub anyone_can_pay: bool,
    pub fork_id: bool,
}

impl From<Sighash> for u32 {
    fn from(s: Sighash) -> Self {
        let base = s.base as u32;
        let base = if s.anyone_can_pay { base | 0x80 } else { base };

        if s.fork_id {
            base | 0x40
        } else {
            base
        }
    }
}

impl Sighash {
    pub fn new(base: SighashBase, anyone_can_pay: bool, fork_id: bool) -> Self {
        Sighash {
            base,
            anyone_can_pay,
            fork_id,
        }
    }

    /// Used by SCRIPT_VERIFY_STRICTENC
    pub fn is_defined(version: SignatureVersion, u: u32) -> bool {
        // reset anyone_can_pay && fork_id (if applicable) bits
        let u = match version {
            SignatureVersion::ForkId => u & !(0x40 | 0x80),
            _ => u & !(0x80),
        };

        // Only exact All | None | Single values are passing this check
        matches!(u, 1 | 2 | 3)
    }

    /// Creates Sighash from any u, even if is_defined() == false
    pub fn from_u32(version: SignatureVersion, u: u32) -> Self {
        let anyone_can_pay = (u & 0x80) == 0x80;
        let fork_id = version == SignatureVersion::ForkId && (u & 0x40) == 0x40;
        let base = match u & 0x1f {
            2 => SighashBase::None,
            3 => SighashBase::Single,
            _ => SighashBase::All,
        };

        Sighash::new(base, anyone_can_pay, fork_id)
    }
}

#[derive(Clone, Debug)]
pub struct UnsignedTransactionInput {
    pub previous_output: OutPoint,
    pub sequence: u32,
    pub amount: u64,
    pub witness: Vec<Vec<u8>>,
}

/// Used for resigning and loading test transactions
impl From<TransactionInput> for UnsignedTransactionInput {
    fn from(i: TransactionInput) -> Self {
        UnsignedTransactionInput {
            previous_output: i.previous_output,
            sequence: i.sequence,
            amount: 0,
            witness: i.script_witness.into_iter().map(Vec::from).collect(),
        }
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug)]
pub enum SignerHashAlgo {
    SHA256,
    DSHA256,
}

impl From<TxHashAlgo> for SignerHashAlgo {
    fn from(tx_hash: TxHashAlgo) -> SignerHashAlgo {
        match tx_hash {
            TxHashAlgo::DSHA256 => SignerHashAlgo::DSHA256,
            TxHashAlgo::SHA256 => SignerHashAlgo::SHA256,
        }
    }
}

impl From<SignerHashAlgo> for TxHashAlgo {
    fn from(algo: SignerHashAlgo) -> Self {
        match algo {
            SignerHashAlgo::DSHA256 => TxHashAlgo::DSHA256,
            SignerHashAlgo::SHA256 => TxHashAlgo::SHA256,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TransactionInputSigner {
    pub version: i32,
    pub n_time: Option<u32>,
    pub overwintered: bool,
    pub version_group_id: u32,
    pub consensus_branch_id: u32,
    pub expiry_height: u32,
    pub value_balance: i64,
    pub inputs: Vec<UnsignedTransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub lock_time: u32,
    pub join_splits: Vec<JoinSplit>,
    pub shielded_spends: Vec<ShieldedSpend>,
    pub shielded_outputs: Vec<ShieldedOutput>,
    pub zcash: bool,
    pub posv: bool,
    pub str_d_zeel: Option<String>,
    pub hash_algo: SignerHashAlgo,
}

/// Used for resigning and loading test transactions
impl From<Transaction> for TransactionInputSigner {
    fn from(t: Transaction) -> Self {
        TransactionInputSigner {
            version: t.version,
            n_time: t.n_time,
            overwintered: t.overwintered,
            version_group_id: t.version_group_id,
            consensus_branch_id: 0,
            expiry_height: t.expiry_height,
            value_balance: t.value_balance,
            inputs: t.inputs.into_iter().map(Into::into).collect(),
            outputs: t.outputs,
            lock_time: t.lock_time,
            join_splits: t.join_splits.clone(),
            shielded_spends: t.shielded_spends.clone(),
            shielded_outputs: t.shielded_outputs.clone(),
            zcash: t.zcash,
            posv: t.posv,
            str_d_zeel: t.str_d_zeel,
            hash_algo: t.tx_hash_algo.into(),
        }
    }
}

/// Used during transaction construction with dynamic fee calculation (sat per kbyte)
/// to calculate tx size, TransactionInputSigner doesn't implement Serializable
impl From<TransactionInputSigner> for Transaction {
    fn from(t: TransactionInputSigner) -> Self {
        Transaction {
            version: t.version,
            n_time: t.n_time,
            overwintered: t.overwintered,
            version_group_id: t.version_group_id,
            expiry_height: t.expiry_height,
            value_balance: t.value_balance,
            inputs: t
                .inputs
                .into_iter()
                .map(|input| TransactionInput {
                    previous_output: input.previous_output,
                    script_sig: vec![].into(),
                    sequence: input.sequence,
                    script_witness: vec![],
                })
                .collect(),
            outputs: t.outputs,
            lock_time: t.lock_time,
            join_splits: t.join_splits.clone(),
            shielded_spends: t.shielded_spends.clone(),
            shielded_outputs: t.shielded_outputs.clone(),
            zcash: t.zcash,
            posv: t.posv,
            binding_sig: H512::default(),
            join_split_pubkey: H256::default(),
            join_split_sig: H512::default(),
            str_d_zeel: t.str_d_zeel,
            tx_hash_algo: t.hash_algo.into(),
        }
    }
}

impl TransactionInputSigner {
    pub fn signature_hash(
        &self,
        input_index: usize,
        input_amount: u64,
        script_pubkey: &Script,
        sigversion: SignatureVersion,
        sighashtype: u32,
    ) -> H256 {
        let sighash = Sighash::from_u32(sigversion, sighashtype);
        match sigversion {
            SignatureVersion::ForkId if sighash.fork_id => {
                self.signature_hash_fork_id(input_index, input_amount, script_pubkey, sighashtype, sighash)
            },
            SignatureVersion::Base | SignatureVersion::ForkId => {
                self.signature_hash_original(input_index, script_pubkey, sighashtype, sighash)
            },
            SignatureVersion::WitnessV0 => {
                self.signature_hash_witness0(input_index, input_amount, script_pubkey, sighashtype, sighash)
            },
        }
    }

    /// input_index - index of input to sign
    /// script_pubkey - script_pubkey of input's previous_output pubkey
    pub fn signed_input(
        &self,
        keypair: &KeyPair,
        input_index: usize,
        input_amount: u64,
        script_pubkey: &Script,
        sigversion: SignatureVersion,
        sighash: u32,
    ) -> TransactionInput {
        let hash = self.signature_hash(input_index, input_amount, script_pubkey, sigversion, sighash);

        let mut signature: Vec<u8> = keypair.private().sign(&hash).unwrap().into();
        signature.push(sighash as u8);
        let script_sig = Builder::default()
            .push_data(&signature)
            //.push_data(keypair.public())
            .into_script();

        let unsigned_input = &self.inputs[input_index];
        TransactionInput {
            previous_output: unsigned_input.previous_output,
            sequence: unsigned_input.sequence,
            script_sig: script_sig.to_bytes(),
            script_witness: vec![],
        }
    }

    pub fn signature_hash_original(
        &self,
        input_index: usize,
        script_pubkey: &Script,
        sighashtype: u32,
        sighash: Sighash,
    ) -> H256 {
        if input_index >= self.inputs.len() {
            return 1u8.into();
        }

        if sighash.base == SighashBase::Single && input_index >= self.outputs.len() {
            return 1u8.into();
        }

        if self.version >= 3 && self.overwintered {
            return self
                .signature_hash_overwintered(input_index, script_pubkey, sighashtype, sighash)
                .unwrap();
        }

        let script_pubkey = script_pubkey.without_separators();

        let inputs = if sighash.anyone_can_pay {
            let input = &self.inputs[input_index];
            vec![TransactionInput {
                previous_output: input.previous_output,
                script_sig: script_pubkey.to_bytes(),
                sequence: input.sequence,
                script_witness: vec![],
            }]
        } else {
            self.inputs
                .iter()
                .enumerate()
                .map(|(n, input)| TransactionInput {
                    previous_output: input.previous_output,
                    script_sig: if n == input_index {
                        script_pubkey.to_bytes()
                    } else {
                        Bytes::default()
                    },
                    sequence: match sighash.base {
                        SighashBase::Single | SighashBase::None if n != input_index => 0,
                        _ => input.sequence,
                    },
                    script_witness: vec![],
                })
                .collect()
        };

        let outputs = match sighash.base {
            SighashBase::All => self.outputs.clone(),
            SighashBase::Single => self
                .outputs
                .iter()
                .take(input_index + 1)
                .enumerate()
                .map(|(n, out)| {
                    if n == input_index {
                        out.clone()
                    } else {
                        TransactionOutput::default()
                    }
                })
                .collect(),
            SighashBase::None => Vec::new(),
        };

        // PoSV transactions have n_time truncated when creating signed inputs
        let n_time: Option<u32> = if self.posv { None } else { self.n_time };

        let tx = Transaction {
            inputs,
            outputs,
            version: self.version,
            n_time,
            lock_time: self.lock_time,
            binding_sig: H512::default(),
            expiry_height: 0,
            join_split_pubkey: H256::default(),
            join_split_sig: H512::default(),
            join_splits: vec![],
            overwintered: false,
            shielded_spends: vec![],
            shielded_outputs: vec![],
            value_balance: 0,
            version_group_id: 0,
            zcash: self.zcash,
            posv: self.posv,
            str_d_zeel: self.str_d_zeel.clone(),
            tx_hash_algo: self.hash_algo.into(),
        };

        let mut stream = Stream::default();
        stream.append(&tx);
        stream.append(&sighashtype);
        let out = stream.out();
        match self.hash_algo {
            SignerHashAlgo::DSHA256 => dhash256(&out),
            SignerHashAlgo::SHA256 => sha256(&out),
        }
    }

    fn signature_hash_witness0(
        &self,
        input_index: usize,
        input_amount: u64,
        script_pubkey: &Script,
        sighashtype: u32,
        sighash: Sighash,
    ) -> H256 {
        let hash_prevouts = compute_hash_prevouts(sighash, &self.inputs);
        let hash_sequence = compute_hash_sequence(sighash, &self.inputs);
        let hash_outputs = compute_hash_outputs(sighash, input_index, &self.outputs);

        let mut stream = Stream::default();
        stream.append(&self.version);
        stream.append(&hash_prevouts);
        stream.append(&hash_sequence);
        stream.append(&self.inputs[input_index].previous_output);
        stream.append_list(script_pubkey);
        stream.append(&input_amount);
        stream.append(&self.inputs[input_index].sequence);
        stream.append(&hash_outputs);
        stream.append(&self.lock_time);
        stream.append(&sighashtype); // this also includes 24-bit fork id. which is 0 for BitcoinCash
        let out = stream.out();
        dhash256(&out)
    }

    fn signature_hash_fork_id(
        &self,
        input_index: usize,
        input_amount: u64,
        script_pubkey: &Script,
        sighashtype: u32,
        sighash: Sighash,
    ) -> H256 {
        if input_index >= self.inputs.len() {
            return 1u8.into();
        }

        if sighash.base == SighashBase::Single && input_index >= self.outputs.len() {
            return 1u8.into();
        }

        self.signature_hash_witness0(input_index, input_amount, script_pubkey, sighashtype, sighash)
    }

    /// https://github.com/zcash/zips/blob/master/zip-0243.rst#notes
    /// This method doesn't cover all possible Sighash combinations so it doesn't fully match the
    /// specification, however I don't need other cases yet as BarterDEX marketmaker always uses
    /// SIGHASH_ALL
    pub fn signature_hash_overwintered(
        &self,
        input_index: usize,
        script_pubkey: &Script,
        sighashtype: u32,
        _sighash: Sighash,
    ) -> Result<H256, String> {
        let mut sig_hash_stream = Stream::new();

        let mut personalization = ZCASH_SIG_HASH_PERSONALIZATION.to_vec();
        // uint32_t leConsensusBranchId = htole32(consensusBranchId);
        // unsigned char personalization[16] = {};
        // memcpy(personalization, "ZcashSigHash", 12);
        // memcpy(personalization+12, &leConsensusBranchId, 4);
        // https://github.com/zcash/zcash/issues/3413
        if self.version >= 3 {
            personalization.extend_from_slice(&self.consensus_branch_id.to_le_bytes());
        }

        let mut header = self.version;
        if self.overwintered {
            header |= 1 << 31;
        }
        sig_hash_stream.append(&header);
        sig_hash_stream.append(&self.version_group_id);

        let mut prev_out_stream = Stream::new();
        for input in self.inputs.iter() {
            prev_out_stream.append(&input.previous_output);
        }
        sig_hash_stream.append(&blake_2b_256_personal(
            &prev_out_stream.out(),
            ZCASH_PREVOUTS_HASH_PERSONALIZATION,
        ));

        let mut sequence_stream = Stream::new();
        for input in self.inputs.iter() {
            sequence_stream.append(&input.sequence);
        }

        sig_hash_stream.append(&blake_2b_256_personal(
            &sequence_stream.out(),
            ZCASH_SEQUENCE_HASH_PERSONALIZATION,
        ));

        let mut outputs_stream = Stream::new();
        for output in self.outputs.iter() {
            outputs_stream.append(output);
        }

        sig_hash_stream.append(&blake_2b_256_personal(
            &outputs_stream.out(),
            ZCASH_OUTPUTS_HASH_PERSONALIZATION,
        ));

        if !self.join_splits.is_empty() {
            let mut join_splits_stream = Stream::new();
            for split in self.join_splits.iter() {
                join_splits_stream.append(split);
            }
            sig_hash_stream.append(&blake_2b_256_personal(
                &join_splits_stream.out(),
                ZCASH_JOIN_SPLITS_HASH_PERSONALIZATION,
            ));
        } else {
            sig_hash_stream.append(&H256::default());
        }

        if !self.shielded_spends.is_empty() {
            let mut s_spends_stream = Stream::new();
            for spend in self.shielded_spends.iter() {
                s_spends_stream
                    .append(&spend.cv)
                    .append(&spend.anchor)
                    .append(&spend.nullifier)
                    .append(&spend.rk)
                    .append(&spend.zkproof);
            }
            sig_hash_stream.append(&blake_2b_256_personal(
                &s_spends_stream.out(),
                ZCASH_SHIELDED_SPENDS_HASH_PERSONALIZATION,
            ));
        } else {
            sig_hash_stream.append(&H256::default());
        }

        if !self.shielded_outputs.is_empty() {
            let mut s_outputs_stream = Stream::new();
            for output in self.shielded_outputs.iter() {
                s_outputs_stream.append(output);
            }
            let hash_shielded_outputs =
                blake_2b_256_personal(&s_outputs_stream.out(), ZCASH_SHIELDED_OUTPUTS_HASH_PERSONALIZATION);
            println!("hash_shielded_outputs {:?}", hash_shielded_outputs.reversed());
            sig_hash_stream.append(&hash_shielded_outputs);
        } else {
            sig_hash_stream.append(&H256::default());
        }

        sig_hash_stream.append(&self.lock_time);
        sig_hash_stream.append(&self.expiry_height);
        sig_hash_stream.append(&self.value_balance);
        sig_hash_stream.append(&sighashtype);

        sig_hash_stream.append(&self.inputs[input_index].previous_output);
        sig_hash_stream.append(&script_pubkey.to_bytes());
        sig_hash_stream.append(&self.inputs[input_index].amount);
        sig_hash_stream.append(&self.inputs[input_index].sequence);

        Ok(blake_2b_256_personal(&sig_hash_stream.out(), &personalization))
    }
}

fn compute_hash_prevouts(sighash: Sighash, inputs: &[UnsignedTransactionInput]) -> H256 {
    match sighash.anyone_can_pay {
        false => {
            let mut stream = Stream::default();
            for input in inputs {
                stream.append(&input.previous_output);
            }
            dhash256(&stream.out())
        },
        true => 0u8.into(),
    }
}

fn compute_hash_sequence(sighash: Sighash, inputs: &[UnsignedTransactionInput]) -> H256 {
    match sighash.base {
        SighashBase::All if !sighash.anyone_can_pay => {
            let mut stream = Stream::default();
            for input in inputs {
                stream.append(&input.sequence);
            }
            dhash256(&stream.out())
        },
        _ => 0u8.into(),
    }
}

fn compute_hash_outputs(sighash: Sighash, input_index: usize, outputs: &[TransactionOutput]) -> H256 {
    match sighash.base {
        SighashBase::All => {
            let mut stream = Stream::default();
            for output in outputs {
                stream.append(output);
            }
            dhash256(&stream.out())
        },
        SighashBase::Single if input_index < outputs.len() => {
            let mut stream = Stream::default();
            stream.append(&outputs[input_index]);
            dhash256(&stream.out())
        },
        _ => 0u8.into(),
    }
}

fn blake_2b_256_personal(input: &[u8], personal: &[u8]) -> H256 {
    H256::from(
        Blake2b::new()
            .hash_length(32)
            .personal(personal)
            .to_state()
            .update(input)
            .finalize()
            .as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::{blake_2b_256_personal, Sighash, SighashBase, SignatureVersion, TransactionInputSigner,
                UnsignedTransactionInput};
    use bytes::Bytes;
    use chain::{OutPoint, Transaction, TransactionOutput};
    use hash::{H160, H256};
    use keys::{Address, AddressHashEnum, Private};
    use script::Script;
    use sign::SignerHashAlgo;

    // http://www.righto.com/2014/02/bitcoins-hard-way-using-raw-bitcoin.html
    // https://blockchain.info/rawtx/81b4c832d70cb56ff957589752eb4125a4cab78a25a8fc52d6a09e5bd4404d48
    // https://blockchain.info/rawtx/3f285f083de7c0acabd9f106a43ec42687ab0bebe2e6f0d529db696794540fea
    #[test]
    fn test_signature_hash_simple() {
        let _private: Private = "5HusYj2b2x4nroApgfvaSfKYZhRbKFH41bVyPooymbC6KfgSXdD".into();
        let previous_tx_hash =
            H256::from_reversed_str("81b4c832d70cb56ff957589752eb4125a4cab78a25a8fc52d6a09e5bd4404d48");
        let previous_output_index = 0;
        let to: Address = "1KKKK6N21XKo48zWKuQKXdvSsCf95ibHFa".into();
        assert!(to.hash.is_address_hash());
        let previous_output = "76a914df3bd30160e6c6145baaf2c88a8844c13a00d1d588ac".into();
        let current_output: Bytes = "76a914c8e90996c7c6080ee06284600c684ed904d14c5c88ac".into();
        let value = 91234;
        let expected_signature_hash: H256 = "5fda68729a6312e17e641e9a49fac2a4a6a680126610af573caab270d232f850".into();

        // this is irrelevant
        let mut hash = H160::default();
        if let AddressHashEnum::AddressHash(h) = to.hash {
            hash = h;
        }
        assert_eq!(&current_output[3..23], &*hash);

        let unsigned_input = UnsignedTransactionInput {
            sequence: 0xffff_ffff,
            previous_output: OutPoint {
                index: previous_output_index,
                hash: previous_tx_hash,
            },
            amount: 0,
            witness: vec![Vec::new()],
        };

        let output = TransactionOutput {
            value,
            script_pubkey: current_output,
        };

        let input_signer = TransactionInputSigner {
            version: 1,
            n_time: None,
            overwintered: false,
            version_group_id: 0,
            consensus_branch_id: 0,
            expiry_height: 0,
            value_balance: 0,
            lock_time: 0,
            inputs: vec![unsigned_input],
            outputs: vec![output],
            join_splits: vec![],
            shielded_spends: vec![],
            shielded_outputs: vec![],
            zcash: false,
            posv: false,
            str_d_zeel: None,
            hash_algo: SignerHashAlgo::DSHA256,
        };

        let hash = input_signer.signature_hash(0, 0, &previous_output, SignatureVersion::Base, SighashBase::All.into());
        assert_eq!(hash, expected_signature_hash);
    }

    #[test]
    fn test_signature_hash_posv() {
        let _private: Private = "cSQJp8ymcCUZCceowcTr5L1rr7tRbeB8uj3pDFRfRaMuRP6yDqfa".into();
        let previous_tx_hash =
            H256::from_reversed_str("0bc54ed426950f50bf2c2776034a03592e844757b42330eb908eb04492dad2c6");
        let previous_output_index = 1;
        let to: Address = "msj7SEQmH7pUCUx8YU6R87DrAHYzcABdzw".into();
        assert!(to.hash.is_address_hash());
        let previous_output = "76a914df3bd30160e6c6145baaf2c88a8844c13a00d1d588ac".into();
        let current_output: Bytes = "76a91485ee21a7f8cdd9034fb55004e0d8ed27db1c03c288ac".into();
        let value = 100000000;
        let expected_signature_hash: H256 = "21d91397ba4e4bfaf73584300804cf9f9fd11cabe43f1bb38f7986cea5ef5519".into();

        let unsigned_input = UnsignedTransactionInput {
            sequence: 0xffff_ffff,
            previous_output: OutPoint {
                index: previous_output_index,
                hash: previous_tx_hash,
            },
            amount: 100,
            witness: vec![Vec::new()],
        };

        let output = TransactionOutput {
            value,
            script_pubkey: current_output,
        };

        let input_signer = TransactionInputSigner {
            version: 2,
            n_time: Some(1682050928),
            overwintered: false,
            version_group_id: 0,
            consensus_branch_id: 0,
            expiry_height: 0,
            value_balance: 0,
            lock_time: 0,
            inputs: vec![unsigned_input],
            outputs: vec![output],
            join_splits: vec![],
            shielded_spends: vec![],
            shielded_outputs: vec![],
            zcash: false,
            posv: true,
            str_d_zeel: None,
            hash_algo: SignerHashAlgo::DSHA256,
        };

        let hash = input_signer.signature_hash(0, 0, &previous_output, SignatureVersion::Base, SighashBase::All.into());
        assert_eq!(hash, expected_signature_hash);
    }

    #[allow(dead_code)]
    fn run_test_sighash(
        tx: &'static str,
        script: &'static str,
        input_index: usize,
        hash_type: i32,
        result: &'static str,
    ) {
        let tx: Transaction = tx.into();
        let signer: TransactionInputSigner = tx.into();
        let script: Script = script.into();
        let expected = H256::from_reversed_str(result);

        let sighash = Sighash::from_u32(SignatureVersion::Base, hash_type as u32);
        let hash = signer.signature_hash_original(input_index, &script, hash_type as u32, sighash);
        assert_eq!(expected, hash);
    }

    #[test]
    fn test_sighash_forkid_from_u32() {
        assert!(!Sighash::is_defined(SignatureVersion::Base, 0xFFFFFF82));
        assert!(!Sighash::is_defined(SignatureVersion::Base, 0x00000182));
        assert!(!Sighash::is_defined(SignatureVersion::Base, 0x00000080));
        assert!(Sighash::is_defined(SignatureVersion::Base, 0x00000001));
        assert!(Sighash::is_defined(SignatureVersion::Base, 0x00000082));
        assert!(Sighash::is_defined(SignatureVersion::Base, 0x00000003));

        assert!(!Sighash::is_defined(SignatureVersion::ForkId, 0xFFFFFFC2));
        assert!(!Sighash::is_defined(SignatureVersion::ForkId, 0x000001C2));
        assert!(Sighash::is_defined(SignatureVersion::ForkId, 0x00000081));
        assert!(Sighash::is_defined(SignatureVersion::ForkId, 0x000000C2));
        assert!(Sighash::is_defined(SignatureVersion::ForkId, 0x00000043));
    }

    #[test]
    fn test_blake_2b_personal() {
        let hash = blake_2b_256_personal(b"", b"ZcashPrevoutHash");
        assert_eq!(
            H256::from("d53a633bbecf82fe9e9484d8a0e727c73bb9e68c96e72dec30144f6a84afa136"),
            hash
        );
    }

    // https://github.com/zcash/zips/blob/master/zip-0243.rst#test-vector-3
    // The preimage and hash in Zcash example are invalid.
    // scriptCode length should be appended to Input part first but it's not there in example.
    #[test]
    fn test_sapling_sig_hash() {
        let tx: Transaction = "0400008085202f8901a8c685478265f4c14dada651969c45a65e1aeb8cd6791f2f5bb6a1d9952104d9010000006b483045022100a61e5d557568c2ddc1d9b03a7173c6ce7c996c4daecab007ac8f34bee01e6b9702204d38fdc0bcf2728a69fde78462a10fb45a9baa27873e6a5fc45fb5c76764202a01210365ffea3efa3908918a8b8627724af852fc9b86d7375b103ab0543cf418bcaa7ffeffffff02005a6202000000001976a9148132712c3ff19f3a151234616777420a6d7ef22688ac8b959800000000001976a9145453e4698f02a38abdaa521cd1ff2dee6fac187188ac29b0040048b004000000000000000000000000".into();
        let mut signer = TransactionInputSigner::from(tx);
        signer.inputs[0].amount = 50000000;
        signer.consensus_branch_id = 0x76b809bb;

        let sig_hash = Sighash::from_u32(SignatureVersion::Base, 1);
        let hash = signer.signature_hash_overwintered(
            0,
            &Script::from("1976a914507173527b4c3318a2aecd793bf1cfed705950cf88ac"),
            1,
            sig_hash,
        );

        assert_eq!(
            H256::from("f27411aa9bd02879181c763a80bdb6f9ea9158f0de71757e7e12ed17760ebe3f"),
            hash.unwrap()
        );

        let hash = signer.signature_hash(
            0,
            0,
            &Script::from("1976a914507173527b4c3318a2aecd793bf1cfed705950cf88ac"),
            SignatureVersion::Base,
            1,
        );

        assert_eq!(
            H256::from("f27411aa9bd02879181c763a80bdb6f9ea9158f0de71757e7e12ed17760ebe3f"),
            hash
        );
    }

    #[test]
    fn test_sapling_sig_hash_2() {
        let tx: Transaction = "0400008085202f89012c07a03638d9cf4d2cc837784b3b06aa9a5c8b819f7cb0d373bf711108f4c0f2010000006b483045022100fceec7ffa2686377fa2e13d43aa1d8836c3b5ace5292dd2f65a75befec2660bd02205dc000c13a89975bf3fe85aa9c891fcdea6eb25bd5459ad204fe2946d22e49c3012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffff0240420f00000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788ac7c288800000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788ac00000000000000000000000000000000000000".into();
        let mut signer = TransactionInputSigner::from(tx);
        signer.inputs[0].amount = 9924260;
        signer.consensus_branch_id = 0x76b809bb;

        let sig_hash = Sighash::from_u32(SignatureVersion::Base, 1);
        let hash = signer.signature_hash_overwintered(
            0,
            &Script::from("76a91405aab5342166f8594baf17a7d9bef5d56744332788ac"),
            1,
            sig_hash,
        );

        assert_eq!(
            H256::from("047da0d9932545770fc570122c4451b53fadad219650008e5026162e957a46f9"),
            hash.unwrap()
        );

        let hash = signer.signature_hash(
            0,
            0,
            &Script::from("76a91405aab5342166f8594baf17a7d9bef5d56744332788ac"),
            SignatureVersion::Base,
            1,
        );

        assert_eq!(
            H256::from("047da0d9932545770fc570122c4451b53fadad219650008e5026162e957a46f9"),
            hash
        );
    }

    #[test]
    fn test_sapling_sig_hash_3() {
        let tx: Transaction = "0400008085202f890162878ceccb1d4ea904681f3fffad22ec79aeea7e83f117a579ec60859ab9ee3e0000000000feffffff00e5c29360c701000010460afaffffffff00015280edc62c0e174cc112dfcc5c47c34d74d119bca3850c54bbe6383ed4d0d9f242c8cb5819674460802bd39257bf0bb7c7b5e9f9efb051e6f48d681f088c916e713187a7fcbce4eb6ec7f35d5bbdab1225ded98e2f845cddae36ebf45c9e7e00e28614c05416da18888e3883e9667e2df9a513612fe41df4c462b28473c1f10aa03cec99d1e4e23cd84d1bfee827ec9077ddb0d0537660017778e880c7835c9d81e4d8cfb1def0506805cac8fd3ebf2f231d15fbd9807e0ec7c50d26b24f8f07b21552dc0abc0feb4e7ee7ee40c620a07160a87a4683801539e4b22f0778960dd0a6e2b8d6b09cd5e0fbf40484340f9a88c0bea726d6221244ea5ba859f04af0dc8798c4e4379620bad0b7ac9093d877f23431f30a39173f8a2db2a6665a0cc0180f9eab676cac85aeeee14ec53abbb94f977b01437310c4543a390e4e4838c6a3b9de82d63ed7c4032d9c32d631adb477391a30807fffd33f98b8ed72b0684eb9f6afd02a4d9c935353448103d490f7095a725f75d75c3539d4e9ea7b76a725d4898403e9047288ad2ba46bec0e130f5ed160e4b2cf7a0db768a7a1f158baf067f6996f171269fa41df02c5b588fd12d430b40f8763b1d2f4b234f6a490ae90266c262fa8ec8da3172e87064d97d3a2a2700b5215739196553fe2d69db970f8b8d0252439f7829dcc7d8ec17dfa85e53320e26650876888a7505327b03abecb91b6367e4c3feada297ae0eff32ce0ff3780d924cb2f19ae97bb648fa6a49e6a8677ef4aa08b55bc9ae77ac6a2a7ad26c3c7ac718ef263ea53fbad012c641aed9997221a283743e337e97aa62ef6db3ae13835f4b572000f8c5a32b73aecfc7ae68b2b9924b6dde7c6fcca207e25feae0a024c4fcfd0207e9ad346b36af5fd581769cac99daeab680c593a617d9571ed5c247846d14cecda2acfe9cfbbb22408c1bc3bb7da3f7ac4a8ca0726ba01ee4531b5036fcc376970998b617d6cef7bbe4bdbf77d6adaebbdc0542ea60ff25b30cde6764777d1f821fd60f91499ccb3aac20d5ccd01ed313a53ac33bcbdaca460209aaa0e94feb16a82cbaa902210644777a21862c7dbc30df403c75e04dc47696230cf40dfa1045f480abdbebf4ebcbac6f700f287cef3d4b147ddf0e8e9d02ee9baa51604682d5b983dc6f8abee274be0e02a8a7de3fbb22563726c2e7d34b15c437e560e7ce1569ea2ed16e37d908e60af15fd44e46b8969fa74a0f24147553947da7db10fa3394c54413f1b4d6dfcab790a3a91027a1947a229644da8663f05bd05ff4a8621db679467ab74bf8eab1be5861f80b004af53cb679be479e82aa3de8ae5002521474e197975860a9b506e698ffaa385803c656dd984b60b0569ad617c6347722fddbde10f0f485362fbedfde0a600bf551c3f8b02778c4d6960a533fcb302".into();
        let mut signer = TransactionInputSigner::from(tx);
        signer.inputs[0].amount = 100000000;
        signer.consensus_branch_id = 0x76b809bb;

        let sig_hash = Sighash::from_u32(SignatureVersion::Base, 1);
        let hash = signer.signature_hash_overwintered(
			0,
			&Script::from("6304e5928060b17521031c632dad67a611de77d9666cbc61e65957c7d7544c25e384f4e76de729e6a1bfac6782012088a914b78f0b837e2c710f8b28e59d06473d489e5315c88821037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5ac68"),
			1,
			sig_hash
		);

        assert_eq!(
            H256::from("4e8465af6add3acdc3bbc05d97ab1aef8e9d66784c29f94b2551ba25f3d90054"),
            hash.unwrap().reversed()
        );

        let hash = signer.signature_hash(
			0,
            0,
			&Script::from("6304e5928060b17521031c632dad67a611de77d9666cbc61e65957c7d7544c25e384f4e76de729e6a1bfac6782012088a914b78f0b837e2c710f8b28e59d06473d489e5315c88821037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5ac68"),
			SignatureVersion::Base,
            1
		);

        assert_eq!(
            H256::from("4e8465af6add3acdc3bbc05d97ab1aef8e9d66784c29f94b2551ba25f3d90054"),
            hash.reversed()
        );
    }
}

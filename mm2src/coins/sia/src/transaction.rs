use crate::encoding::{Encodable, Encoder, HexArray64, PrefixedH256, PrefixedPublicKey, PrefixedSignature};
use crate::spend_policy::{SpendPolicy, SpendPolicyHelper, UnlockCondition, UnlockKey};
use crate::types::{Address, ChainIndex};
use ed25519_dalek::{PublicKey, Signature};
use rpc::v1::types::H256;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use serde_with::{serde_as, FromInto};
use std::str::FromStr;

type SiacoinOutputID = H256;
const V2_REPLAY_PREFIX: u8 = 2;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Currency {
    lo: u64,
    hi: u64,
}

// TODO does this also need to be able to deserialize from an integer?
// walletd API returns this as a string
impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CurrencyVisitor;

        impl<'de> serde::de::Visitor<'de> for CurrencyVisitor {
            type Value = Currency;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string representing a u128 value")
            }

            fn visit_str<E>(self, value: &str) -> Result<Currency, E>
            where
                E: serde::de::Error,
            {
                let u128_value = u128::from_str(value).map_err(E::custom)?;
                let lo = u128_value as u64;
                let hi = (u128_value >> 64) as u64;
                Ok(Currency::new(lo, hi))
            }
        }

        deserializer.deserialize_str(CurrencyVisitor)
    }
}

impl Serialize for Currency {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_u128().to_string())
    }
}

impl Currency {
    pub fn new(lo: u64, hi: u64) -> Self { Currency { lo, hi } }

    pub fn to_u128(&self) -> u128 { ((self.hi as u128) << 64) | (self.lo as u128) }
}

impl From<u64> for Currency {
    fn from(value: u64) -> Self { Currency { lo: value, hi: 0 } }
}

// Currency remains the same data structure between V1 and V2 however the encoding changes
#[derive(Clone, Debug)]
pub enum CurrencyVersion<'a> {
    V1(&'a Currency),
    V2(&'a Currency),
}

impl<'a> Encodable for CurrencyVersion<'a> {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            CurrencyVersion::V1(currency) => {
                let mut buffer = [0u8; 16];

                buffer[8..].copy_from_slice(&currency.lo.to_be_bytes());
                buffer[..8].copy_from_slice(&currency.hi.to_be_bytes());

                // Trim leading zero bytes from the buffer
                let trimmed_buf = match buffer.iter().position(|&x| x != 0) {
                    Some(index) => &buffer[index..],
                    None => &buffer[..], // In case all bytes are zero
                };
                encoder.write_len_prefixed_bytes(trimmed_buf);
            },
            CurrencyVersion::V2(currency) => {
                encoder.write_u64(currency.lo);
                encoder.write_u64(currency.hi);
            },
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SatisfiedPolicy {
    #[serde_as(as = "FromInto<SpendPolicyHelper>")]
    pub policy: SpendPolicy,
    #[serde_as(as = "Vec<FromInto<PrefixedSignature>>")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<Signature>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preimages: Vec<Vec<u8>>,
}

impl Encodable for Signature {
    fn encode(&self, encoder: &mut Encoder) { encoder.write_slice(&self.to_bytes()); }
}

impl Encodable for SatisfiedPolicy {
    fn encode(&self, encoder: &mut Encoder) {
        self.policy.encode(encoder);
        let mut sigi: usize = 0;
        let mut prei: usize = 0;

        fn rec(policy: &SpendPolicy, encoder: &mut Encoder, sigi: &mut usize, prei: &mut usize, sp: &SatisfiedPolicy) {
            match policy {
                SpendPolicy::PublicKey(_) => {
                    if *sigi < sp.signatures.len() {
                        sp.signatures[*sigi].encode(encoder);
                        *sigi += 1;
                    } else {
                        // Sia Go code panics here but our code assumes encoding will always be successful
                        // TODO: check if Sia Go will fix this
                        encoder.write_string("Broken PublicKey encoding, see SatisfiedPolicy::encode")
                    }
                },
                SpendPolicy::Hash(_) => {
                    if *prei < sp.preimages.len() {
                        encoder.write_len_prefixed_bytes(&sp.preimages[*prei]);
                        *prei += 1;
                    } else {
                        // Sia Go code panics here but our code assumes encoding will always be successful
                        // consider changing the signature of encode() to return a Result
                        encoder.write_string("Broken Hash encoding, see SatisfiedPolicy::encode")
                    }
                },
                SpendPolicy::Threshold { n: _, of } => {
                    for p in of {
                        rec(p, encoder, sigi, prei, sp);
                    }
                },
                SpendPolicy::UnlockConditions(uc) => {
                    for unlock_key in &uc.unlock_keys {
                        if let UnlockKey::Ed25519(public_key) = unlock_key {
                            rec(&SpendPolicy::PublicKey(*public_key), encoder, sigi, prei, sp);
                        }
                        // else FIXME consider when this is possible, is it always developer error or could it be forced maliciously?
                    }
                },
                _ => {},
            }
        }

        rec(&self.policy, encoder, &mut sigi, &mut prei, self);
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StateElement {
    #[serde_as(as = "FromInto<PrefixedH256>")]
    pub id: H256,
    pub leaf_index: u64,
    #[serde_as(as = "Option<Vec<FromInto<PrefixedH256>>>")]
    pub merkle_proof: Option<Vec<H256>>,
}

impl Encodable for StateElement {
    fn encode(&self, encoder: &mut Encoder) {
        self.id.encode(encoder);
        encoder.write_u64(self.leaf_index);

        match &self.merkle_proof {
            Some(proof) => {
                encoder.write_u64(proof.len() as u64);
                for p in proof {
                    p.encode(encoder);
                }
            },
            None => {
                encoder.write_u64(0u64);
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SiafundElement {
    #[serde(flatten)]
    pub state_element: StateElement,
    pub siafund_output: SiafundOutput,
    pub claim_start: Currency,
}

impl Encodable for SiafundElement {
    fn encode(&self, encoder: &mut Encoder) {
        self.state_element.encode(encoder);
        SiafundOutputVersion::V2(&self.siafund_output).encode(encoder);
        CurrencyVersion::V2(&self.claim_start).encode(encoder);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SiacoinElement {
    #[serde(flatten)]
    pub state_element: StateElement,
    pub siacoin_output: SiacoinOutput,
    pub maturity_height: u64,
}

impl Encodable for SiacoinElement {
    fn encode(&self, encoder: &mut Encoder) {
        self.state_element.encode(encoder);
        SiacoinOutputVersion::V2(&self.siacoin_output).encode(encoder);
        encoder.write_u64(self.maturity_height);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SiafundInputV2 {
    pub parent: SiafundElement,
    pub claim_address: Address,
    pub satisfied_policy: SatisfiedPolicy,
}

impl Encodable for SiafundInputV2 {
    fn encode(&self, encoder: &mut Encoder) {
        self.parent.encode(encoder);
        self.claim_address.encode(encoder);
        self.satisfied_policy.encode(encoder);
    }
}

// https://github.com/SiaFoundation/core/blob/6c19657baf738c6b730625288e9b5413f77aa659/types/types.go#L197-L198
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SiacoinInputV1 {
    pub parent_id: SiacoinOutputID,
    pub unlock_condition: UnlockCondition,
}

impl Encodable for SiacoinInputV1 {
    fn encode(&self, encoder: &mut Encoder) {
        self.parent_id.encode(encoder);
        self.unlock_condition.encode(encoder);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SiacoinInputV2 {
    pub parent: SiacoinElement,
    pub satisfied_policy: SatisfiedPolicy,
}

impl Encodable for SiacoinInputV2 {
    fn encode(&self, encoder: &mut Encoder) {
        self.parent.encode(encoder);
        self.satisfied_policy.encode(encoder);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SiafundOutput {
    pub value: u64,
    pub address: Address,
}

// SiafundOutput remains the same data structure between V1 and V2 however the encoding changes
#[derive(Clone, Debug)]
pub enum SiafundOutputVersion<'a> {
    V1(&'a SiafundOutput),
    V2(&'a SiafundOutput),
}

impl<'a> Encodable for SiafundOutputVersion<'a> {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            SiafundOutputVersion::V1(v1) => {
                CurrencyVersion::V1(&Currency::from(v1.value)).encode(encoder);
                v1.address.encode(encoder);
            },
            SiafundOutputVersion::V2(v2) => {
                encoder.write_u64(v2.value);
                v2.address.encode(encoder);
            },
        }
    }
}

// SiacoinOutput remains the same data structure between V1 and V2 however the encoding changes
#[derive(Clone, Debug)]
pub enum SiacoinOutputVersion<'a> {
    V1(&'a SiacoinOutput),
    V2(&'a SiacoinOutput),
}

impl<'a> Encodable for SiacoinOutputVersion<'a> {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            SiacoinOutputVersion::V1(v1) => {
                CurrencyVersion::V1(&v1.value).encode(encoder);
                v1.address.encode(encoder);
            },
            SiacoinOutputVersion::V2(v2) => {
                CurrencyVersion::V2(&v2.value).encode(encoder);
                v2.address.encode(encoder);
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SiacoinOutput {
    pub value: Currency,
    pub address: Address,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoveredFields {
    pub whole_transaction: bool,
    pub siacoin_inputs: Vec<u64>,
    pub siacoin_outputs: Vec<u64>,
    pub file_contracts: Vec<u64>,
    pub file_contract_revisions: Vec<u64>,
    pub storage_proofs: Vec<u64>,
    pub siafund_inputs: Vec<u64>,
    pub siafund_outputs: Vec<u64>,
    pub miner_fees: Vec<u64>,
    pub arbitrary_data: Vec<u64>,
    pub signatures: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TransactionSignature {
    pub parent_id: H256,
    pub public_key_index: u64,
    pub timelock: u64,
    pub covered_fields: CoveredFields,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileContract {
    pub filesize: u64,
    pub file_merkle_root: H256,
    pub window_start: u64,
    pub window_end: u64,
    pub payout: Currency,
    pub valid_proof_outputs: Vec<SiacoinOutput>,
    pub missed_proof_outputs: Vec<SiacoinOutput>,
    pub unlock_hash: H256,
    pub revision_number: u64,
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct V2FileContract {
    pub filesize: u64,
    #[serde_as(as = "FromInto<PrefixedH256>")]
    pub file_merkle_root: H256,
    pub proof_height: u64,
    pub expiration_height: u64,
    pub renter_output: SiacoinOutput,
    pub host_output: SiacoinOutput,
    pub missed_host_value: Currency,
    pub total_collateral: Currency,
    #[serde_as(as = "FromInto<PrefixedPublicKey>")]
    pub renter_public_key: PublicKey,
    #[serde_as(as = "FromInto<PrefixedPublicKey>")]
    pub host_public_key: PublicKey,
    pub revision_number: u64,
    #[serde_as(as = "FromInto<PrefixedSignature>")]
    pub renter_signature: Signature,
    #[serde_as(as = "FromInto<PrefixedSignature>")]
    pub host_signature: Signature,
}

impl V2FileContract {
    pub fn with_nil_sigs(&self) -> V2FileContract {
        V2FileContract {
            renter_signature: Signature::from_bytes(&[0u8; 64]).expect("Err unreachable"),
            host_signature: Signature::from_bytes(&[0u8; 64]).expect("Err unreachable"),
            ..self.clone()
        }
    }
}

impl Encodable for V2FileContract {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.write_u64(self.filesize);
        self.file_merkle_root.encode(encoder);
        encoder.write_u64(self.proof_height);
        encoder.write_u64(self.expiration_height);
        SiacoinOutputVersion::V2(&self.renter_output).encode(encoder);
        SiacoinOutputVersion::V2(&self.host_output).encode(encoder);
        CurrencyVersion::V2(&self.missed_host_value).encode(encoder);
        CurrencyVersion::V2(&self.total_collateral).encode(encoder);
        self.renter_public_key.encode(encoder);
        self.host_public_key.encode(encoder);
        encoder.write_u64(self.revision_number);
        self.renter_signature.encode(encoder);
        self.host_signature.encode(encoder);
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct V2FileContractElement {
    #[serde(flatten)]
    pub state_element: StateElement,
    pub v2_file_contract: V2FileContract,
}

impl Encodable for V2FileContractElement {
    fn encode(&self, encoder: &mut Encoder) {
        self.state_element.encode(encoder);
        self.v2_file_contract.encode(encoder);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct FileContractRevisionV2 {
    pub parent: V2FileContractElement,
    pub revision: V2FileContract,
}

impl FileContractRevisionV2 {
    pub fn with_nil_sigs(&self) -> FileContractRevisionV2 {
        FileContractRevisionV2 {
            revision: self.revision.with_nil_sigs(),
            ..self.clone()
        }
    }
}

impl Encodable for FileContractRevisionV2 {
    fn encode(&self, encoder: &mut Encoder) {
        self.parent.encode(encoder);
        self.revision.encode(encoder);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attestation {
    pub public_key: PublicKey,
    pub key: String,
    pub value: Vec<u8>,
    pub signature: Signature,
}

impl Encodable for Attestation {
    fn encode(&self, encoder: &mut Encoder) {
        self.public_key.encode(encoder);
        encoder.write_string(&self.key);
        encoder.write_len_prefixed_bytes(&self.value);
        self.signature.encode(encoder);
    }
}
#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StorageProof {
    pub parent_id: FileContractID,
    pub leaf: HexArray64,
    pub proof: Vec<H256>,
}

type SiafundOutputID = H256;
type FileContractID = H256;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileContractRevision {
    pub parent_id: FileContractID,
    pub unlock_condition: UnlockCondition,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SiafundInputV1 {
    pub parent_id: SiafundOutputID,
    pub unlock_condition: UnlockCondition,
    pub claim_address: Address,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ResolutionType {
    Renewal,
    StorageProof,
    Expiration,
    Finalization,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct V2FileContractResolution {
    pub parent: V2FileContractElement,
    #[serde(rename = "type")]
    pub resolution_type: ResolutionType,
    pub resolution: V2FileContractResolutionWrapper,
}

impl Encodable for V2FileContractResolution {
    fn encode(&self, _encoder: &mut Encoder) { todo!() }
}

impl<'de> Deserialize<'de> for V2FileContractResolution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Debug)]
        struct V2FileContractResolutionHelper {
            parent: V2FileContractElement,
            #[serde(rename = "type")]
            resolution_type: ResolutionType,
            resolution: Value,
        }

        let helper = V2FileContractResolutionHelper::deserialize(deserializer)?;

        let resolution_data = match helper.resolution_type {
            ResolutionType::Renewal => serde_json::from_value::<V2FileContractRenewal>(helper.resolution)
                .map(|data| V2FileContractResolutionWrapper::Renewal(Box::new(data)))
                .map_err(serde::de::Error::custom),
            ResolutionType::StorageProof => serde_json::from_value::<V2StorageProof>(helper.resolution)
                .map(V2FileContractResolutionWrapper::StorageProof)
                .map_err(serde::de::Error::custom),
            ResolutionType::Finalization => serde_json::from_value::<V2FileContractFinalization>(helper.resolution)
                .map(|data| V2FileContractResolutionWrapper::Finalization(Box::new(data)))
                .map_err(serde::de::Error::custom),
            // expiration is a special case because it has no data. It is just an empty object, "{}".
            ResolutionType::Expiration => match &helper.resolution {
                Value::Object(map) if map.is_empty() => Ok(V2FileContractResolutionWrapper::Expiration),
                _ => Err(serde::de::Error::custom("expected an empty map for expiration")),
            },
        }?;

        Ok(V2FileContractResolution {
            parent: helper.parent,
            resolution_type: helper.resolution_type,
            resolution: resolution_data,
        })
    }
}

impl Encodable for V2FileContractResolutionWrapper {
    fn encode(&self, _encoder: &mut Encoder) {
        todo!();
    }
}

impl V2FileContractResolution {
    fn with_nil_sigs(&self) -> V2FileContractResolution {
        V2FileContractResolution {
            resolution: self.resolution.with_nil_sigs(),
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum V2FileContractResolutionWrapper {
    Finalization(Box<V2FileContractFinalization>),
    Renewal(Box<V2FileContractRenewal>),
    StorageProof(V2StorageProof),
    #[serde(serialize_with = "serialize_variant_as_empty_object")]
    Expiration,
}

fn serialize_variant_as_empty_object<S>(serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str("{}")
}

impl V2FileContractResolutionWrapper {
    fn with_nil_sigs(&self) -> V2FileContractResolutionWrapper {
        match self {
            V2FileContractResolutionWrapper::Finalization(f) => {
                V2FileContractResolutionWrapper::Finalization(Box::new(f.with_nil_sigs()))
            },
            V2FileContractResolutionWrapper::Renewal(r) => {
                V2FileContractResolutionWrapper::Renewal(Box::new(r.with_nil_sigs()))
            },
            V2FileContractResolutionWrapper::StorageProof(s) => {
                V2FileContractResolutionWrapper::StorageProof(s.with_nil_merkle_proof())
            },
            V2FileContractResolutionWrapper::Expiration => V2FileContractResolutionWrapper::Expiration,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct V2FileContractFinalization(pub V2FileContract);

impl V2FileContractFinalization {
    fn with_nil_sigs(&self) -> V2FileContractFinalization { V2FileContractFinalization(self.0.with_nil_sigs()) }
}

// TODO unit test
impl Encodable for V2FileContractFinalization {
    fn encode(&self, encoder: &mut Encoder) { self.0.encode(encoder); }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct V2FileContractRenewal {
    final_revision: V2FileContract,
    new_contract: V2FileContract,
    renter_rollover: Currency,
    host_rollover: Currency,
    #[serde_as(as = "FromInto<PrefixedSignature>")]
    renter_signature: Signature,
    #[serde_as(as = "FromInto<PrefixedSignature>")]
    host_signature: Signature,
}

impl V2FileContractRenewal {
    pub fn with_nil_sigs(&self) -> V2FileContractRenewal {
        debug_assert!(
            Signature::from_bytes(&[0u8; 64]).is_ok(),
            "nil signature is valid and cannot return Err"
        );
        V2FileContractRenewal {
            final_revision: self.final_revision.with_nil_sigs(),
            new_contract: self.new_contract.with_nil_sigs(),
            renter_signature: Signature::from_bytes(&[0u8; 64]).expect("Err unreachable"),
            host_signature: Signature::from_bytes(&[0u8; 64]).expect("Err unreachable"),
            ..self.clone()
        }
    }
}

// TODO unit test
impl Encodable for V2FileContractRenewal {
    fn encode(&self, encoder: &mut Encoder) {
        self.final_revision.encode(encoder);
        self.new_contract.encode(encoder);
        CurrencyVersion::V2(&self.renter_rollover).encode(encoder);
        CurrencyVersion::V2(&self.host_rollover).encode(encoder);
        self.renter_signature.encode(encoder);
        self.host_signature.encode(encoder);
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct V2StorageProof {
    proof_index: ChainIndexElement,
    leaf: HexArray64,
    proof: Vec<H256>,
}

impl V2StorageProof {
    pub fn with_nil_merkle_proof(&self) -> V2StorageProof {
        V2StorageProof {
            proof_index: ChainIndexElement {
                state_element: StateElement {
                    merkle_proof: None,
                    ..self.proof_index.state_element.clone()
                },
                ..self.proof_index.clone()
            },
            ..self.clone()
        }
    }
}

// TODO unit test
impl Encodable for V2StorageProof {
    fn encode(&self, encoder: &mut Encoder) {
        self.proof_index.encode(encoder);
        encoder.write_slice(&self.leaf.0);
        encoder.write_u64(self.proof.len() as u64);
        for proof in &self.proof {
            proof.encode(encoder);
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChainIndexElement {
    #[serde(flatten)]
    pub state_element: StateElement,
    pub chain_index: ChainIndex,
}

// TODO unit test
impl Encodable for ChainIndexElement {
    fn encode(&self, encoder: &mut Encoder) {
        self.state_element.encode(encoder);
        self.chain_index.encode(encoder);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileContractElementV1 {
    #[serde(flatten)]
    pub state_element: StateElement,
    pub file_contract: FileContractV1,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileContractV1 {
    pub filesize: u64,
    pub file_merkle_root: H256,
    pub window_start: u64,
    pub window_end: u64,
    pub payout: Currency,
    pub valid_proof_outputs: Vec<SiacoinOutput>,
    pub missed_proof_outputs: Vec<SiacoinOutput>,
    pub unlock_hash: H256,
    pub revision_number: u64,
}
/*
While implementing this, we faced two options.
    1.) Treat every field as an Option<>
    2.) Always initialize every empty field as a Vec<>

We chose the latter as it allows for simpler encoding of this struct.
It is possible this may need to change in later implementations.
*/
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct V1Transaction {
    pub siacoin_inputs: Vec<SiacoinInputV1>,
    pub siacoin_outputs: Vec<SiacoinOutput>,
    pub file_contracts: Vec<FileContract>,
    pub file_contract_revisions: Vec<FileContractRevision>,
    pub storage_proofs: Vec<StorageProof>,
    pub siafund_inputs: Vec<SiafundInputV1>,
    pub siafund_outputs: Vec<SiafundOutput>,
    pub miner_fees: Vec<Currency>,
    pub arbitrary_data: Vec<u8>,
    pub signatures: Vec<TransactionSignature>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct V2Transaction {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub siacoin_inputs: Vec<SiacoinInputV2>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub siacoin_outputs: Vec<SiacoinOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub siafund_inputs: Vec<SiafundInputV2>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub siafund_outputs: Vec<SiafundOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub file_contracts: Vec<V2FileContract>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub file_contract_revisions: Vec<FileContractRevisionV2>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub file_contract_resolutions: Vec<V2FileContractResolution>, // TODO needs Encodable trait
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attestations: Vec<Attestation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub arbitrary_data: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_foundation_address: Option<Address>,
    pub miner_fee: Currency,
}

impl V2Transaction {
    pub fn with_nil_sigs(&self) -> V2Transaction {
        V2Transaction {
            file_contracts: self.file_contracts.clone(),
            file_contract_revisions: self.file_contract_revisions.clone(),
            file_contract_resolutions: self.file_contract_resolutions.clone(),
            ..self.clone()
        }
    }

    pub fn input_sig_hash(&self) -> H256 {
        let mut encoder = Encoder::default();
        encoder.write_distinguisher("sig/input");
        encoder.write_u8(V2_REPLAY_PREFIX);
        self.encode(&mut encoder);
        encoder.hash()
    }
}

// this encoding corresponds to the Go implementation's "V2TransactionSemantics" rather than "V2Transaction"
impl Encodable for V2Transaction {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.write_u64(self.siacoin_inputs.len() as u64);
        for si in &self.siacoin_inputs {
            si.parent.state_element.id.encode(encoder);
        }

        encoder.write_u64(self.siacoin_outputs.len() as u64);
        for so in &self.siacoin_outputs {
            SiacoinOutputVersion::V2(so).encode(encoder);
        }

        encoder.write_u64(self.siafund_inputs.len() as u64);
        for si in &self.siafund_inputs {
            si.parent.state_element.id.encode(encoder);
        }

        encoder.write_u64(self.siafund_outputs.len() as u64);
        for so in &self.siafund_outputs {
            SiafundOutputVersion::V2(so).encode(encoder);
        }

        encoder.write_u64(self.file_contracts.len() as u64);
        for fc in &self.file_contracts {
            fc.with_nil_sigs().encode(encoder);
        }

        encoder.write_u64(self.file_contract_revisions.len() as u64);
        for fcr in &self.file_contract_revisions {
            fcr.parent.state_element.id.encode(encoder);
            fcr.revision.with_nil_sigs().encode(encoder);
        }

        encoder.write_u64(self.file_contract_resolutions.len() as u64);
        for fcr in &self.file_contract_resolutions {
            fcr.parent.state_element.id.encode(encoder);
            fcr.with_nil_sigs().encode(encoder);
            // FIXME .encode() leads to unimplemented!()
        }

        encoder.write_u64(self.attestations.len() as u64);
        for att in &self.attestations {
            att.encode(encoder);
        }

        encoder.write_len_prefixed_bytes(&self.arbitrary_data);

        encoder.write_bool(self.new_foundation_address.is_some());
        match &self.new_foundation_address {
            Some(addr) => addr.encode(encoder),
            None => (),
        }
        CurrencyVersion::V2(&self.miner_fee).encode(encoder);
    }
}

use crate::blake2b_internal::{public_key_leaf, sigs_required_leaf, standard_unlock_hash, timelock_leaf, Accumulator};
use crate::encoding::{Encodable, Encoder, PrefixedH256, PrefixedPublicKey};
use crate::specifier::Specifier;
use crate::types::Address;
use ed25519_dalek::PublicKey;
use nom::bytes::complete::{take_until, take_while, take_while_m_n};
use nom::character::complete::char;
use nom::combinator::all_consuming;
use nom::combinator::map_res;
use nom::IResult;
use rpc::v1::types::H256;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

const POLICY_VERSION: u8 = 1u8;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum SpendPolicy {
    Above(u64),
    After(u64),
    PublicKey(PublicKey),
    Hash(H256),
    Threshold { n: u8, of: Vec<SpendPolicy> },
    Opaque(Address),
    UnlockConditions(UnlockCondition), // For v1 compatibility
}

// Helper to serialize/deserialize SpendPolicy with prefixed PublicKey and H256
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", content = "policy", rename_all = "camelCase")]
pub enum SpendPolicyHelper {
    Above(u64),
    After(u64),
    Pk(PrefixedPublicKey),
    H(PrefixedH256),
    Thresh { n: u8, of: Vec<SpendPolicyHelper> },
    Opaque(Address),
    Uc(UnlockCondition), // For v1 compatibility
}

impl From<SpendPolicyHelper> for SpendPolicy {
    fn from(helper: SpendPolicyHelper) -> Self {
        match helper {
            SpendPolicyHelper::Above(height) => SpendPolicy::Above(height),
            SpendPolicyHelper::After(time) => SpendPolicy::After(time),
            SpendPolicyHelper::Pk(pk) => SpendPolicy::PublicKey(pk.0),
            SpendPolicyHelper::H(hash) => SpendPolicy::Hash(hash.0),
            SpendPolicyHelper::Thresh { n, of } => SpendPolicy::Threshold {
                n,
                of: of.into_iter().map(SpendPolicy::from).collect(),
            },
            SpendPolicyHelper::Opaque(address) => SpendPolicy::Opaque(address),
            SpendPolicyHelper::Uc(uc) => SpendPolicy::UnlockConditions(uc),
        }
    }
}

impl From<SpendPolicy> for SpendPolicyHelper {
    fn from(policy: SpendPolicy) -> Self {
        match policy {
            SpendPolicy::Above(height) => SpendPolicyHelper::Above(height),
            SpendPolicy::After(time) => SpendPolicyHelper::After(time),
            SpendPolicy::PublicKey(pk) => SpendPolicyHelper::Pk(PrefixedPublicKey(pk)),
            SpendPolicy::Hash(hash) => SpendPolicyHelper::H(PrefixedH256(hash)),
            SpendPolicy::Threshold { n, of } => SpendPolicyHelper::Thresh {
                n,
                of: of.into_iter().map(SpendPolicyHelper::from).collect(),
            },
            SpendPolicy::Opaque(address) => SpendPolicyHelper::Opaque(address),
            SpendPolicy::UnlockConditions(uc) => SpendPolicyHelper::Uc(uc),
        }
    }
}

impl Encodable for SpendPolicy {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.write_u8(POLICY_VERSION);
        self.encode_wo_prefix(encoder);
    }
}

impl SpendPolicy {
    pub fn to_u8(&self) -> u8 {
        match self {
            SpendPolicy::Above(_) => 1,
            SpendPolicy::After(_) => 2,
            SpendPolicy::PublicKey(_) => 3,
            SpendPolicy::Hash(_) => 4,
            SpendPolicy::Threshold { n: _, of: _ } => 5,
            SpendPolicy::Opaque(_) => 6,
            SpendPolicy::UnlockConditions(_) => 7,
        }
    }

    pub fn encode_wo_prefix(&self, encoder: &mut Encoder) {
        let opcode = self.to_u8();
        match self {
            SpendPolicy::Above(height) => {
                encoder.write_u8(opcode);
                encoder.write_u64(*height);
            },
            SpendPolicy::After(time) => {
                encoder.write_u8(opcode);
                encoder.write_u64(*time);
            },
            SpendPolicy::PublicKey(pubkey) => {
                encoder.write_u8(opcode);
                encoder.write_slice(&pubkey.to_bytes());
            },
            SpendPolicy::Hash(hash) => {
                encoder.write_u8(opcode);
                encoder.write_slice(&hash.0);
            },
            SpendPolicy::Threshold { n, of } => {
                encoder.write_u8(opcode);
                encoder.write_u8(*n);
                encoder.write_u8(of.len() as u8);
                for policy in of {
                    policy.encode_wo_prefix(encoder);
                }
            },
            SpendPolicy::Opaque(address) => {
                encoder.write_u8(opcode);
                encoder.write_slice(&address.0 .0);
            },
            SpendPolicy::UnlockConditions(unlock_condition) => {
                encoder.write_u8(opcode);
                encoder.write_u64(unlock_condition.timelock);
                encoder.write_u64(unlock_condition.unlock_keys.len() as u64);
                for uc in &unlock_condition.unlock_keys {
                    uc.encode(encoder);
                }
                encoder.write_u64(unlock_condition.signatures_required);
            },
        }
    }

    pub fn address(&self) -> Address {
        if let SpendPolicy::UnlockConditions(unlock_condition) = self {
            return unlock_condition.address();
        }
        let mut encoder = Encoder::default();
        encoder.write_distinguisher("address");

        // if self is a threshold policy, we need to convert all of its subpolicies to opaque
        let new_policy = match self {
            SpendPolicy::Threshold { n, of } => SpendPolicy::Threshold {
                n: *n,
                of: of.iter().map(SpendPolicy::opaque).collect(),
            },
            _ => self.clone(),
        };
        new_policy.encode(&mut encoder);

        Address(encoder.hash())
    }

    pub fn above(height: u64) -> Self { SpendPolicy::Above(height) }

    pub fn after(time: u64) -> Self { SpendPolicy::After(time) }

    pub fn public_key(pk: PublicKey) -> Self { SpendPolicy::PublicKey(pk) }

    pub fn hash(h: H256) -> Self { SpendPolicy::Hash(h) }

    pub fn threshold(n: u8, of: Vec<SpendPolicy>) -> Self { SpendPolicy::Threshold { n, of } }

    pub fn opaque(p: &SpendPolicy) -> Self { SpendPolicy::Opaque(p.address()) }

    pub fn anyone_can_spend() -> Self { SpendPolicy::threshold(0, vec![]) }
}

pub fn opacify_policy(p: &SpendPolicy) -> SpendPolicy { SpendPolicy::Opaque(p.address()) }

pub fn spend_policy_atomic_swap(alice: PublicKey, bob: PublicKey, lock_time: u64, hash: H256) -> SpendPolicy {
    let policy_after = SpendPolicy::After(lock_time);
    let policy_hash = SpendPolicy::Hash(hash);

    let policy_success = SpendPolicy::Threshold {
        n: 2,
        of: vec![SpendPolicy::PublicKey(alice), policy_hash],
    };

    let policy_refund = SpendPolicy::Threshold {
        n: 2,
        of: vec![SpendPolicy::PublicKey(bob), policy_after],
    };

    SpendPolicy::Threshold {
        n: 1,
        of: vec![policy_success, policy_refund],
    }
}

pub fn spend_policy_atomic_swap_success(alice: PublicKey, bob: PublicKey, lock_time: u64, hash: H256) -> SpendPolicy {
    match spend_policy_atomic_swap(alice, bob, lock_time, hash) {
        SpendPolicy::Threshold { n, mut of } => {
            of[1] = opacify_policy(&of[1]);
            SpendPolicy::Threshold { n, of }
        },
        _ => unreachable!(),
    }
}

pub fn spend_policy_atomic_swap_refund(alice: PublicKey, bob: PublicKey, lock_time: u64, hash: H256) -> SpendPolicy {
    match spend_policy_atomic_swap(alice, bob, lock_time, hash) {
        SpendPolicy::Threshold { n, mut of } => {
            of[0] = opacify_policy(&of[0]);
            SpendPolicy::Threshold { n, of }
        },
        _ => unreachable!(),
    }
}

// Sia Go v1 technically supports arbitrary length public keys
// We only support ed25519 but must be able to deserialize others
// This data structure deviates from the Go implementation
#[derive(Clone, Debug, PartialEq)]
pub enum UnlockKey {
    Ed25519(PublicKey),
    Unsupported { algorithm: Specifier, public_key: Vec<u8> },
}

impl<'de> Deserialize<'de> for UnlockKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UnlockKeyVisitor;

        impl<'de> serde::de::Visitor<'de> for UnlockKeyVisitor {
            type Value = UnlockKey;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string representing a Sia v1 UnlockKey; most often 'ed25519:<hex>'")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match UnlockKey::from_str(value) {
                    Ok(key) => Ok(key),
                    Err(e) => Err(E::custom(format!("failed to parse UnlockKey: {}", e.0))),
                }
            }
        }

        deserializer.deserialize_str(UnlockKeyVisitor)
    }
}

impl Serialize for UnlockKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

fn parse_specifier(input: &str) -> IResult<&str, Specifier> {
    let (input, prefix_str) = take_until(":")(input)?;
    let specifier = Specifier::from_str_expect(prefix_str);
    let (input, _) = char(':')(input)?;
    Ok((input, specifier))
}

fn parse_unlock_key(input: &str) -> IResult<&str, UnlockKey> {
    let (input, specifier) = parse_specifier(input)?;
    match specifier {
        Specifier::Ed25519 => {
            let (input, public_key) = map_res(
                all_consuming(map_res(
                    take_while_m_n(64, 64, |c: char| c.is_ascii_hexdigit()),
                    hex::decode,
                )),
                |bytes: Vec<u8>| PublicKey::from_bytes(&bytes),
            )(input)?;
            Ok((input, UnlockKey::Ed25519(public_key)))
        },
        _ => {
            let (input, public_key) =
                all_consuming(map_res(take_while(|c: char| c.is_ascii_hexdigit()), |hex_str: &str| {
                    hex::decode(hex_str)
                }))(input)?;
            Ok((input, UnlockKey::Unsupported {
                algorithm: specifier,
                public_key,
            }))
        },
    }
}

#[derive(Debug)]
pub struct UnlockKeyParseError(pub String);

impl FromStr for UnlockKey {
    type Err = UnlockKeyParseError;

    fn from_str(input: &str) -> Result<UnlockKey, Self::Err> {
        match all_consuming(parse_unlock_key)(input) {
            Ok((_, key)) => Ok(key),
            Err(e) => Err(UnlockKeyParseError(e.to_string())), // TODO unit test to check how verbose or useful this is
        }
    }
}

impl fmt::Display for UnlockKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnlockKey::Ed25519(public_key) => write!(f, "ed25519:{}", hex::encode(public_key.as_bytes())),
            UnlockKey::Unsupported { algorithm, public_key } => {
                write!(f, "{}:{}", algorithm, hex::encode(public_key))
            },
        }
    }
}

impl Encodable for PublicKey {
    fn encode(&self, encoder: &mut Encoder) { encoder.write_slice(&self.to_bytes()); }
}

impl Encodable for UnlockKey {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            UnlockKey::Ed25519(public_key) => {
                Specifier::Ed25519.encode(encoder);
                encoder.write_u64(32); // ed25519 public key length
                public_key.encode(encoder);
            },
            UnlockKey::Unsupported { algorithm, public_key } => {
                algorithm.encode(encoder);
                encoder.write_u64(public_key.len() as u64);
                encoder.write_slice(public_key);
            },
        }
    }
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockCondition {
    #[serde(rename = "publicKeys")]
    pub unlock_keys: Vec<UnlockKey>,
    pub timelock: u64,
    pub signatures_required: u64,
}

impl Encodable for UnlockCondition {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.write_u64(self.timelock);
        encoder.write_u64(self.unlock_keys.len() as u64);
        for unlock_key in &self.unlock_keys {
            unlock_key.encode(encoder);
        }
        encoder.write_u64(self.signatures_required);
    }
}

impl UnlockCondition {
    pub fn new(pubkeys: Vec<PublicKey>, timelock: u64, signatures_required: u64) -> Self {
        let unlock_keys = pubkeys.into_iter().map(UnlockKey::Ed25519).collect();

        UnlockCondition {
            unlock_keys,
            timelock,
            signatures_required,
        }
    }

    pub fn standard_unlock(public_key: PublicKey) -> Self {
        UnlockCondition {
            unlock_keys: vec![UnlockKey::Ed25519(public_key)],
            timelock: 0,
            signatures_required: 1,
        }
    }

    pub fn unlock_hash(&self) -> H256 {
        // almost all UnlockConditions are standard, so optimize for that case
        if let UnlockKey::Ed25519(public_key) = &self.unlock_keys[0] {
            if self.timelock == 0 && self.unlock_keys.len() == 1 && self.signatures_required == 1 {
                return standard_unlock_hash(public_key);
            }
        }

        let mut accumulator = Accumulator::default();

        accumulator.add_leaf(timelock_leaf(self.timelock));

        for unlock_key in &self.unlock_keys {
            accumulator.add_leaf(public_key_leaf(unlock_key));
        }

        accumulator.add_leaf(sigs_required_leaf(self.signatures_required));
        accumulator.root()
    }

    pub fn address(&self) -> Address { Address(self.unlock_hash()) }
}

use crate::sia::address::Address;
use crate::sia::blake2b_internal::{public_key_leaf, sigs_required_leaf, standard_unlock_hash, timelock_leaf,
                                   Accumulator, ED25519_IDENTIFIER};
use crate::sia::encoding::Encoder;
use ed25519_dalek::PublicKey;
use rpc::v1::types::H256;

#[cfg(test)] use std::str::FromStr;

const POLICY_VERSION: u8 = 1u8;

#[derive(Debug, Clone)]
pub enum SpendPolicy {
    Above(u64),
    After(u64),
    PublicKey(PublicKey),
    Hash(H256),
    Threshold(PolicyTypeThreshold),
    Opaque(Address),
    UnlockConditions(PolicyTypeUnlockConditions), // For v1 compatibility
}

impl SpendPolicy {
    pub fn to_u8(&self) -> u8 {
        match self {
            SpendPolicy::Above(_) => 1,
            SpendPolicy::After(_) => 2,
            SpendPolicy::PublicKey(_) => 3,
            SpendPolicy::Hash(_) => 4,
            SpendPolicy::Threshold(_) => 5,
            SpendPolicy::Opaque(_) => 6,
            SpendPolicy::UnlockConditions(_) => 7,
        }
    }

    pub fn encode(&self) -> Encoder {
        let mut encoder = Encoder::default();
        encoder.write_u8(POLICY_VERSION);
        encoder.write_slice(&self.encode_wo_prefix().buffer);
        encoder
    }

    pub fn encode_wo_prefix(&self) -> Encoder {
        let mut encoder = Encoder::default();
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
            SpendPolicy::Threshold(PolicyTypeThreshold { n, of }) => {
                encoder.write_u8(opcode);
                encoder.write_u8(*n);
                encoder.write_u8(of.len() as u8);
                for policy in of {
                    encoder.write_slice(&policy.encode_wo_prefix().buffer);
                }
            },
            SpendPolicy::Opaque(p) => {
                encoder.write_u8(opcode);
                encoder.write_slice(&p.0 .0);
            },
            SpendPolicy::UnlockConditions(PolicyTypeUnlockConditions(unlock_condition)) => {
                encoder.write_u8(opcode);
                encoder.write_u64(unlock_condition.timelock);
                encoder.write_u64(unlock_condition.pubkeys.len() as u64);
                for pubkey in &unlock_condition.pubkeys {
                    encoder.write_slice(&ED25519_IDENTIFIER);
                    encoder.write_slice(&pubkey.to_bytes());
                }
                encoder.write_u64(unlock_condition.sigs_required);
            },
        }
        encoder
    }

    fn address(&self) -> Address {
        if let SpendPolicy::UnlockConditions(PolicyTypeUnlockConditions(unlock_condition)) = self {
            return unlock_condition.address();
        }
        let mut encoder = Encoder::default();
        encoder.write_distinguisher("address");

        // if self is a threshold policy, we need to convert all of its subpolicies to opaque
        let mut new_policy = self.clone();
        if let SpendPolicy::Threshold(ref mut p) = new_policy {
            p.of = p.of.iter().map(SpendPolicy::opaque).collect();
        }

        let encoded_policy = new_policy.encode();
        encoder.write_slice(&encoded_policy.buffer);
        Address(encoder.hash())
    }

    pub fn above(height: u64) -> Self { SpendPolicy::Above(height) }

    pub fn after(time: u64) -> Self { SpendPolicy::After(time) }

    pub fn public_key(pk: PublicKey) -> Self { SpendPolicy::PublicKey(pk) }

    pub fn hash(h: H256) -> Self { SpendPolicy::Hash(h) }

    pub fn threshold(n: u8, of: Vec<SpendPolicy>) -> Self { SpendPolicy::Threshold(PolicyTypeThreshold { n, of }) }

    pub fn opaque(p: &SpendPolicy) -> Self { SpendPolicy::Opaque(p.address()) }

    pub fn anyone_can_spend() -> Self { SpendPolicy::threshold(0, vec![]) }
}

#[derive(Debug, Clone)]
pub struct PolicyTypeThreshold {
    pub n: u8,
    pub of: Vec<SpendPolicy>,
}

// Compatibility with Sia's "UnlockConditions"
#[derive(Debug, Clone)]
pub struct PolicyTypeUnlockConditions(UnlockCondition);

#[derive(Debug, Clone)]
pub struct UnlockCondition {
    pubkeys: Vec<PublicKey>,
    timelock: u64,
    sigs_required: u64,
}

impl UnlockCondition {
    pub fn new(pubkeys: Vec<PublicKey>, timelock: u64, sigs_required: u64) -> Self {
        // TODO check go implementation to see if there should be limitations or checks imposed here
        UnlockCondition {
            pubkeys,
            timelock,
            sigs_required,
        }
    }

    pub fn unlock_hash(&self) -> H256 {
        // almost all UnlockConditions are standard, so optimize for that case
        if self.timelock == 0 && self.pubkeys.len() == 1 && self.sigs_required == 1 {
            return standard_unlock_hash(&self.pubkeys[0]);
        }

        let mut accumulator = Accumulator::default();

        accumulator.add_leaf(timelock_leaf(self.timelock));

        for pubkey in &self.pubkeys {
            accumulator.add_leaf(public_key_leaf(pubkey));
        }

        accumulator.add_leaf(sigs_required_leaf(self.sigs_required));
        accumulator.root()
    }

    pub fn address(&self) -> Address { Address(self.unlock_hash()) }
}

#[test]
fn test_unlock_condition_unlock_hash_standard() {
    let pubkey = PublicKey::from_bytes(
        &hex::decode("0102030000000000000000000000000000000000000000000000000000000000").unwrap(),
    )
    .unwrap();
    let unlock_condition = UnlockCondition::new(vec![pubkey], 0, 1);

    let hash = unlock_condition.unlock_hash();
    let expected = H256::from("72b0762b382d4c251af5ae25b6777d908726d75962e5224f98d7f619bb39515d");
    assert_eq!(hash, expected);

    let hash = standard_unlock_hash(&pubkey);
    assert_eq!(hash, expected);
}

#[test]
fn test_unlock_condition_unlock_hash_2of2_multisig() {
    let pubkey = PublicKey::from_bytes(
        &hex::decode("0102030000000000000000000000000000000000000000000000000000000000").unwrap(),
    )
    .unwrap();
    let pubkey2 = PublicKey::from_bytes(
        &hex::decode("0101010000000000000000000000000000000000000000000000000000000000").unwrap(),
    )
    .unwrap();
    let unlock_condition = UnlockCondition::new(vec![pubkey, pubkey2], 0, 2);

    let hash = unlock_condition.unlock_hash();
    let expected = H256::from("1e94357817d236167e54970a8c08bbd41b37bfceeeb52f6c1ce6dd01d50ea1e7");
    assert_eq!(hash, expected);
}

#[test]
fn test_unlock_condition_unlock_hash_1of2_multisig() {
    let pubkey = PublicKey::from_bytes(
        &hex::decode("0102030000000000000000000000000000000000000000000000000000000000").unwrap(),
    )
    .unwrap();
    let pubkey2 = PublicKey::from_bytes(
        &hex::decode("0101010000000000000000000000000000000000000000000000000000000000").unwrap(),
    )
    .unwrap();
    let unlock_condition = UnlockCondition::new(vec![pubkey, pubkey2], 0, 1);

    let hash = unlock_condition.unlock_hash();
    let expected = H256::from("d7f84e3423da09d111a17f64290c8d05e1cbe4cab2b6bed49e3a4d2f659f0585");
    assert_eq!(hash, expected);
}

#[test]
fn test_spend_policy_encode_above() {
    let policy = SpendPolicy::above(1);

    let hash = policy.encode().hash();
    let expected = H256::from("bebf6cbdfb440a92e3e5d832ac30fe5d226ff6b352ed3a9398b7d35f086a8ab6");
    assert_eq!(hash, expected);

    let address = policy.address();
    let expected =
        Address::from_str("addr:188b997bb99dee13e95f92c3ea150bd76b3ec72e5ba57b0d57439a1a6e2865e9b25ea5d1825e").unwrap();
    assert_eq!(address, expected);
}

#[test]
fn test_spend_policy_encode_after() {
    let policy = SpendPolicy::after(1);

    let hash = policy.encode().hash();
    let expected = H256::from("07b0f28eafd87a082ad11dc4724e1c491821260821a30bec68254444f97d9311");
    assert_eq!(hash, expected);

    let address = policy.address();
    let expected =
        Address::from_str("addr:60c74e0ce5cede0f13f83b0132cb195c995bc7688c9fac34bbf2b14e14394b8bbe2991bc017f").unwrap();
    assert_eq!(address, expected);
}

#[test]
fn test_spend_policy_encode_pubkey() {
    let pubkey = PublicKey::from_bytes(
        &hex::decode("0102030000000000000000000000000000000000000000000000000000000000").unwrap(),
    )
    .unwrap();
    let policy = SpendPolicy::PublicKey(pubkey);

    let hash = policy.encode().hash();
    let expected = H256::from("4355c8f80f6e5a98b70c9c2f9a22f17747989b4744783c90439b2b034f698bfe");
    assert_eq!(hash, expected);

    let address = policy.address();
    let expected =
        Address::from_str("addr:55a7793237722c6df8222fd512063cb74228085ef1805c5184713648c159b919ac792fbad0e1").unwrap();
    assert_eq!(address, expected);
}

#[test]
fn test_spend_policy_encode_hash() {
    let hash = H256::from("0102030000000000000000000000000000000000000000000000000000000000");
    let policy = SpendPolicy::Hash(hash);

    let encoded = policy.encode();
    let hash = encoded.hash();
    let expected = H256::from("9938967aefa6cbecc1f1620d2df5170d6811d4b2f47a879b621c1099a3b0628a");
    assert_eq!(hash, expected);

    let address = policy.address();
    let expected =
        Address::from_str("addr:a4d5a06d8d3c2e45aa26627858ce8e881505ae3c9d122a1d282c7824163751936cffb347e435").unwrap();
    assert_eq!(address, expected);
}

#[test]
fn test_spend_policy_encode_threshold() {
    let policy = SpendPolicy::Threshold(PolicyTypeThreshold {
        n: 1,
        of: vec![SpendPolicy::above(1), SpendPolicy::after(1)],
    });

    let encoded = policy.encode();
    let hash = encoded.hash();
    let expected = H256::from("7d792df6cd0b5e0f795287b3bf4087bbcc4c1bd0c52880a552cdda3e5e33d802");
    assert_eq!(hash, expected);

    let address = policy.address();
    let expected =
        Address::from_str("addr:4179b53aba165e46e4c85b3c8766bb758fb6f0bfa5721550b81981a3ec38efc460557dc1ded4").unwrap();
    assert_eq!(address, expected);
}

#[test]
fn test_spend_policy_encode_unlock_condition() {
    let pubkey = PublicKey::from_bytes(
        &hex::decode("0102030000000000000000000000000000000000000000000000000000000000").unwrap(),
    )
    .unwrap();
    let unlock_condition = UnlockCondition::new(vec![pubkey], 0, 1);

    let sub_policy = SpendPolicy::UnlockConditions(PolicyTypeUnlockConditions(unlock_condition));
    let base_address = sub_policy.address();
    let expected =
        Address::from_str("addr:72b0762b382d4c251af5ae25b6777d908726d75962e5224f98d7f619bb39515dd64b9a56043a").unwrap();
    assert_eq!(base_address, expected);

    let policy = SpendPolicy::Threshold(PolicyTypeThreshold {
        n: 1,
        of: vec![sub_policy],
    });
    let address = policy.address();
    let expected =
        Address::from_str("addr:1498a58c843ce66740e52421632d67a0f6991ea96db1fc97c29e46f89ae56e3534078876331d").unwrap();
    assert_eq!(address, expected);
}

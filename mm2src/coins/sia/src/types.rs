use crate::blake2b_internal::standard_unlock_hash;
use crate::encoding::{Encodable, Encoder, PrefixedH256};
use crate::transaction::{FileContractElementV1, SiacoinElement, SiafundElement, StateElement, V1Transaction,
                         V2FileContractResolution, V2Transaction};
use blake2b_simd::Params;
use chrono::{DateTime, Utc};
use ed25519_dalek::PublicKey;
use hex::FromHexError;
use rpc::v1::types::H256;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use serde_with::{serde_as, FromInto};
use std::convert::From;
use std::convert::TryInto;
use std::fmt;
use std::str::FromStr;

// TODO this could probably include the checksum within the data type
// generating the checksum on the fly is how Sia Go does this however
#[derive(Debug, Clone, PartialEq)]
pub struct Address(pub H256);

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_str = format!("{}", self);
        serializer.serialize_str(&hex_str)
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AddressVisitor;

        impl<'de> serde::de::Visitor<'de> for AddressVisitor {
            type Value = Address;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string prefixed with 'addr:' and followed by a 76-character hex string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Address::from_str(value).map_err(|_| E::invalid_value(serde::de::Unexpected::Str(value), &self))
            }
        }

        deserializer.deserialize_str(AddressVisitor)
    }
}

impl Address {
    pub fn str_without_prefix(&self) -> String {
        let bytes = self.0 .0.as_ref();
        let checksum = blake2b_checksum(bytes);
        format!("{}{}", hex::encode(bytes), hex::encode(checksum))
    }
}

impl Encodable for Address {
    fn encode(&self, encoder: &mut Encoder) { self.0.encode(encoder) }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "addr:{}", self.str_without_prefix()) }
}

impl fmt::Display for ParseAddressError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "Failed to parse Address: {:?}", self) }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum ParseAddressError {
    #[serde(rename = "Address must begin with addr: prefix")]
    MissingPrefix,
    InvalidHexEncoding(String),
    InvalidChecksum,
    InvalidLength,
}

impl From<FromHexError> for ParseAddressError {
    fn from(e: FromHexError) -> Self { ParseAddressError::InvalidHexEncoding(e.to_string()) }
}

impl FromStr for Address {
    type Err = ParseAddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("addr:") {
            return Err(ParseAddressError::MissingPrefix);
        }

        let without_prefix = &s[5..];
        if without_prefix.len() != (32 + 6) * 2 {
            return Err(ParseAddressError::InvalidLength);
        }

        let (address_hex, checksum_hex) = without_prefix.split_at(32 * 2);

        let address_bytes: [u8; 32] = hex::decode(address_hex)
            .map_err(ParseAddressError::from)?
            .try_into()
            .expect("length is 32 bytes");

        let checksum = hex::decode(checksum_hex).map_err(ParseAddressError::from)?;
        let checksum_bytes: [u8; 6] = checksum.try_into().expect("length is 6 bytes");

        if checksum_bytes != blake2b_checksum(&address_bytes) {
            return Err(ParseAddressError::InvalidChecksum);
        }

        Ok(Address(H256::from(address_bytes)))
    }
}

// Sia uses the first 6 bytes of blake2b(preimage) appended
// to address as checksum
fn blake2b_checksum(preimage: &[u8]) -> [u8; 6] {
    let hash = Params::new().hash_length(32).to_state().update(preimage).finalize();
    hash.as_array()[0..6].try_into().expect("array is 64 bytes long")
}

pub fn v1_standard_address_from_pubkey(pubkey: &PublicKey) -> Address {
    let hash = standard_unlock_hash(pubkey);
    Address(hash)
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockID(pub H256);

impl From<BlockID> for H256 {
    fn from(sia_hash: BlockID) -> Self { sia_hash.0 }
}

impl From<H256> for BlockID {
    fn from(h256: H256) -> Self { BlockID(h256) }
}

impl<'de> Deserialize<'de> for BlockID {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BlockIDVisitor;

        impl<'de> serde::de::Visitor<'de> for BlockIDVisitor {
            type Value = BlockID;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string prefixed with 'bid:' and followed by a 64-character hex string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if let Some(hex_str) = value.strip_prefix("bid:") {
                    H256::from_str(hex_str)
                        .map(BlockID)
                        .map_err(|_| E::invalid_value(serde::de::Unexpected::Str(value), &self))
                } else {
                    Err(E::invalid_value(serde::de::Unexpected::Str(value), &self))
                }
            }
        }

        deserializer.deserialize_str(BlockIDVisitor)
    }
}

impl Serialize for BlockID {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{}", self))
    }
}

impl fmt::Display for BlockID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "bid:{}", self.0) }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ChainIndex {
    pub height: u64,
    pub id: BlockID,
}

// TODO unit test
impl Encodable for ChainIndex {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.write_u64(self.height);
        let block_id: H256 = self.id.clone().into();
        block_id.encode(encoder);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventV1Transaction {
    pub transaction: V1Transaction,
    pub spent_siacoin_elements: Vec<SiacoinElement>,
    pub spent_siafund_elements: Vec<SiafundElement>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EventV1ContractResolution {
    pub parent: FileContractElementV1,
    pub siacoin_element: SiacoinElement,
    pub missed: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPayout {
    pub siacoin_element: SiacoinElement,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
    Miner,
    Foundation,
    SiafundClaim,
    V1Transaction,
    V2Transaction,
    V1ContractResolution,
    V2ContractResolution,
}

#[serde_as]
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    #[serde_as(as = "FromInto<PrefixedH256>")]
    pub id: H256,
    pub index: ChainIndex,
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "maturityHeight")]
    pub maturity_height: u64,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub data: EventDataWrapper,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevant: Option<Vec<Address>>,
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Debug)]
        struct EventHelper {
            id: PrefixedH256,
            index: ChainIndex,
            timestamp: DateTime<Utc>,
            #[serde(rename = "maturityHeight")]
            maturity_height: u64,
            #[serde(rename = "type")]
            event_type: EventType,
            data: Value,
            relevant: Option<Vec<Address>>,
        }

        let helper = EventHelper::deserialize(deserializer)?;
        let event_data = match helper.event_type {
            EventType::Miner => serde_json::from_value::<EventPayout>(helper.data)
                .map(EventDataWrapper::MinerPayout)
                .map_err(serde::de::Error::custom),
            EventType::Foundation => serde_json::from_value::<EventPayout>(helper.data)
                .map(EventDataWrapper::FoundationPayout)
                .map_err(serde::de::Error::custom),
            EventType::SiafundClaim => serde_json::from_value::<EventPayout>(helper.data)
                .map(EventDataWrapper::ClaimPayout)
                .map_err(serde::de::Error::custom),
            EventType::V1Transaction => serde_json::from_value::<EventV1Transaction>(helper.data)
                .map(EventDataWrapper::V1Transaction)
                .map_err(serde::de::Error::custom),
            EventType::V2Transaction => serde_json::from_value::<V2Transaction>(helper.data)
                .map(EventDataWrapper::V2Transaction)
                .map_err(serde::de::Error::custom),
            EventType::V1ContractResolution => unimplemented!(),
            EventType::V2ContractResolution => serde_json::from_value::<EventV2ContractResolution>(helper.data)
                .map(|data| EventDataWrapper::V2FileContractResolution(Box::new(data)))
                .map_err(serde::de::Error::custom),
        }?;

        Ok(Event {
            id: helper.id.into(),
            index: helper.index,
            timestamp: helper.timestamp,
            maturity_height: helper.maturity_height,
            event_type: helper.event_type,
            data: event_data,
            relevant: helper.relevant,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EventDataWrapper {
    MinerPayout(EventPayout),
    FoundationPayout(EventPayout),
    ClaimPayout(EventPayout),
    V2Transaction(V2Transaction),
    V2FileContractResolution(Box<EventV2ContractResolution>),
    V1Transaction(EventV1Transaction),
    EventV1ContractResolution(EventV1ContractResolution),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventV2ContractResolution {
    pub resolution: V2FileContractResolution,
    pub siacoin_element: SiacoinElement,
    pub missed: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainIndexElement {
    #[serde(flatten)]
    state_element: StateElement,
    chain_index: ChainIndex,
}

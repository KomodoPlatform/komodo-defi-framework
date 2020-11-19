use super::{MatchBy as SuperMatchBy, TakerAction};
use crate::mm2::lp_ordermatch::OrderConfirmationsSettings;
use common::mm_number::MmNumber;
use compact_uuid::CompactUuid;
use num_rational::BigRational;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize)]
#[allow(clippy::large_enum_variant)]
pub enum OrdermatchMessage {
    MakerOrderCreated(MakerOrderCreated),
    MakerOrderUpdated(MakerOrderUpdated),
    PubkeyKeepAlive(PubkeyKeepAlive),
    MakerOrderCancelled(MakerOrderCancelled),
    TakerRequest(TakerRequest),
    MakerReserved(MakerReserved),
    TakerConnect(TakerConnect),
    MakerConnected(MakerConnected),
}

impl From<PubkeyKeepAlive> for OrdermatchMessage {
    fn from(keep_alive: PubkeyKeepAlive) -> Self { OrdermatchMessage::PubkeyKeepAlive(keep_alive) }
}

impl From<MakerOrderUpdated> for OrdermatchMessage {
    fn from(message: MakerOrderUpdated) -> Self { OrdermatchMessage::MakerOrderUpdated(message) }
}

impl From<MakerOrderUpdated> for OrdermatchMessage {
    fn from(message: MakerOrderUpdated) -> Self { OrdermatchMessage::MakerOrderUpdated(message) }
}

/// MsgPack compact representation does not work with tagged enums (encoding works, but decoding fails)
/// This is untagged representation also using compact Uuid representation
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum MatchBy {
    Any,
    Orders(HashSet<CompactUuid>),
    Pubkeys(HashSet<[u8; 32]>),
}

impl From<SuperMatchBy> for MatchBy {
    fn from(match_by: SuperMatchBy) -> MatchBy {
        match match_by {
            SuperMatchBy::Any => MatchBy::Any,
            SuperMatchBy::Orders(uuids) => MatchBy::Orders(uuids.into_iter().map(|uuid| uuid.into()).collect()),
            SuperMatchBy::Pubkeys(pubkeys) => MatchBy::Pubkeys(pubkeys.into_iter().map(|pubkey| pubkey.0).collect()),
        }
    }
}

impl Into<SuperMatchBy> for MatchBy {
    fn into(self) -> SuperMatchBy {
        match self {
            MatchBy::Any => SuperMatchBy::Any,
            MatchBy::Orders(uuids) => SuperMatchBy::Orders(uuids.into_iter().map(|uuid| uuid.into()).collect()),
            MatchBy::Pubkeys(pubkeys) => {
                SuperMatchBy::Pubkeys(pubkeys.into_iter().map(|pubkey| pubkey.into()).collect())
            },
        }
    }
}

mod compact_uuid {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::str::FromStr;
    use uuid::Uuid;

    /// Default MsgPack encoded UUID length is 38 bytes (seems like it is encoded as string)
    /// This wrapper is encoded to raw 16 bytes representation
    /// Derives all traits of wrapped value
    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    pub struct CompactUuid(Uuid);

    impl From<Uuid> for CompactUuid {
        fn from(uuid: Uuid) -> Self { CompactUuid(uuid) }
    }

    impl Into<Uuid> for CompactUuid {
        fn into(self) -> Uuid { self.0 }
    }

    impl FromStr for CompactUuid {
        type Err = uuid::parser::ParseError;

        fn from_str(str: &str) -> Result<Self, Self::Err> {
            let uuid = Uuid::parse_str(str)?;
            Ok(uuid.into())
        }
    }

    impl Serialize for CompactUuid {
        fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            s.serialize_bytes(self.0.as_bytes())
        }
    }

    impl<'de> Deserialize<'de> for CompactUuid {
        fn deserialize<D>(d: D) -> Result<CompactUuid, D::Error>
        where
            D: Deserializer<'de>,
        {
            let bytes: &[u8] = Deserialize::deserialize(d)?;
            let uuid = Uuid::from_slice(bytes)
                .map_err(|e| serde::de::Error::custom(format!("Uuid::from_slice error {}", e)))?;
            Ok(uuid.into())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MakerOrderCreated {
    pub uuid: CompactUuid,
    pub base: String,
    pub rel: String,
    pub price: BigRational,
    pub max_volume: BigRational,
    pub min_volume: BigRational,
    pub created_at: u64,
    pub conf_settings: OrderConfirmationsSettings,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PubkeyKeepAlive {
    pub orders_trie_root: [u8; 8],
    pub timestamp: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MakerOrderCancelled {
    pub uuid: CompactUuid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MakerOrderUpdated {
    uuid: CompactUuid,
    new_price: Option<BigRational>,
    new_max_volume: Option<BigRational>,
    new_min_volume: Option<BigRational>,
}

impl MakerOrderUpdated {
    pub fn new(uuid: Uuid) -> Self {
        MakerOrderUpdated {
            uuid: uuid.into(),
            new_price: None,
            new_max_volume: None,
            new_min_volume: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_new_price(mut self, new_price: BigRational) -> Self {
        self.new_price = Some(new_price);
        self
    }

    pub fn with_new_max_volume(mut self, new_max_volume: BigRational) -> Self {
        self.new_max_volume = Some(new_max_volume);
        self
    }

    #[allow(dead_code)]
    pub fn with_new_min_volume(mut self, new_min_volume: BigRational) -> Self {
        self.new_min_volume = Some(new_min_volume);
        self
    }

    pub fn new_price(&self) -> Option<MmNumber> { self.new_price.as_ref().map(|num| num.clone().into()) }

    pub fn new_max_volume(&self) -> Option<MmNumber> { self.new_max_volume.as_ref().map(|num| num.clone().into()) }

    pub fn new_min_volume(&self) -> Option<MmNumber> { self.new_min_volume.as_ref().map(|num| num.clone().into()) }

    pub fn uuid(&self) -> Uuid { self.uuid.into() }
}

impl MakerOrderUpdated {
    pub fn new(uuid: Uuid) -> Self {
        MakerOrderUpdated {
            uuid: uuid.into(),
            new_price: None,
            new_max_volume: None,
            new_min_volume: None,
        }
    }

    pub fn with_new_price(mut self, new_price: MmNumber) -> Self {
        self.new_price = Some(new_price);
        self
    }

    pub fn with_new_max_volume(mut self, new_max_volume: MmNumber) -> Self {
        self.new_max_volume = Some(new_max_volume);
        self
    }

    pub fn with_new_min_volume(mut self, new_min_volume: MmNumber) -> Self {
        self.new_min_volume = Some(new_min_volume);
        self
    }

    pub fn new_price(&self) -> &Option<MmNumber> { &self.new_price }

    pub fn new_max_volume(&self) -> &Option<MmNumber> { &self.new_max_volume }

    pub fn new_min_volume(&self) -> &Option<MmNumber> { &self.new_min_volume }

    pub fn uuid(&self) -> Uuid { self.uuid.into() }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TakerRequest {
    pub base: String,
    pub rel: String,
    pub base_amount: BigRational,
    pub rel_amount: BigRational,
    pub action: TakerAction,
    pub uuid: CompactUuid,
    pub match_by: MatchBy,
    pub conf_settings: OrderConfirmationsSettings,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MakerReserved {
    pub base: String,
    pub rel: String,
    pub base_amount: BigRational,
    pub rel_amount: BigRational,
    pub taker_order_uuid: CompactUuid,
    pub maker_order_uuid: CompactUuid,
    pub conf_settings: OrderConfirmationsSettings,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TakerConnect {
    pub taker_order_uuid: CompactUuid,
    pub maker_order_uuid: CompactUuid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MakerConnected {
    pub taker_order_uuid: CompactUuid,
    pub maker_order_uuid: CompactUuid,
}

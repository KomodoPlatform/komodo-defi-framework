use async_std::prelude::FutureExt;
use chrono::Utc;
use common::executor::SpawnFuture;
use common::expirable_map::ExpirableEntry;
use common::{log, HttpStatusCode, StatusCode};
use derive_more::Display;
use futures::channel::oneshot::{self, Receiver, Sender};
use instant::{Duration, Instant};
use lazy_static::lazy_static;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;
use mm2_libp2p::{decode_message, encode_message, pub_sub_topic, Libp2pPublic, TopicPrefix};
use mm2_net::p2p::P2PContext;
use ser_error_derive::SerializeErrorType;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use std::str::FromStr;
use std::sync::Mutex;

use crate::lp_network::broadcast_p2p_msg;

pub(crate) const PEER_HEALTHCHECK_PREFIX: TopicPrefix = "hcheck";

const fn healthcheck_message_exp_secs() -> u64 {
    #[cfg(test)]
    return 3;

    #[cfg(not(test))]
    10
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, target_arch = "wasm32"), derive(PartialEq))]
pub(crate) struct HealthcheckMessage {
    #[serde(deserialize_with = "deserialize_bytes")]
    signature: Vec<u8>,
    data: HealthcheckData,
}

/// Wrapper of `libp2p::PeerId` with trait additional implementations.
///
/// TODO: This should be used as a replacement of `libp2p::PeerId` in the entire project.
#[derive(Clone, Copy, Debug, Display, PartialEq)]
pub struct PeerAddress(mm2_libp2p::PeerId);

impl From<mm2_libp2p::PeerId> for PeerAddress {
    fn from(value: mm2_libp2p::PeerId) -> Self { Self(value) }
}

impl From<PeerAddress> for mm2_libp2p::PeerId {
    fn from(value: PeerAddress) -> Self { value.0 }
}

impl Serialize for PeerAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for PeerAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PeerAddressVisitor;

        impl<'de> serde::de::Visitor<'de> for PeerAddressVisitor {
            type Value = PeerAddress;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string representation of peer id.")
            }

            fn visit_str<E>(self, value: &str) -> Result<PeerAddress, E>
            where
                E: serde::de::Error,
            {
                if value.len() > 100 {
                    return Err(serde::de::Error::invalid_length(
                        value.len(),
                        &"peer id cannot exceed 100 characters.",
                    ));
                }

                Ok(mm2_libp2p::PeerId::from_str(value)
                    .map_err(serde::de::Error::custom)?
                    .into())
            }

            fn visit_string<E>(self, value: String) -> Result<PeerAddress, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_str(PeerAddressVisitor)
    }
}

impl HealthcheckMessage {
    pub(crate) fn generate_message(ctx: &MmArc, is_a_reply: bool) -> Result<Self, String> {
        let p2p_ctx = P2PContext::fetch_from_mm_arc(ctx);
        let sender_peer = p2p_ctx.peer_id().into();
        let keypair = p2p_ctx.keypair();
        let sender_public_key = keypair.public().encode_protobuf();

        let data = HealthcheckData {
            sender_peer,
            sender_public_key,
            expires_at: Utc::now().timestamp() + healthcheck_message_exp_secs() as i64,
            is_a_reply,
        };

        let signature = try_s!(keypair.sign(&try_s!(data.encode())));

        Ok(Self { signature, data })
    }

    fn generate_or_use_cached_message(ctx: &MmArc) -> Result<Self, String> {
        const MIN_DURATION_FOR_REUSABLE_MSG: Duration = Duration::from_secs(5);

        lazy_static! {
            static ref RECENTLY_GENERATED_MESSAGE: Mutex<ExpirableEntry<HealthcheckMessage>> =
                Mutex::new(ExpirableEntry::new(
                    // Using dummy values in order to initialize `HealthcheckMessage` context.
                    HealthcheckMessage {
                        signature: vec![],
                        data: HealthcheckData {
                            sender_peer: create_test_peer_address(),
                            sender_public_key: vec![],
                            expires_at: 0,
                            is_a_reply: false,
                        },
                    },
                    Duration::from_secs(0)
                ));
        }

        // If recently generated message has longer life than `MIN_DURATION_FOR_REUSABLE_MSG`, we can reuse it to
        // reduce the message generation overhead under high pressure.
        let mut mutexed_msg = RECENTLY_GENERATED_MESSAGE.lock().unwrap();

        if mutexed_msg.has_longer_life_than(MIN_DURATION_FOR_REUSABLE_MSG) {
            Ok(mutexed_msg.get_element().clone())
        } else {
            let new_msg = HealthcheckMessage::generate_message(ctx, true)?;

            mutexed_msg.update_value(new_msg.clone());
            mutexed_msg.update_expiration(Instant::now() + Duration::from_secs(healthcheck_message_exp_secs()));

            Ok(new_msg)
        }
    }

    pub(crate) fn is_received_message_valid(&self) -> bool {
        let now = Utc::now().timestamp();
        let remaining_expiration_seconds = u64::try_from(self.data.expires_at - now).unwrap_or(0);

        if remaining_expiration_seconds == 0 {
            log::debug!(
                "Healthcheck message is expired. Current time in UTC: {now}, healthcheck `expires_at` in UTC: {}",
                self.data.expires_at
            );
            return false;
        } else if remaining_expiration_seconds > healthcheck_message_exp_secs() {
            log::debug!(
                "Healthcheck message have too high expiration time.\nMax allowed expiration seconds: {}\nReceived message expiration seconds: {}",
                self.data.expires_at,
                remaining_expiration_seconds,
            );
            return false;
        }

        let Ok(public_key) = Libp2pPublic::try_decode_protobuf(&self.data.sender_public_key) else {
            log::debug!("Couldn't decode public key from the healthcheck message.");

            return false
        };

        if self.data.sender_peer != public_key.to_peer_id().into() {
            log::debug!("`sender_peer` and `sender_public_key` doesn't belong each other.");

            return false;
        }

        let Ok(encoded_message) = self.data.encode() else {
            log::debug!("Couldn't encode healthcheck data.");
            return false
        };

        let res = public_key.verify(&encoded_message, &self.signature);

        if !res {
            log::debug!("Healthcheck isn't signed correctly.");
        }

        res
    }

    #[inline]
    pub(crate) fn encode(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> { encode_message(self) }

    #[inline]
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, rmp_serde::decode::Error> { decode_message(bytes) }

    #[inline]
    pub(crate) fn should_reply(&self) -> bool { !self.data.is_a_reply }

    #[inline]
    pub(crate) fn sender_peer(&self) -> PeerAddress { self.data.sender_peer }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, target_arch = "wasm32"), derive(PartialEq))]
struct HealthcheckData {
    sender_peer: PeerAddress,
    #[serde(deserialize_with = "deserialize_bytes")]
    sender_public_key: Vec<u8>,
    expires_at: i64,
    is_a_reply: bool,
}

impl HealthcheckData {
    #[inline]
    fn encode(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> { encode_message(self) }
}

#[inline]
pub fn peer_healthcheck_topic(peer_address: &PeerAddress) -> String {
    pub_sub_topic(PEER_HEALTHCHECK_PREFIX, &peer_address.to_string())
}

#[derive(Deserialize)]
pub struct RequestPayload {
    peer_address: PeerAddress,
}

fn deserialize_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ByteVisitor;

    impl<'de> serde::de::Visitor<'de> for ByteVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a non-empty byte array up to 512 bytes")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Vec<u8>, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut buffer = vec![];
            while let Some(byte) = seq.next_element()? {
                if buffer.len() >= 512 {
                    return Err(serde::de::Error::invalid_length(
                        buffer.len(),
                        &"longest possible length allowed for this field is 512 bytes (with RSA algorithm).",
                    ));
                }

                buffer.push(byte);
            }

            if buffer.is_empty() {
                return Err(serde::de::Error::custom("Can't be empty."));
            }

            Ok(buffer)
        }
    }

    deserializer.deserialize_seq(ByteVisitor)
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum HealthcheckRpcError {
    MessageGenerationFailed { reason: String },
    MessageEncodingFailed { reason: String },
    Internal { reason: String },
}

impl HttpStatusCode for HealthcheckRpcError {
    fn status_code(&self) -> common::StatusCode {
        match self {
            HealthcheckRpcError::MessageGenerationFailed { .. }
            | HealthcheckRpcError::Internal { .. }
            | HealthcheckRpcError::MessageEncodingFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn peer_connection_healthcheck_rpc(
    ctx: MmArc,
    req: RequestPayload,
) -> Result<bool, MmError<HealthcheckRpcError>> {
    // When things go awry, we want records to clear themselves to keep the memory clean of unused data.
    // This is unrelated to the timeout logic.
    let address_record_exp = Duration::from_secs(healthcheck_message_exp_secs());

    let target_peer_address = req.peer_address;

    let p2p_ctx = P2PContext::fetch_from_mm_arc(&ctx);
    if target_peer_address == p2p_ctx.peer_id().into() {
        // That's us, so return true.
        return Ok(true);
    }

    let message = HealthcheckMessage::generate_message(&ctx, false)
        .map_err(|reason| HealthcheckRpcError::MessageGenerationFailed { reason })?;

    let encoded_message = message
        .encode()
        .map_err(|e| HealthcheckRpcError::MessageEncodingFailed { reason: e.to_string() })?;

    let (tx, rx): (Sender<()>, Receiver<()>) = oneshot::channel();

    {
        let mut book = ctx.healthcheck_response_handler.lock().await;
        book.clear_expired_entries();
        book.insert(target_peer_address.to_string(), tx, address_record_exp);
    }

    broadcast_p2p_msg(
        &ctx,
        peer_healthcheck_topic(&target_peer_address),
        encoded_message,
        None,
    );

    let timeout_duration = Duration::from_secs(healthcheck_message_exp_secs());
    Ok(rx.timeout(timeout_duration).await == Ok(Ok(())))
}

pub(crate) async fn process_p2p_healthcheck_message(ctx: &MmArc, message: mm2_libp2p::GossipsubMessage) {
    macro_rules! try_or_return {
        ($exp:expr, $msg: expr) => {
            match $exp {
                Ok(t) => t,
                Err(e) => {
                    log::error!("{}, error: {e:?}", $msg);
                    return;
                },
            }
        };
    }

    let data = try_or_return!(
        HealthcheckMessage::decode(&message.data),
        "Couldn't decode healthcheck message"
    );

    let sender_peer = data.sender_peer();

    let ctx = ctx.clone();

    // Pass the remaining work to another thread to free up this one as soon as possible,
    // so KDF can handle a high amount of healthcheck messages more efficiently.
    ctx.spawner().spawn(async move {
        if !data.is_received_message_valid() {
            log::error!("Received an invalid healthcheck message.");
            log::debug!("Message context: {:?}", data);
            return;
        };

        if data.should_reply() {
            // Reply the message so they know we are healthy.

            let msg = try_or_return!(
                HealthcheckMessage::generate_or_use_cached_message(&ctx),
                "Couldn't generate the healthcheck message, this is very unusual!"
            );

            let encoded_msg = try_or_return!(
                msg.encode(),
                "Couldn't encode healthcheck message, this is very unusual!"
            );

            let topic = peer_healthcheck_topic(&sender_peer);
            broadcast_p2p_msg(&ctx, topic, encoded_msg, None);
        } else {
            // The requested peer is healthy; signal the response channel.
            let mut response_handler = ctx.healthcheck_response_handler.lock().await;
            if let Some(tx) = response_handler.remove(&sender_peer.to_string()) {
                if tx.send(()).is_err() {
                    log::error!("Result channel isn't present for peer '{sender_peer}'.");
                };
            } else {
                log::info!("Peer '{sender_peer}' isn't recorded in the healthcheck response handler.");
            };
        }
    });
}

fn create_test_peer_address() -> PeerAddress {
    let keypair = mm2_libp2p::Keypair::generate_ed25519();
    mm2_libp2p::PeerId::from(keypair.public()).into()
}

#[cfg(any(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use common::cross_test;
    use crypto::CryptoCtx;
    use mm2_libp2p::behaviours::atomicdex::generate_ed25519_keypair;
    use mm2_test_helpers::for_tests::mm_ctx_with_iguana;

    common::cfg_wasm32! {
        use wasm_bindgen_test::*;
        wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
    }

    fn ctx() -> MmArc {
        let ctx = mm_ctx_with_iguana(Some("dummy-value"));
        let p2p_key = {
            let crypto_ctx = CryptoCtx::from_ctx(&ctx).unwrap();
            let key = bitcrypto::sha256(crypto_ctx.mm2_internal_privkey_slice());
            key.take()
        };

        let (cmd_tx, _) = futures::channel::mpsc::channel(0);

        let p2p_context = P2PContext::new(cmd_tx, generate_ed25519_keypair(p2p_key));
        p2p_context.store_to_mm_arc(&ctx);

        ctx
    }

    cross_test!(test_peer_address, {
        #[derive(Deserialize, Serialize)]
        struct PeerAddressTest {
            peer_address: PeerAddress,
        }

        let address_str = "12D3KooWEtuv7kmgGCC7oAQ31hB7AR5KkhT3eEWB2bP2roo3M7rY";
        let json_content = format!("{{\"peer_address\": \"{address_str}\"}}");
        let address_struct: PeerAddressTest = serde_json::from_str(&json_content).unwrap();

        let actual_peer_id = mm2_libp2p::PeerId::from_str(address_str).unwrap();
        let deserialized_peer_id: mm2_libp2p::PeerId = address_struct.peer_address.into();

        assert_eq!(deserialized_peer_id, actual_peer_id);
    });

    cross_test!(test_valid_message, {
        let ctx = ctx();
        let message = HealthcheckMessage::generate_message(&ctx, false).unwrap();
        assert!(message.is_received_message_valid());
    });

    cross_test!(test_corrupted_messages, {
        let ctx = ctx();

        let mut message = HealthcheckMessage::generate_message(&ctx, false).unwrap();
        message.data.expires_at += 1;
        assert!(!message.is_received_message_valid());

        let mut message = HealthcheckMessage::generate_message(&ctx, false).unwrap();
        message.data.is_a_reply = !message.data.is_a_reply;
        assert!(!message.is_received_message_valid());

        let mut message = HealthcheckMessage::generate_message(&ctx, false).unwrap();
        message.data.sender_peer = create_test_peer_address();
        assert!(!message.is_received_message_valid());
    });

    cross_test!(test_expired_message, {
        let ctx = ctx();
        let message = HealthcheckMessage::generate_message(&ctx, false).unwrap();
        common::executor::Timer::sleep(3.).await;
        assert!(!message.is_received_message_valid());
    });

    cross_test!(test_encode_decode, {
        let ctx = ctx();
        let original = HealthcheckMessage::generate_message(&ctx, false).unwrap();

        let encoded = original.encode().unwrap();
        assert!(!encoded.is_empty());

        let decoded = HealthcheckMessage::decode(&encoded).unwrap();
        assert_eq!(original, decoded);
    });
}

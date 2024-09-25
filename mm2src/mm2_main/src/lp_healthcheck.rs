use async_std::prelude::FutureExt;
use chrono::Utc;
use common::executor::SpawnFuture;
use common::{log, HttpStatusCode, StatusCode};
use derive_more::Display;
use futures::channel::oneshot::{self, Receiver, Sender};
use instant::Duration;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;
use mm2_libp2p::{decode_message, encode_message, pub_sub_topic, Libp2pPublic, PeerId, TopicPrefix};
use mm2_net::p2p::P2PContext;
use ser_error_derive::SerializeErrorType;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use std::num::TryFromIntError;
use std::str::FromStr;

use crate::lp_network::broadcast_p2p_msg;

pub(crate) const PEER_HEALTHCHECK_PREFIX: TopicPrefix = "hcheck";

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, target_arch = "wasm32"), derive(PartialEq))]
pub(crate) struct HealthcheckMessage {
    #[serde(deserialize_with = "deserialize_bytes")]
    signature: Vec<u8>,
    data: HealthcheckData,
}

impl HealthcheckMessage {
    pub(crate) fn generate_message(
        ctx: &MmArc,
        target_peer: PeerId,
        is_a_reply: bool,
        expires_in_seconds: i64,
    ) -> Result<Self, String> {
        let p2p_ctx = P2PContext::fetch_from_mm_arc(ctx);
        let sender_peer = p2p_ctx.peer_id();
        let keypair = p2p_ctx.keypair();
        let sender_public_key = keypair.public().encode_protobuf();

        let data = HealthcheckData {
            sender_peer,
            sender_public_key,
            target_peer,
            expires_at: Utc::now().timestamp() + expires_in_seconds,
            is_a_reply,
        };

        let signature = try_s!(keypair.sign(&try_s!(data.encode())));

        Ok(Self { signature, data })
    }

    pub(crate) fn is_received_message_valid(&self, my_peer_id: PeerId) -> bool {
        let now = Utc::now().timestamp();
        if now > self.data.expires_at {
            log::debug!(
                "Healthcheck message is expired. Current time in UTC: {now}, healthcheck `expires_at` in UTC: {}",
                self.data.expires_at
            );
            return false;
        }

        if self.data.target_peer != my_peer_id {
            log::debug!(
                "`target_peer` doesn't match with our peer address. Our address: '{}', healthcheck `target_peer`: '{}'.",
                my_peer_id,
                self.data.target_peer
            );
            return false;
        }

        let Ok(public_key) = Libp2pPublic::try_decode_protobuf(&self.data.sender_public_key) else {
            log::debug!("Couldn't decode public key from the healthcheck message.");

            return false
        };

        if self.data.sender_peer != public_key.to_peer_id() {
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
    pub(crate) fn sender_peer(&self) -> PeerId { self.data.sender_peer }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, target_arch = "wasm32"), derive(PartialEq))]
struct HealthcheckData {
    #[serde(deserialize_with = "deserialize_peer_id", serialize_with = "serialize_peer_id")]
    sender_peer: PeerId,
    #[serde(deserialize_with = "deserialize_bytes")]
    sender_public_key: Vec<u8>,
    #[serde(deserialize_with = "deserialize_peer_id", serialize_with = "serialize_peer_id")]
    target_peer: PeerId,
    expires_at: i64,
    is_a_reply: bool,
}

impl HealthcheckData {
    #[inline]
    fn encode(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> { encode_message(self) }
}

#[inline]
pub fn peer_healthcheck_topic(peer_id: &PeerId) -> String {
    pub_sub_topic(PEER_HEALTHCHECK_PREFIX, &peer_id.to_string())
}

#[derive(Deserialize)]
pub struct RequestPayload {
    #[serde(deserialize_with = "deserialize_peer_id")]
    peer_id: PeerId,
}

fn deserialize_peer_id<'de, D>(deserializer: D) -> Result<PeerId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct PeerIdVisitor;

    impl<'de> serde::de::Visitor<'de> for PeerIdVisitor {
        type Value = PeerId;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string representation of peer id.")
        }

        fn visit_str<E>(self, value: &str) -> Result<PeerId, E>
        where
            E: serde::de::Error,
        {
            if value.len() > 100 {
                return Err(serde::de::Error::invalid_length(
                    value.len(),
                    &"peer id cannot exceed 100 characters.",
                ));
            }

            PeerId::from_str(value).map_err(serde::de::Error::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<PeerId, E>
        where
            E: serde::de::Error,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_str(PeerIdVisitor)
}

fn serialize_peer_id<S>(peer_id: &PeerId, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&peer_id.to_string())
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
    let address_record_exp = Duration::from_secs(ctx.health_checker.config.timeout_secs);

    let target_peer_id = req.peer_id;

    let p2p_ctx = P2PContext::fetch_from_mm_arc(&ctx);
    if target_peer_id == p2p_ctx.peer_id() {
        // That's us, so return true.
        return Ok(true);
    }

    let message = HealthcheckMessage::generate_message(
        &ctx,
        target_peer_id,
        false,
        ctx.health_checker
            .config
            .message_expiration
            .try_into()
            .map_err(|e: TryFromIntError| HealthcheckRpcError::Internal { reason: e.to_string() })?,
    )
    .map_err(|reason| HealthcheckRpcError::MessageGenerationFailed { reason })?;

    let encoded_message = message
        .encode()
        .map_err(|e| HealthcheckRpcError::MessageEncodingFailed { reason: e.to_string() })?;

    let (tx, rx): (Sender<()>, Receiver<()>) = oneshot::channel();

    {
        let mut book = ctx.health_checker.response_handler.lock().unwrap();
        book.clear_expired_entries();
        book.insert(target_peer_id.to_string(), tx, address_record_exp);
    }

    broadcast_p2p_msg(&ctx, peer_healthcheck_topic(&target_peer_id), encoded_message, None);

    let timeout_duration = Duration::from_secs(ctx.health_checker.config.timeout_secs);
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

    let sender_peer = data.sender_peer().to_owned();

    let mut ddos_shield = ctx.health_checker.ddos_shield.lock().unwrap();
    ddos_shield.clear_expired_entries();
    if ddos_shield
        .insert(
            sender_peer.to_string(),
            (),
            Duration::from_millis(ctx.health_checker.config.blocking_ms_for_per_address),
        )
        .is_some()
    {
        log::warn!("Peer '{sender_peer}' exceeded the healthcheck blocking time, skipping their message.");
        return;
    }
    drop(ddos_shield);

    let ctx_c = ctx.clone();

    // Pass the remaining work to another thread to free up this one as soon as possible,
    // so KDF can handle a high amount of healthcheck messages more efficiently.
    ctx.spawner().spawn(async move {
        let my_peer_id = P2PContext::fetch_from_mm_arc(&ctx_c).peer_id();
        if !data.is_received_message_valid(my_peer_id) {
            log::error!("Received an invalid healthcheck message.");
            log::debug!("Message context: {:?}", data);
            return;
        };

        if data.should_reply() {
            // Reply the message so they know we are healthy.

            let topic = peer_healthcheck_topic(&sender_peer);

            let msg = try_or_return!(
                HealthcheckMessage::generate_message(&ctx_c, sender_peer, true, 10),
                "Couldn't generate the healthcheck message, this is very unusual!"
            );

            let encoded_msg = try_or_return!(
                msg.encode(),
                "Couldn't encode healthcheck message, this is very unusual!"
            );

            broadcast_p2p_msg(&ctx_c, topic, encoded_msg, None);
        } else {
            // The requested peer is healthy; signal the response channel.
            let mut response_handler = ctx_c.health_checker.response_handler.lock().unwrap();
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

    fn create_test_peer_id() -> PeerId {
        let keypair = mm2_libp2p::Keypair::generate_ed25519();
        PeerId::from(keypair.public())
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

    cross_test!(test_valid_message, {
        let ctx = ctx();
        let target_peer = create_test_peer_id();
        let message = HealthcheckMessage::generate_message(&ctx, target_peer, false, 5).unwrap();
        assert!(message.is_received_message_valid(target_peer));
    });

    cross_test!(test_corrupted_messages, {
        let ctx = ctx();
        let target_peer = create_test_peer_id();

        let mut message = HealthcheckMessage::generate_message(&ctx, target_peer, false, 5).unwrap();
        message.data.expires_at += 1;
        assert!(!message.is_received_message_valid(target_peer));

        let mut message = HealthcheckMessage::generate_message(&ctx, target_peer, false, 5).unwrap();
        message.data.is_a_reply = !message.data.is_a_reply;
        assert!(!message.is_received_message_valid(target_peer));

        let mut message = HealthcheckMessage::generate_message(&ctx, target_peer, false, 5).unwrap();
        message.data.sender_peer = message.data.target_peer;
        assert!(!message.is_received_message_valid(target_peer));

        let mut message = HealthcheckMessage::generate_message(&ctx, target_peer, false, 5).unwrap();
        message.data.target_peer = message.data.sender_peer;
        assert!(!message.is_received_message_valid(target_peer));

        let message = HealthcheckMessage::generate_message(&ctx, target_peer, false, 5).unwrap();
        assert!(!message.is_received_message_valid(message.data.sender_peer));
    });

    cross_test!(test_expired_message, {
        let ctx = ctx();
        let target_peer = create_test_peer_id();
        let message = HealthcheckMessage::generate_message(&ctx, target_peer, false, -1).unwrap();
        assert!(!message.is_received_message_valid(target_peer));
    });

    cross_test!(test_encode_decode, {
        let ctx = ctx();
        let target_peer = create_test_peer_id();

        let original = HealthcheckMessage::generate_message(&ctx, target_peer, false, 10).unwrap();

        let encoded = original.encode().unwrap();
        assert!(!encoded.is_empty());

        let decoded = HealthcheckMessage::decode(&encoded).unwrap();
        assert_eq!(original, decoded);
    });
}

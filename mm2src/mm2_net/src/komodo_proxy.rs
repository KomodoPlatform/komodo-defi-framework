use common::get_utc_timestamp;
use http::Uri;
use mm2_libp2p::{Keypair, Libp2pPublic, SigningError};
use serde::{Deserialize, Serialize};

/// Represents a message and its corresponding signature.
#[derive(Serialize, Deserialize)]
pub struct ProxySign {
    /// Signature of the raw message.
    signature_bytes: Vec<u8>,
    /// The raw message that has been signed.
    raw_message: RawMessage,
}

/// Essential type that contains information required for generating signed messages (see `ProxySign`).
#[derive(Serialize, Deserialize)]
pub struct RawMessage {
    uri: String,
    body_size: usize,
    public_key_encoded: Vec<u8>,
    expires_at: i64,
}

impl RawMessage {
    fn new(uri: &Uri, body_size: usize, public_key_encoded: Vec<u8>, expires_in_seconds: i64) -> Self {
        RawMessage {
            uri: uri.to_string(),
            body_size,
            public_key_encoded,
            expires_at: get_utc_timestamp() + expires_in_seconds,
        }
    }

    /// Generates a byte vector representation of `self`.
    fn encode(&self) -> Vec<u8> {
        const PREFIX: &str = "Encoded Message for KDP\n";
        let mut bytes = PREFIX.as_bytes().to_owned();
        bytes.extend(self.public_key_encoded.clone());
        bytes.extend(self.uri.to_string().as_bytes().to_owned());
        bytes.extend(self.expires_at.to_ne_bytes().to_owned());
        bytes
    }

    /// Generates `ProxySign` using the provided keypair and coin ticker.
    pub fn sign(
        keypair: &Keypair,
        uri: &Uri,
        body_size: usize,
        expires_in_seconds: i64,
    ) -> Result<ProxySign, SigningError> {
        let public_key_encoded = keypair.public().encode_protobuf();
        let raw_message = RawMessage::new(uri, body_size, public_key_encoded, expires_in_seconds);
        let signature_bytes = keypair.sign(&raw_message.encode())?;

        Ok(ProxySign {
            raw_message,
            signature_bytes,
        })
    }
}

impl ProxySign {
    /// Validates if the message is still valid based on its expiration time and signature verification.
    pub fn is_valid_message(&self) -> bool {
        if get_utc_timestamp() > self.raw_message.expires_at {
            return false;
        }

        let Ok(public_key) = Libp2pPublic::try_decode_protobuf(&self.raw_message.public_key_encoded) else { return false };

        public_key.verify(&self.raw_message.encode(), &self.signature_bytes)
    }
}

#[cfg(test)]
pub mod tests {
    use mm2_libp2p::behaviours::atomicdex::generate_ed25519_keypair;

    use super::*;

    fn random_keypair() -> Keypair {
        let mut p2p_key = [0u8; 32];
        common::os_rng(&mut p2p_key).unwrap();
        generate_ed25519_keypair(p2p_key)
    }

    #[test]
    fn sign_and_verify() {
        let keypair = random_keypair();
        let signed_proxy_message = RawMessage::sign(&keypair, &Uri::from_static("http://example.com"), 0, 5).unwrap();
        assert!(signed_proxy_message.is_valid_message());
    }

    #[test]
    fn expired_signature() {
        let keypair = random_keypair();
        let signed_proxy_message = RawMessage::sign(&keypair, &Uri::from_static("http://example.com"), 0, -1).unwrap();
        assert!(!signed_proxy_message.is_valid_message());
    }

    #[test]
    fn dirty_raw_message() {
        let keypair = random_keypair();
        let mut signed_proxy_message =
            RawMessage::sign(&keypair, &Uri::from_static("http://example.com"), 0, 5).unwrap();
        signed_proxy_message.raw_message.uri = "http://demo.com".to_string();
        assert!(!signed_proxy_message.is_valid_message());

        let mut signed_proxy_message =
            RawMessage::sign(&keypair, &Uri::from_static("http://example.com"), 0, 5).unwrap();
        signed_proxy_message.raw_message.body_size += 1;
        assert!(!signed_proxy_message.is_valid_message());

        let mut signed_proxy_message =
            RawMessage::sign(&keypair, &Uri::from_static("http://example.com"), 0, 5).unwrap();
        signed_proxy_message.raw_message.expires_at += 1;
        assert!(!signed_proxy_message.is_valid_message());
    }

    #[test]
    fn verify_peer_id() {
        let p2p_key = [123u8; 32];
        let key_pair = generate_ed25519_keypair(p2p_key);
        assert_eq!(
            key_pair.public().to_peer_id().to_string(),
            "12D3KooWJPtxrHVDPoETNfPJY4WWVzX7Ti4WPemtXDgb5qmFrDiv"
        );
    }
}

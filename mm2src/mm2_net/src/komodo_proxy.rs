use common::get_utc_timestamp;
use mm2_libp2p::{Keypair, Libp2pPublic, SigningError};
use serde::{Deserialize, Serialize};

/// TODO
#[derive(Serialize, Deserialize)]
pub struct SignedProxyMessage {
    signature_bytes: Vec<u8>,
    raw_message: RawMessage,
}

/// TODO
#[derive(Serialize, Deserialize)]
pub struct RawMessage {
    public_key_encoded: Vec<u8>,
    coin_ticker: String,
    expires_at: i64,
}

impl RawMessage {
    fn new(coin_ticker: String, public_key_encoded: Vec<u8>) -> Self {
        const SIGNED_MESSAGE_LIFETIME_SEC: i64 = 20;

        RawMessage {
            public_key_encoded,
            coin_ticker,
            expires_at: get_utc_timestamp() + SIGNED_MESSAGE_LIFETIME_SEC,
        }
    }

    fn encode(&self) -> Vec<u8> {
        const PREFIX: &str = "Encoded Message for KDP\n";
        let mut bytes = PREFIX.as_bytes().to_owned();
        bytes.extend(self.public_key_encoded.clone());
        bytes.extend(self.coin_ticker.as_bytes().to_owned());
        bytes.extend(self.expires_at.to_ne_bytes().to_owned());
        bytes
    }

    /// TODO
    pub fn sign(keypair: &Keypair, coin_ticker: &str) -> Result<SignedProxyMessage, SigningError> {
        let public_key_encoded = keypair.public().encode_protobuf();
        let raw_message = RawMessage::new(coin_ticker.to_owned(), public_key_encoded);
        let signature_bytes = keypair.sign(&raw_message.encode())?;

        Ok(SignedProxyMessage {
            raw_message,
            signature_bytes,
        })
    }
}

impl SignedProxyMessage {
    /// TODO
    pub fn is_valid_message(&self) -> bool {
        let Ok(public_key) = Libp2pPublic::try_decode_protobuf(&self.raw_message.public_key_encoded) else { return false };
        public_key.verify(&self.raw_message.encode(), &self.signature_bytes)
    }
}

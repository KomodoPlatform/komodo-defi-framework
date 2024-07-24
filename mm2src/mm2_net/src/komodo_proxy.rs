use common::get_utc_timestamp;
use mm2_libp2p::{Keypair, SigningError};
use serde::Serialize;

/// TODO
#[derive(Serialize)]
pub struct SignedProxyMessage {
    signature_bytes: Vec<u8>,
    raw_message: RawMessage,
}

/// TODO
#[derive(Serialize)]
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

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.public_key_encoded.clone();
        bytes.extend(self.coin_ticker.as_bytes().to_owned());
        bytes.extend(self.expires_at.to_ne_bytes().to_owned());
        bytes
    }

    /// TODO
    pub fn sign(keypair: &Keypair, coin_ticker: &str) -> Result<SignedProxyMessage, SigningError> {
        let public_key_encoded = keypair.public().encode_protobuf();
        let raw_message = RawMessage::new(coin_ticker.to_owned(), public_key_encoded);
        let signature_bytes = keypair.sign(&raw_message.to_bytes())?;

        Ok(SignedProxyMessage {
            raw_message,
            signature_bytes,
        })
    }
}

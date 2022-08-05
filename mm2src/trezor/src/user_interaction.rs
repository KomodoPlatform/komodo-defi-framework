use derive_more::Display;

#[derive(Clone, Debug, Deserialize, Display, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrezorUserInteraction {
    ButtonRequest,
    PinMatrix3x3,
    PassphraseRequest,
    Other(String),
}

/// Use the numeric keypad to describe number positions.
/// The layout is:
/// 7 8 9
/// 4 5 6
/// 1 2 3
#[derive(Debug, Deserialize, Serialize)]
pub struct TrezorPinMatrix3x3Response {
    pub pin: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TrezorPassphraseResponse {
    /// A non-empty passphrase if the user is willing to use a hidden wallet,
    /// otherwise an empty passphrase.
    pub passphrase: String,
}

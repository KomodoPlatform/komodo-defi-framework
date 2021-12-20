use derive_more::Display;

#[derive(Clone, Copy, Debug, Display, Deserialize, Serialize)]
pub enum TrezorCoin {
    Bitcoin,
    Komodo,
    Qtum,
}

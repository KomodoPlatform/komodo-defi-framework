use common::serde_derive::Deserialize;
use mm2_number::{construct_detailed, Fraction, MmNumber};
use serde::Serialize;
use skip_serializing_none::skip_serializing_none;

#[derive(Serialize)]
#[serde(tag = "method", rename = "min_trading_vol")]
pub(crate) struct MinTradingVolRequest {
    pub(crate) coin: String,
}

#[derive(Serialize)]
#[serde(tag = "method", rename = "max_taker_vol")]
pub(crate) struct MaxTakerVolRequest {
    pub(crate) coin: String,
}

#[derive(Deserialize)]
pub(crate) struct MaxTakerVolResponse {
    pub(crate) coin: String,
    pub(crate) result: Fraction,
}

#[skip_serializing_none]
#[derive(Serialize)]
pub(crate) struct TradePreimageRequest {
    pub(crate) base: String,
    pub(crate) rel: String,
    pub(crate) swap_method: TradePreimageMethod,
    pub(crate) price: MmNumber,
    pub(crate) volume: Option<MmNumber>,
    pub(crate) max: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TradePreimageMethod {
    SetPrice,
    Buy,
    Sell,
}

#[derive(Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum TradePreimageResponse {
    MakerPreimage(MakerPreimage),
    TakerPreimage(TakerPreimage),
}

#[derive(Deserialize)]
pub(crate) struct MakerPreimage {
    pub(crate) base_coin_fee: TradeFeeResponse,
    pub(crate) rel_coin_fee: TradeFeeResponse,
    #[serde(flatten)]
    pub(crate) volume: Option<DetailedVolume>,
    pub(crate) total_fees: Vec<TotalTradeFeeResponse>,
}

#[derive(Deserialize)]
pub(crate) struct TakerPreimage {
    pub(crate) base_coin_fee: TradeFeeResponse,
    pub(crate) rel_coin_fee: TradeFeeResponse,
    pub(crate) taker_fee: TradeFeeResponse,
    pub(crate) fee_to_send_taker_fee: TradeFeeResponse,
    pub(crate) total_fees: Vec<TotalTradeFeeResponse>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct TradeFeeResponse {
    pub(crate) coin: String,
    #[serde(flatten)]
    pub(crate) amount: DetailedAmount,
    pub(crate) paid_from_trading_vol: bool,
}

#[derive(Clone, Deserialize)]
pub(crate) struct TotalTradeFeeResponse {
    pub(crate) coin: String,
    #[serde(flatten)]
    pub(crate) amount: DetailedAmount,
    #[serde(flatten)]
    pub(crate) required_balance: DetailedRequiredBalance,
}

construct_detailed!(DetailedAmount, amount);
construct_detailed!(DetailedVolume, volume);
construct_detailed!(DetailedRequiredBalance, required_balance);

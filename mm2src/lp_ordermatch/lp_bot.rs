//
//  lp_bot.rs
//  marketmaker
//

use common::{mm_ctx::{from_ctx, MmArc},
             mm_number::MmNumber};
use derive_more::Display;
use futures::lock::Mutex as AsyncMutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, sync::Arc};

#[cfg(test)] use mocktopus::macros::*;

#[path = "simple_market_maker.rs"] mod simple_market_maker_bot;
pub use simple_market_maker_bot::{process_price_request, start_simple_market_maker_bot, stop_simple_market_maker_bot,
                                  StartSimpleMakerBotRequest};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "simple_market_maker_tests.rs"]
pub mod simple_market_maker_tests;

#[derive(PartialEq)]
enum TradingBotState {
    Running,
    Stopping,
    Stopped,
}

impl Default for TradingBotState {
    fn default() -> Self { TradingBotState::Stopped }
}

pub type SimpleMakerBotRegistry = HashMap<String, SimpleCoinMarketMakerCfg>;

#[derive(Debug, Serialize, Deserialize, Display, Clone)]
#[display(fmt = "{} {} {} {}", base, rel, enable, spread)]
pub struct SimpleCoinMarketMakerCfg {
    pub base: String,
    pub rel: String,
    pub min_volume: Option<MmNumber>,
    pub spread: MmNumber,
    pub base_confs: Option<u64>,
    pub base_nota: Option<bool>,
    pub rel_confs: Option<u64>,
    pub rel_nota: Option<bool>,
    pub enable: bool,
    pub price_elapsed_validity: Option<f64>,
    pub check_last_bidirectional_trade_thresh_hold: Option<bool>,
    pub max: Option<bool>,
    pub balance_percent: Option<MmNumber>,
}

#[derive(Default)]
pub struct TickerInfosRegistry(HashMap<String, TickerInfos>);

#[derive(Debug, Serialize, Deserialize)]
pub struct TickerInfos {
    ticker: String,
    last_price: MmNumber,
    last_updated: String,
    last_updated_timestamp: u64,
    #[serde(rename = "volume24h")]
    volume24_h: MmNumber,
    price_provider: Provider,
    volume_provider: Provider,
    #[serde(rename = "sparkline_7d")]
    sparkline_7_d: Option<Vec<f64>>,
    sparkline_provider: Provider,
    #[serde(rename = "change_24h")]
    change_24_h: MmNumber,
    #[serde(rename = "change_24h_provider")]
    change_24_h_provider: Provider,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum Provider {
    #[serde(rename = "binance")]
    Binance,
    #[serde(rename = "coingecko")]
    Coingecko,
    #[serde(rename = "coinpaprika")]
    Coinpaprika,
    #[serde(rename = "unknown")]
    Unknown,
}

impl Default for Provider {
    fn default() -> Self { Provider::Unknown }
}

#[derive(Default)]
pub struct TradingBotContext {
    trading_bot_states: AsyncMutex<TradingBotState>,
    trading_bot_cfg: AsyncMutex<SimpleMakerBotRegistry>,
}

#[derive(Default, Clone, Debug)]
pub struct RateInfos {
    base: String,
    rel: String,
    price: MmNumber,
    last_updated_timestamp: u64,
    base_provider: Provider,
    rel_provider: Provider,
}

impl RateInfos {
    pub fn retrieve_elapsed_times(&self) -> SystemTime {
        let last_updated_time = UNIX_EPOCH + Duration::from_secs(self.last_updated_timestamp);
        let time_diff: SystemTime = SystemTime::now() - last_updated_time.elapsed().unwrap();
        time_diff
    }

    pub fn new(base: String, rel: String) -> RateInfos {
        RateInfos {
            base,
            rel,
            base_provider: Provider::Unknown,
            rel_provider: Provider::Unknown,
            ..Default::default()
        }
    }
}

impl TickerInfosRegistry {
    fn get_infos(&self, ticker: &str) -> Option<&TickerInfos> { self.0.get(ticker) }

    fn get_infos_pair(&self, base: &str, rel: &str) -> Option<(&TickerInfos, &TickerInfos)> {
        self.get_infos(base).zip(self.get_infos(rel))
    }

    pub fn get_cex_rates(&self, base: String, rel: String) -> Option<RateInfos> {
        match self.get_infos_pair(&base, &rel) {
            Some((base_price_infos, rel_price_infos)) => {
                let mut rate_infos = RateInfos::new(base, rel);
                if base_price_infos.price_provider == Provider::Unknown
                    || rel_price_infos.price_provider == Provider::Unknown
                {
                    return None;
                }

                rate_infos.base_provider = base_price_infos.price_provider.clone();
                rate_infos.rel_provider = rel_price_infos.price_provider.clone();
                rate_infos.last_updated_timestamp =
                    if base_price_infos.last_updated_timestamp <= rel_price_infos.last_updated_timestamp {
                        base_price_infos.last_updated_timestamp
                    } else {
                        rel_price_infos.last_updated_timestamp
                    };
                rate_infos.price = &base_price_infos.last_price / &rel_price_infos.last_price;
                Some(rate_infos)
            },
            None => None,
        }
    }
}

#[cfg_attr(test, mockable)]
impl TradingBotContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    fn from_ctx(ctx: &MmArc) -> Result<Arc<TradingBotContext>, String> {
        Ok(try_s!(from_ctx(&ctx.simple_market_maker_bot_ctx, move || {
            Ok(TradingBotContext::default())
        })))
    }
}

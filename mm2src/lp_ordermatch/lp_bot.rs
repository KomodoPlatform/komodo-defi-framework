//
//  lp_bot.rs
//  marketmaker
//

use async_trait::async_trait;
use common::event_dispatcher::{EventListener, Events};
use common::log::info;
use common::{mm_ctx::{from_ctx, MmArc},
             mm_number::MmNumber};
use derive_more::Display;
use futures::lock::Mutex as AsyncMutex;
#[cfg(test)] use mocktopus::macros::*;
use std::ops::Deref;
use std::{collections::HashMap, sync::Arc};

#[path = "simple_market_maker.rs"] mod simple_market_maker_bot;
use crate::mm2::lp_dispatcher::LpEvents;
use crate::mm2::lp_ordermatch::lp_bot::simple_market_maker_bot::PRECISION_FOR_NOTIFICATION;
use crate::mm2::message_service::MessageService;
pub use simple_market_maker_bot::{process_price_request, start_simple_market_maker_bot, stop_simple_market_maker_bot,
                                  StartSimpleMakerBotRequest, KMD_PRICE_ENDPOINT};

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
    #[serde(rename = "min_volume")]
    pub min_volume_percentage: Option<MmNumber>,
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
    price_url: AsyncMutex<String>,
    message_service: AsyncMutex<MessageService>,
    bot_refresh_rate: AsyncMutex<f64>,
}

#[async_trait]
impl EventListener for TradingBotContext {
    type Event = LpEvents;

    fn process_event(&self, _event: Self::Event) { unimplemented!() }

    async fn process_event_async(&self, event: Self::Event) {
        if let LpEvents::MakerSwapStatusChanged {
            uuid,
            taker_coin,
            maker_coin,
            taker_amount,
            maker_amount,
            event_status,
        } = event
        {
            let msg = format!(
                "[{}: {} ({}) <-> {} ({})] status changed: {}",
                uuid,
                taker_coin,
                taker_amount.with_prec(PRECISION_FOR_NOTIFICATION),
                maker_coin,
                maker_amount.with_prec(PRECISION_FOR_NOTIFICATION),
                event_status
            );
            info!("event received: {}", msg);
            let message_service = self.message_service.lock().await;
            let _ = message_service.send_message(msg.to_string(), false).await;
        }
    }

    fn get_desired_events(&self) -> Events<Self::Event> {
        vec![LpEvents::MakerSwapStatusChanged {
            uuid: Default::default(),
            taker_coin: "".to_string(),
            maker_coin: "".to_string(),
            taker_amount: Default::default(),
            maker_amount: Default::default(),
            event_status: "".to_string(),
        }]
    }
}

#[derive(Clone)]
pub struct ArcTradingBotContext(Arc<TradingBotContext>);

impl Deref for ArcTradingBotContext {
    type Target = TradingBotContext;
    fn deref(&self) -> &TradingBotContext { &*self.0 }
}

#[async_trait]
impl EventListener for ArcTradingBotContext {
    type Event = LpEvents;
    fn process_event(&self, ev: Self::Event) { self.0.process_event(ev); }

    async fn process_event_async(&self, event: Self::Event) { self.0.process_event_async(event).await }

    fn get_desired_events(&self) -> Events<Self::Event> { self.0.get_desired_events() }
}

#[derive(Default, Clone, Debug)]
pub struct RateInfos {
    base: String,
    rel: String,
    price: MmNumber,
    last_updated_timestamp: Option<u64>,
    base_provider: Provider,
    rel_provider: Provider,
}

impl RateInfos {
    pub fn retrieve_elapsed_times(&self) -> f64 {
        let time_diff: f64 = common::now_float() - self.last_updated_timestamp.unwrap_or_default() as f64;
        time_diff
    }

    pub fn new(base: String, rel: String) -> RateInfos {
        RateInfos {
            base,
            rel,
            base_provider: Provider::Unknown,
            rel_provider: Provider::Unknown,
            last_updated_timestamp: None,
            ..Default::default()
        }
    }
}

impl TickerInfosRegistry {
    fn get_infos(&self, ticker: &str) -> Option<&TickerInfos> {
        let mut ticker_infos = self.0.get(ticker);
        let limit = ticker.len() - 1;
        let pos = ticker.find('-').unwrap_or(limit);
        if ticker_infos.is_none() && pos < limit {
            ticker_infos = self.0.get(&ticker[0..pos])
        }
        ticker_infos
    }

    fn get_infos_pair(&self, base: &str, rel: &str) -> Option<(&TickerInfos, &TickerInfos)> {
        self.get_infos(base).zip(self.get_infos(rel))
    }

    pub fn get_cex_rates(&self, base: String, rel: String) -> Option<RateInfos> {
        match self.get_infos_pair(&base, &rel) {
            Some((base_price_infos, rel_price_infos)) => {
                let mut rate_infos = RateInfos::new(base, rel);
                if base_price_infos.price_provider == Provider::Unknown
                    || rel_price_infos.price_provider == Provider::Unknown
                    || base_price_infos.last_updated_timestamp == 0
                    || rel_price_infos.last_updated_timestamp == 0
                {
                    return None;
                }

                rate_infos.base_provider = base_price_infos.price_provider.clone();
                rate_infos.rel_provider = rel_price_infos.price_provider.clone();
                rate_infos.last_updated_timestamp =
                    if base_price_infos.last_updated_timestamp <= rel_price_infos.last_updated_timestamp {
                        Some(base_price_infos.last_updated_timestamp)
                    } else {
                        Some(rel_price_infos.last_updated_timestamp)
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
    fn from_ctx(ctx: &MmArc) -> Result<ArcTradingBotContext, String> {
        let arc_bot_context = try_s!(from_ctx(&ctx.simple_market_maker_bot_ctx, move || {
            Ok(TradingBotContext::default())
        }));
        Ok(ArcTradingBotContext(arc_bot_context))
    }
}

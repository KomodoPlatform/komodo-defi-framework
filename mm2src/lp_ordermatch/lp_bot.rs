//
//  lp_bot.rs
//  marketmaker
//

use async_trait::async_trait;
use common::event_dispatcher::{EventListener, EventUniqueId};
use common::log::info;
use common::{mm_ctx::{from_ctx, MmArc},
             mm_number::MmNumber};
use derive_more::Display;
use futures::lock::Mutex as AsyncMutex;
#[cfg(test)] use mocktopus::macros::*;
use std::any::TypeId;
use std::ops::Deref;
use std::{collections::HashMap, sync::Arc};

#[path = "simple_market_maker.rs"] mod simple_market_maker_bot;
use crate::mm2::lp_dispatcher::LpEvents;
use crate::mm2::lp_ordermatch::lp_bot::simple_market_maker_bot::{BOT_DEFAULT_REFRESH_RATE, PRECISION_FOR_NOTIFICATION};
use crate::mm2::lp_swap::MakerSwapStatusChanged;
use crate::mm2::message_service::MessageService;
pub use simple_market_maker_bot::{process_price_request, start_simple_market_maker_bot, stop_simple_market_maker_bot,
                                  StartSimpleMakerBotRequest, KMD_PRICE_ENDPOINT};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "simple_market_maker_tests.rs"]
pub mod simple_market_maker_tests;

#[derive(Clone, Display)]
#[display(fmt = "simple_market_maker_bot will stop within {} seconds", bot_refresh_rate)]
pub struct TradingBotStopping {
    bot_refresh_rate: f64,
}

impl TradingBotStopping {
    fn event_id() -> TypeId { TypeId::of::<TradingBotStopping>() }
}

#[derive(Clone, Display)]
#[display(fmt = "simple_market_maker_bot successfully started with {} pairs", nb_pairs)]
pub struct TradingBotStarted {
    nb_pairs: usize,
}

impl TradingBotStarted {
    fn event_id() -> TypeId { TypeId::of::<TradingBotStarted>() }
}

#[derive(Clone, Display)]
pub enum TradingBotEvent {
    Started(TradingBotStarted),
    Stopping(TradingBotStopping),
}

impl EventUniqueId for TradingBotEvent {
    fn event_id(&self) -> TypeId {
        match self {
            TradingBotEvent::Started(_) => TradingBotStarted::event_id(),
            TradingBotEvent::Stopping(_) => TradingBotStopping::event_id(),
        }
    }
}

impl From<TradingBotStopping> for TradingBotEvent {
    fn from(trading_bot_stopping: TradingBotStopping) -> Self { TradingBotEvent::Stopping(trading_bot_stopping) }
}

impl From<TradingBotStarted> for TradingBotEvent {
    fn from(trading_bot_started: TradingBotStarted) -> Self { TradingBotEvent::Started(trading_bot_started) }
}

pub struct RunningState {
    trading_bot_cfg: SimpleMakerBotRegistry,
    bot_refresh_rate: f64,
    price_url: String,
}

pub struct StoppingState {
    trading_bot_cfg: SimpleMakerBotRegistry,
}

#[derive(Default)]
pub struct StoppedState {
    trading_bot_cfg: SimpleMakerBotRegistry,
}

enum TradingBotState {
    Running(RunningState),
    Stopping(StoppingState),
    Stopped(StoppedState),
}

impl From<RunningState> for TradingBotState {
    fn from(running_state: RunningState) -> Self { Self::Running(running_state) }
}

impl From<StoppingState> for TradingBotState {
    fn from(stopping_state: StoppingState) -> Self { Self::Stopping(stopping_state) }
}

impl From<StoppedState> for TradingBotState {
    fn from(stopped_state: StoppedState) -> Self { Self::Stopped(stopped_state) }
}

impl Default for TradingBotState {
    fn default() -> Self { StoppedState::default().into() }
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
    pub min_base_price: Option<MmNumber>,
    pub min_rel_price: Option<MmNumber>,
    pub min_pair_price: Option<MmNumber>,
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
    message_service: AsyncMutex<MessageService>,
}

impl TradingBotContext {
    async fn get_refresh_rate(&self) -> f64 {
        let state = self.trading_bot_states.lock().await;
        if let TradingBotState::Running(running_state) = &*state {
            return running_state.bot_refresh_rate;
        }
        BOT_DEFAULT_REFRESH_RATE
    }
}

#[derive(Clone)]
pub struct ArcTradingBotContext(Arc<TradingBotContext>);

impl Deref for ArcTradingBotContext {
    type Target = TradingBotContext;
    fn deref(&self) -> &TradingBotContext { &*self.0 }
}

#[allow(clippy::single_match)]
impl TradingBotContext {
    async fn bot_dispatch_msg(&self, msg_format: String) {
        info!("{}", msg_format);
        let message_service = self.message_service.lock().await;
        let _ = message_service.send_message(msg_format, false).await;
    }

    async fn on_trading_bot_event(&self, trading_bot_event: &TradingBotEvent) {
        let msg_format = format!("{}", trading_bot_event);
        match trading_bot_event {
            TradingBotEvent::Started { .. } | TradingBotEvent::Stopping { .. } => {
                self.bot_dispatch_msg(msg_format).await
            },
        }
    }

    async fn on_maker_swap_status_changed(&self, swap_infos: &MakerSwapStatusChanged) {
        let msg = format!(
            "[{}: {} ({}) <-> {} ({})] status changed: {}",
            swap_infos.uuid,
            swap_infos.taker_coin,
            swap_infos.taker_amount.with_prec(PRECISION_FOR_NOTIFICATION),
            swap_infos.maker_coin,
            swap_infos.maker_amount.with_prec(PRECISION_FOR_NOTIFICATION),
            swap_infos.event_status
        );
        info!("event received: {}", msg);
        let state = self.trading_bot_states.lock().await;
        match &*state {
            TradingBotState::Running(_) => {
                let message_service = self.message_service.lock().await;
                let _ = message_service.send_message(msg.to_string(), false).await;
            },
            _ => {},
        }
    }
}

#[async_trait]
impl EventListener for ArcTradingBotContext {
    type Event = LpEvents;

    async fn process_event_async(&self, event: Self::Event) {
        match &event {
            LpEvents::MakerSwapStatusChanged(swap_infos) => self.on_maker_swap_status_changed(swap_infos).await,
            LpEvents::TradingBotEvent(trading_bot_event) => self.on_trading_bot_event(trading_bot_event).await,
        }
    }

    fn get_desired_events(&self) -> Vec<TypeId> {
        vec![
            MakerSwapStatusChanged::event_id(),
            TradingBotStopping::event_id(),
            TradingBotStarted::event_id(),
        ]
    }

    fn listener_id(&self) -> &'static str { "lp_bot_listener" }
}

#[derive(Default, Clone, Debug)]
pub struct RateInfos {
    base: String,
    rel: String,
    base_price: MmNumber,
    rel_price: MmNumber,
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
                rate_infos.base_price = base_price_infos.last_price.clone();
                rate_infos.rel_price = rel_price_infos.last_price.clone();
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

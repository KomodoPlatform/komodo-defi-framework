use crate::mm2::lp_ordermatch::lp_bot::{RateInfos, TickerInfosRegistry};
use crate::mm2::lp_ordermatch::{update_maker_order, MakerOrderUpdateReq};
use crate::{mm2::lp_ordermatch::lp_bot::TickerInfos,
            mm2::lp_ordermatch::lp_bot::{Provider, SimpleCoinMarketMakerCfg, SimpleMakerBotRegistry,
                                         TradingBotContext, TradingBotState},
            mm2::lp_ordermatch::{cancel_order, create_maker_order, MakerOrder, OrdermatchContext, SetPriceReq}};
use bigdecimal::Zero;
use coins::lp_coinfind;
use common::mm_number::MmNumber;
use common::{executor::{spawn, Timer},
             log::{error, info, warn},
             mm_ctx::MmArc,
             mm_error::MmError,
             slurp_url, HttpStatusCode};
use derive_more::Display;
use futures::compat::Future01CompatExt;
use http::StatusCode;
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};
use std::str::Utf8Error;
use uuid::Uuid;

// !< constants
const KMD_PRICE_ENDPOINT: &str = "https://prices.komodo.live:1313/api/v1/tickers";

// !< Type definitions
pub type StartSimpleMakerBotResult = Result<StartSimpleMakerBotRes, MmError<StartSimpleMakerBotError>>;
pub type StopSimpleMakerBotResult = Result<StopSimpleMakerBotRes, MmError<StopSimpleMakerBotError>>;
pub type OrderProcessingResult = Result<bool, MmError<OrderProcessingError>>;

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum OrderProcessingError {
    #[display(fmt = "The provider is unknown - skipping")]
    ProviderUnknown,
    #[display(fmt = "The rates price is zero - skipping")]
    PriceIsZero,
    #[display(fmt = "The rates last updated timestamp is invalid - skipping")]
    LastUpdatedTimestampInvalid,
    #[display(fmt = "The price elapsed validity is invalid - skipping")]
    PriceElapsedValidityExpired,
    #[display(fmt = "Unable to parse/treat elapsed time - skipping")]
    PriceElapsedValidityUntreatable,
    #[display(fmt = "Asset not enabled - skipping")]
    AssetNotEnabled,
    #[display(fmt = "Internal coin find error - skipping")]
    InternalCoinFindError,
    #[display(fmt = "Internal error when retrieving balance - skipping")]
    BalanceInternalError,
    #[display(fmt = "Balance is zero - skipping")]
    BalanceIsZero,
    #[display(fmt = "Error when creating the order")]
    OrderCreationError,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct StartSimpleMakerBotRequest {
    cfg: SimpleMakerBotRegistry,
}

#[cfg(test)]
impl StartSimpleMakerBotRequest {
    pub fn new() -> StartSimpleMakerBotRequest {
        return StartSimpleMakerBotRequest {
            cfg: Default::default(),
        };
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StopSimpleMakerBotRes {
    result: String,
}

#[cfg(test)]
impl StopSimpleMakerBotRes {
    pub fn get_result(&self) -> String { self.result.clone() }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StartSimpleMakerBotRes {
    result: String,
}

#[cfg(test)]
impl StartSimpleMakerBotRes {
    pub fn get_result(&self) -> String { self.result.clone() }
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum StopSimpleMakerBotError {
    #[display(fmt = "The bot is already stopped")]
    AlreadyStopped,
    #[display(fmt = "The bot is already stopping")]
    AlreadyStopping,
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum StartSimpleMakerBotError {
    #[display(fmt = "The bot is already started")]
    AlreadyStarted,
    #[display(fmt = "Invalid bot configuration")]
    InvalidBotConfiguration,
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

#[derive(Debug)]
pub enum PriceServiceRequestError {
    HttpProcessError(String),
    ParsingAnswerError(String),
}

impl From<std::string::String> for PriceServiceRequestError {
    fn from(error: String) -> Self { PriceServiceRequestError::HttpProcessError(error) }
}

impl From<std::str::Utf8Error> for PriceServiceRequestError {
    fn from(error: Utf8Error) -> Self { PriceServiceRequestError::HttpProcessError(error.to_string()) }
}

impl HttpStatusCode for StartSimpleMakerBotError {
    fn status_code(&self) -> StatusCode {
        match self {
            StartSimpleMakerBotError::AlreadyStarted | StartSimpleMakerBotError::InvalidBotConfiguration => {
                StatusCode::BAD_REQUEST
            },
            StartSimpleMakerBotError::Transport(_) | StartSimpleMakerBotError::InternalError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

impl HttpStatusCode for StopSimpleMakerBotError {
    fn status_code(&self) -> StatusCode {
        match self {
            // maybe bad request is not adapted for the first errors.
            StopSimpleMakerBotError::AlreadyStopped | StopSimpleMakerBotError::AlreadyStopping => {
                StatusCode::BAD_REQUEST
            },
            StopSimpleMakerBotError::Transport(_) | StopSimpleMakerBotError::InternalError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

struct TradingPair {
    base: String,
    rel: String,
}

impl TradingPair {
    pub fn new(base: String, rel: String) -> TradingPair { TradingPair { base, rel } }

    pub fn as_combination(&self) -> String { self.base.clone() + "/" + self.rel.clone().as_str() }
}

pub async fn tear_down_bot(ctx: MmArc) {
    let simple_market_maker_bot_ctx = TradingBotContext::from_ctx(&ctx).unwrap();
    {
        let mut trading_bot_cfg = simple_market_maker_bot_ctx.trading_bot_cfg.lock().await;
        // todo: check if clear is adapted, if i understand its keep the memory allocated for later usage.
        trading_bot_cfg.clear();
    }
    // todo: cancel all pending orders
}

// This function check if coin is enabled and if check_balance is set to true and the balance is non-zero it's returned for later usage
// e.g i want to use 50% of my KMD balance instead of max, will be used later.
async fn coin_find_and_checks(
    ticker: String,
    key_trade_pair: String,
    check_balance: bool,
    ctx: &MmArc,
) -> Result<MmNumber, MmError<OrderProcessingError>> {
    let coin = match lp_coinfind(ctx, ticker.as_str()).await {
        Ok(None) => {
            warn!("{} not enabled - skipping for {}", ticker, key_trade_pair);
            return MmError::err(OrderProcessingError::AssetNotEnabled);
        },
        Err(err) => {
            warn!(
                "err with {} - reason: {} - skipping for {}",
                ticker, err, key_trade_pair
            );
            return MmError::err(OrderProcessingError::InternalCoinFindError);
        },
        Ok(Some(t)) => t,
    };

    if check_balance {
        let coin_balance = match coin.my_balance().compat().await {
            Ok(coin_balance) => coin_balance,
            Err(err) => {
                warn!(
                    "err with balance: {} - reason: {} - skipping for {}",
                    ticker,
                    err.to_string(),
                    key_trade_pair
                );
                return MmError::err(OrderProcessingError::BalanceInternalError);
            },
        };
        if coin_balance.spendable.is_zero() {
            warn!("balance for: {} is zero - skipping for {}", ticker, key_trade_pair);
            return MmError::err(OrderProcessingError::BalanceIsZero);
        }
        return Ok(MmNumber::from(coin_balance.spendable));
    }
    Ok(MmNumber::default())
}

async fn vwap_apply(_calculated_price: &mut MmNumber) {}

async fn cancel_single_order(ctx: &MmArc, uuid: Uuid) {
    info!("cancelling single order with uuid: {}", uuid);
    let resp = match cancel_order(ctx.clone(), json!({"uuid": uuid.to_string()})).await {
        Ok(resp) => resp,
        Err(_) => {
            warn!("Couldn't cancel the order with uuid: {}", uuid);
            return;
        },
    };

    if resp.status() == StatusCode::OK {
        info!("Order with uuid: {} successfully cancelled", uuid);
    }
}

async fn checks_order_prerequisites(
    rates: &RateInfos,
    cfg: &SimpleCoinMarketMakerCfg,
    key_trade_pair: String,
) -> OrderProcessingResult {
    if rates.base_provider == Provider::Unknown || rates.rel_provider == Provider::Unknown {
        warn!("rates from provider are Unknown - skipping for {}", key_trade_pair);
        return MmError::err(OrderProcessingError::ProviderUnknown);
    }

    if rates.price.is_zero() {
        warn!("price from provider is zero - skipping for {}", key_trade_pair);
        return MmError::err(OrderProcessingError::PriceIsZero);
    }

    if rates.last_updated_timestamp == 0 {
        warn!(
            "last updated price timestamp is invalid - skipping for {}",
            key_trade_pair
        );
        return MmError::err(OrderProcessingError::LastUpdatedTimestampInvalid);
    }

    // Elapsed validity is the field defined in the cfg or 5 min by default (300 sec)
    let time_diff = rates.retrieve_elapsed_times();
    let elapsed = match time_diff.elapsed() {
        Ok(elapsed) => elapsed.as_secs_f64(),
        Err(_) => return MmError::err(OrderProcessingError::PriceElapsedValidityUntreatable),
    };
    let elapsed_validity = cfg.price_elapsed_validity.unwrap_or(300.0);

    if elapsed > elapsed_validity {
        warn!(
            "last updated price timestamp elapsed {} is more than the elapsed validity {} - skipping for {}",
            elapsed, elapsed_validity, key_trade_pair,
        );
        return MmError::err(OrderProcessingError::PriceElapsedValidityExpired);
    }
    info!("elapsed since last price update: {} secs", elapsed);
    Ok(true)
}

async fn update_single_order(
    cfg: SimpleCoinMarketMakerCfg,
    uuid: Uuid,
    _order: MakerOrder,
    key_trade_pair: String,
    ctx: &MmArc,
) -> OrderProcessingResult {
    info!("need to update order: {} of {} - cfg: {}", uuid, key_trade_pair, cfg);
    let simple_market_maker_bot_ctx = TradingBotContext::from_ctx(ctx).unwrap();
    let registry = simple_market_maker_bot_ctx.price_tickers_registry.lock().await;
    let rates = registry.get_cex_rates(cfg.base.clone(), cfg.rel.clone());
    drop(registry);
    match checks_order_prerequisites(&rates, &cfg, key_trade_pair.clone()).await {
        Ok(x) => x,
        Err(err) => {
            cancel_single_order(ctx, uuid).await;
            return Err(err);
        },
    };

    let base_balance = coin_find_and_checks(cfg.base.clone(), key_trade_pair.clone(), true, ctx).await?;
    coin_find_and_checks(cfg.rel.clone(), key_trade_pair.clone(), false, ctx).await?;

    info!("balance for {} is {}", cfg.base, base_balance);

    let mut calculated_price = rates.price * cfg.spread;
    info!("calculated price is: {}", calculated_price);
    if cfg.check_last_bidirectional_trade_thresh_hold.unwrap_or(false) {
        vwap_apply(&mut calculated_price).await;
    }

    // I sell 1 KMD because 50 % percent of the balance is 1 KMD -> order.max_base_vol -> 1
    let volume = match cfg.balance_percent {
        Some(balance_percent) => balance_percent * base_balance.clone(),
        None => MmNumber::default(),
    };

    let min_vol: Option<MmNumber> = match cfg.min_volume {
        Some(min_volume) => {
            if cfg.max.unwrap_or(false) {
                Some(min_volume * base_balance.clone())
            } else {
                Some(min_volume * volume.clone())
            }
        },
        None => None,
    };

    let req = MakerOrderUpdateReq {
        uuid,
        new_price: Some(calculated_price),
        max: cfg.max,
        volume_delta: None,
        min_volume: min_vol,
        base_confs: cfg.base_confs,
        base_nota: cfg.base_nota,
        rel_confs: cfg.rel_confs,
        rel_nota: cfg.rel_nota,
    };

    let resp = match update_maker_order(ctx, req).await {
        Ok(x) => x,
        Err(err) => {
            warn!(
                "Couldn't update the order {} - for {} - reason: {}",
                uuid, key_trade_pair, err
            );
            cancel_single_order(ctx, uuid).await;
            return MmError::err(OrderProcessingError::OrderCreationError);
        },
    };
    info!("Successfully update order for {} - uuid: {}", key_trade_pair, resp.uuid);
    Ok(true)
}

async fn create_single_order(
    cfg: SimpleCoinMarketMakerCfg,
    key_trade_pair: String,
    ctx: &MmArc,
) -> OrderProcessingResult {
    info!("need to create order for: {} - cfg: {}", key_trade_pair, cfg);
    let simple_market_maker_bot_ctx = TradingBotContext::from_ctx(ctx).unwrap();
    let registry = simple_market_maker_bot_ctx.price_tickers_registry.lock().await;
    let rates = registry.get_cex_rates(cfg.base.clone(), cfg.rel.clone());
    drop(registry);

    checks_order_prerequisites(&rates, &cfg, key_trade_pair.clone()).await?;

    let base_balance = coin_find_and_checks(cfg.base.clone(), key_trade_pair.clone(), true, ctx).await?;
    coin_find_and_checks(cfg.rel.clone(), key_trade_pair.clone(), false, ctx).await?;

    info!("balance for {} is {}", cfg.base, base_balance);

    let mut calculated_price = rates.price * cfg.spread;
    info!("calculated price is: {}", calculated_price);
    if cfg.check_last_bidirectional_trade_thresh_hold.unwrap_or(false) {
        vwap_apply(&mut calculated_price).await;
    }

    let volume = match cfg.balance_percent {
        Some(balance_percent) => balance_percent * base_balance.clone(),
        None => MmNumber::default(),
    };

    let min_vol: Option<MmNumber> = match cfg.min_volume {
        Some(min_volume) => {
            if cfg.max.unwrap_or(false) {
                Some(min_volume * base_balance.clone())
            } else {
                Some(min_volume * volume.clone())
            }
        },
        None => None,
    };

    let req = SetPriceReq {
        base: cfg.base.clone(),
        rel: cfg.rel.clone(),
        price: calculated_price,
        max: cfg.max.unwrap_or(false),
        volume,
        min_volume: min_vol,
        cancel_previous: true,
        base_confs: cfg.base_confs,
        base_nota: cfg.base_nota,
        rel_confs: cfg.rel_confs,
        rel_nota: cfg.rel_nota,
        save_in_history: true,
    };
    let resp = match create_maker_order(ctx, req).await {
        Ok(x) => x,
        Err(err) => {
            warn!("Couldn't place the order for {} - reason: {}", key_trade_pair, err);
            return MmError::err(OrderProcessingError::OrderCreationError);
        },
    };
    info!("Successfully placed order for {} - uuid: {}", key_trade_pair, resp.uuid);
    Ok(true)
}

async fn process_bot_logic(ctx: &MmArc) {
    let simple_market_maker_bot_ctx = TradingBotContext::from_ctx(ctx).unwrap();
    // note: Copy the cfg here will not be expensive, and this will be thread safe.
    let cfg = simple_market_maker_bot_ctx.trading_bot_cfg.lock().await.clone();

    let mut memoization_pair_registry: HashSet<String> = HashSet::new();
    let ordermatch_ctx = OrdermatchContext::from_ctx(ctx).unwrap();
    let maker_orders_guard = ordermatch_ctx.my_maker_orders.lock().await;
    // I'm forced to iterate cloned orders here, otherwise i will deadlock if i need to cancel one.
    let maker_orders = maker_orders_guard.clone();
    drop(maker_orders_guard);

    info!("nb_orders: {}", maker_orders.len());

    // Iterating over maker orders and update order that are present in cfg as the key_trade_pair e.g KMD/LTC
    for (key, value) in maker_orders.iter() {
        let key_trade_pair = TradingPair::new(value.base.clone(), value.rel.clone());
        match cfg.get(&key_trade_pair.as_combination()) {
            Some(coin_cfg) => {
                // res will be used later for reporting error to the users, also usefullt o be coupled with a telegram service to send notification to the user
                let _res = update_single_order(
                    coin_cfg.clone(),
                    *key,
                    value.clone(),
                    key_trade_pair.as_combination(),
                    ctx,
                )
                .await;
                memoization_pair_registry.insert(key_trade_pair.as_combination());
            },
            _ => continue,
        }
        println!("{}", key);
    }

    // Now iterate over the registry and for every pairs that are not hit let's create an order
    for (trading_pair, cur_cfg) in cfg.iter() {
        match memoization_pair_registry.get(trading_pair) {
            Some(_) => continue,
            None => {
                // res will be used later for reporting error to the users, also usefullt o be coupled with a telegram service to send notification to the user
                let _res = create_single_order(cur_cfg.clone(), trading_pair.clone(), ctx).await;
            },
        };
    }
}

pub async fn lp_bot_loop(ctx: MmArc) {
    info!("lp_bot_loop successfully started");
    loop {
        // todo: this log should probably in debug
        info!("tick lp_bot_loop");
        if ctx.is_stopping() {
            // todo: can we cancel all the pending orders when the ctx is stopping or call tear_down ?
            break;
        }
        let simple_market_maker_bot_ctx = TradingBotContext::from_ctx(&ctx).unwrap();
        let mut states = simple_market_maker_bot_ctx.trading_bot_states.lock().await;
        if *states == TradingBotState::Stopping {
            *states = TradingBotState::Stopped;
            // todo: verify if there is a possible deadlock here if i use states inside tear_down_bot
            tear_down_bot(ctx).await;
            break;
        }
        drop(states);
        process_bot_logic(&ctx).await;
        Timer::sleep(30.0).await;
    }
    info!("lp_bot_loop successfully stopped");
}

pub async fn process_price_request() -> Result<TickerInfosRegistry, MmError<PriceServiceRequestError>> {
    info!("Fetching price from: {}", KMD_PRICE_ENDPOINT);
    let (status, headers, body) = slurp_url(KMD_PRICE_ENDPOINT).await?;
    let (status_code, body, _) = (status, std::str::from_utf8(&body)?.trim().into(), headers);
    if status_code != StatusCode::OK {
        return MmError::err(PriceServiceRequestError::HttpProcessError(body));
    }
    let model: HashMap<String, TickerInfos> = match serde_json::from_str(&body) {
        Ok(model) => model,
        Err(err) => {
            return MmError::err(PriceServiceRequestError::ParsingAnswerError(err.to_string()));
        },
    };

    Ok(TickerInfosRegistry(model))
}

async fn fetch_price_tickers(ctx: &MmArc) -> Result<bool, MmError<PriceServiceRequestError>> {
    let model = process_price_request().await?;
    let simple_market_maker_bot_ctx = TradingBotContext::from_ctx(ctx).unwrap();
    let mut price_registry = simple_market_maker_bot_ctx.price_tickers_registry.lock().await;
    *price_registry = model;
    info!("registry size: {}", price_registry.0.len());
    Ok(true)
}

pub async fn lp_price_service_loop(ctx: MmArc) {
    info!("lp_price_service successfully started");
    loop {
        // todo: this log should probably in debug
        info!("tick lp_price_service_loop");
        if ctx.is_stopping() {
            break;
        }

        let simple_market_maker_bot_ctx = TradingBotContext::from_ctx(&ctx).unwrap();
        let states = simple_market_maker_bot_ctx.trading_bot_states.lock().await;
        if *states == TradingBotState::Stopping {
            info!("stop price service loop");
            break;
        }
        drop(states);
        match fetch_price_tickers(&ctx).await {
            Ok(_) => info!("price successfully fetched"),
            Err(err) => error!("error during fetching price: {:?}", err),
        };
        Timer::sleep(20.0).await;
    }
    info!("lp_price_service successfully stopped");
}

pub async fn start_simple_market_maker_bot(ctx: MmArc, req: StartSimpleMakerBotRequest) -> StartSimpleMakerBotResult {
    let simple_market_maker_bot_ctx = TradingBotContext::from_ctx(&ctx).unwrap();
    {
        let mut states = simple_market_maker_bot_ctx.trading_bot_states.lock().await;
        if *states == TradingBotState::Running {
            return MmError::err(StartSimpleMakerBotError::AlreadyStarted);
        }
        let mut trading_bot_cfg = simple_market_maker_bot_ctx.trading_bot_cfg.lock().await;
        *trading_bot_cfg = req.cfg;
        *states = TradingBotState::Running;
    }

    info!("simple_market_maker_bot successfully started");
    spawn(lp_price_service_loop(ctx.clone()));
    spawn(lp_bot_loop(ctx.clone()));
    Ok(StartSimpleMakerBotRes {
        result: "Success".to_string(),
    })
}

pub async fn stop_simple_market_maker_bot(ctx: MmArc, _req: Json) -> StopSimpleMakerBotResult {
    let simple_market_maker_bot_ctx = TradingBotContext::from_ctx(&ctx).unwrap();
    {
        let mut state = simple_market_maker_bot_ctx.trading_bot_states.lock().await;

        match *state {
            TradingBotState::Stopped => return MmError::err(StopSimpleMakerBotError::AlreadyStopped),
            TradingBotState::Stopping => return MmError::err(StopSimpleMakerBotError::AlreadyStopping),
            _ => *state = TradingBotState::Stopping,
        }
    }
    info!("simple_market_maker_bot will stop within 30 seconds");
    Ok(StopSimpleMakerBotRes {
        result: "Success".to_string(),
    })
}

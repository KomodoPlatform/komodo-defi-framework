use crate::mm2::lp_ordermatch::{cancel_all_orders, CancelBy};
use crate::mm2::{lp_ordermatch::{cancel_order, create_maker_order,
                                 lp_bot::TickerInfos,
                                 lp_bot::{Provider, SimpleCoinMarketMakerCfg, SimpleMakerBotRegistry,
                                          TradingBotContext, TradingBotState},
                                 lp_bot::{RateInfos, TickerInfosRegistry},
                                 update_maker_order, CancelOrderReq, MakerOrder, MakerOrderUpdateReq,
                                 OrdermatchContext, SetPriceReq},
                 lp_swap::{my_recent_swaps, MyRecentSwapsAnswer, MyRecentSwapsErr, MyRecentSwapsReq, MySwapsFilter}};
use bigdecimal::Zero;
use coins::{lp_coinfind, MmCoinEnum};
use common::{executor::{spawn, Timer},
             log::{error, info, warn},
             mm_ctx::MmArc,
             mm_error::MmError,
             mm_number::MmNumber,
             slurp_url, HttpStatusCode, PagingOptions};
use derive_more::Display;
use futures::compat::Future01CompatExt;
use http::StatusCode;
use num_traits::ToPrimitive;
use serde_json::Value as Json;
use std::{collections::{HashMap, HashSet},
          num::NonZeroUsize,
          str::Utf8Error};
use uuid::Uuid;

// !< constants
const KMD_PRICE_ENDPOINT: &str = "https://prices.komodo.live:1313/api/v1/tickers";

// !< Type definitions
pub type StartSimpleMakerBotResult = Result<StartSimpleMakerBotRes, MmError<StartSimpleMakerBotError>>;
pub type StopSimpleMakerBotResult = Result<StopSimpleMakerBotRes, MmError<StopSimpleMakerBotError>>;
pub type OrderProcessingResult = Result<bool, MmError<OrderProcessingError>>;
pub type VwapProcessingResult = Result<MmNumber, MmError<OrderProcessingError>>;
pub type OrderPreparationResult = Result<(Option<MmNumber>, MmNumber, MmNumber), MmError<OrderProcessingError>>;

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
    #[display(fmt = "Error when querying swap history")]
    MyRecentSwapsError,
    #[display(fmt = "Legacy error - skipping")]
    LegacyError(String),
}

impl From<MyRecentSwapsErr> for OrderProcessingError {
    fn from(_: MyRecentSwapsErr) -> Self { OrderProcessingError::MyRecentSwapsError }
}

impl From<std::string::String> for OrderProcessingError {
    fn from(error: std::string::String) -> Self { OrderProcessingError::LegacyError(error) }
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

enum VwapCalculationSide {
    Base,
    Rel,
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
        trading_bot_cfg.clear();
    }
    cancel_pending_orders(&ctx).await;
}

async fn get_non_zero_balance(coin: MmCoinEnum) -> Result<MmNumber, MmError<OrderProcessingError>> {
    let coin_balance = match coin.my_balance().compat().await {
        Ok(coin_balance) => coin_balance,
        Err(err) => {
            warn!("err with balance: {} - reason: {}", coin.ticker(), err.to_string());
            return MmError::err(OrderProcessingError::BalanceInternalError);
        },
    };
    if coin_balance.spendable.is_zero() {
        warn!("balance for: {} is zero", coin.ticker());
        return MmError::err(OrderProcessingError::BalanceIsZero);
    }
    Ok(MmNumber::from(coin_balance.spendable))
}

async fn vwap_calculation(
    kind: VwapCalculationSide,
    swaps_answer: MyRecentSwapsAnswer,
    nb_valid_trades: &mut usize,
    cfg: &SimpleCoinMarketMakerCfg,
    calculated_price: MmNumber,
) -> MmNumber {
    let mut average_trading_price = calculated_price.clone();
    let mut total_sum_price_volume = MmNumber::default();
    let mut total_volume = MmNumber::default();
    for swap in swaps_answer.swaps.iter() {
        if !swap.is_finished_and_success() {
            *nb_valid_trades -= 1;
            continue;
        }
        let (my_amount, other_amount) = match swap.get_my_info() {
            Some(x) => (MmNumber::from(x.my_amount), MmNumber::from(x.other_amount)),
            None => {
                *nb_valid_trades -= 1;
                continue;
            },
        };
        // todo: refactor to a function
        let cur_sum_price_volume = match kind {
            VwapCalculationSide::Base => {
                let cur_price = my_amount / other_amount.clone();
                let cur_sum_price_volume = cur_price.clone() * other_amount.clone();
                total_volume += other_amount.clone();
                info!(
                    "[{}/{}] - price: {} - amount: {} - avgprice: {} - total volume: {}",
                    cfg.base, cfg.rel, cur_price, other_amount, average_trading_price, total_volume
                );
                cur_sum_price_volume
            },
            VwapCalculationSide::Rel => {
                let cur_price = other_amount.clone() / my_amount.clone();
                let cur_sum_price_volume = cur_price.clone() * my_amount.clone();
                total_volume += my_amount.clone();
                info!(
                    "[{}/{}] - price: {} - amount: {} - avgprice: {} - total volume: {}",
                    cfg.base, cfg.rel, cur_price, my_amount, average_trading_price, total_volume
                );
                cur_sum_price_volume
            },
        };
        total_sum_price_volume += cur_sum_price_volume;
    }
    if total_sum_price_volume.is_zero() {
        warn!("Unable to get average price from last trades - stick with calculated price");
        return calculated_price;
    }
    average_trading_price = total_sum_price_volume / total_volume;
    average_trading_price
}

async fn vwap_logic(
    base_swaps: MyRecentSwapsAnswer,
    rel_swaps: MyRecentSwapsAnswer,
    calculated_price: MmNumber,
    cfg: &SimpleCoinMarketMakerCfg,
) -> MmNumber {
    let mut nb_valid_trades = base_swaps.swaps.len() + rel_swaps.swaps.len();
    let base_swaps_empty = base_swaps.swaps.is_empty();
    let rel_swaps_empty = rel_swaps.swaps.is_empty();
    let base_vwap = vwap_calculation(
        VwapCalculationSide::Rel,
        base_swaps,
        &mut nb_valid_trades,
        cfg,
        calculated_price.clone(),
    )
    .await;
    let rel_vwap = vwap_calculation(
        VwapCalculationSide::Base,
        rel_swaps,
        &mut nb_valid_trades,
        cfg,
        calculated_price.clone(),
    )
    .await;
    if base_vwap == calculated_price && rel_vwap == calculated_price {
        return calculated_price;
    }
    let mut to_divide = 0;
    let mut total_vwap = MmNumber::default();
    if !base_swaps_empty {
        to_divide += 1;
        total_vwap += base_vwap;
    }
    if !rel_swaps_empty {
        to_divide += 1;
        total_vwap += rel_vwap;
    }
    // here divide cannot be 0 anymore because if both swaps history are empty we do not pass through this function.
    let vwap_price = total_vwap / MmNumber::from(to_divide);
    if vwap_price > calculated_price {
        info!(
            "[{}/{}]: price: {} is less than average trading price ({} swaps): - using vwap price: {}",
            cfg.base, cfg.rel, calculated_price, nb_valid_trades, vwap_price
        );
        return vwap_price;
    }
    info!("price calculated by the CEX rates {} is above the vwap price ({} swaps) {} - skipping threshold readjustment for pair: [{}/{}]", 
            calculated_price, nb_valid_trades, vwap_price, cfg.base, cfg.rel);
    calculated_price
}

pub async fn vwap(
    base_swaps: MyRecentSwapsAnswer,
    rel_swaps: MyRecentSwapsAnswer,
    calculated_price: MmNumber,
    cfg: &SimpleCoinMarketMakerCfg,
) -> MmNumber {
    // since the limit is `1000` unwrap is fine here.
    let nb_diff_swaps = rel_swaps.swaps.len().to_isize().unwrap() - base_swaps.swaps.len().to_isize().unwrap();
    let have_precedent_swaps = !rel_swaps.swaps.is_empty() && !base_swaps.swaps.is_empty();
    if nb_diff_swaps.is_zero() && !have_precedent_swaps {
        info!(
            "No last trade for trading pair: [{}/{}] - keeping calculated price: {}",
            cfg.base, cfg.rel, calculated_price
        );
        return calculated_price;
    }
    vwap_logic(base_swaps, rel_swaps, calculated_price, cfg).await
}

async fn vwap_calculator(
    calculated_price: MmNumber,
    ctx: &MmArc,
    cfg: &SimpleCoinMarketMakerCfg,
) -> VwapProcessingResult {
    let my_recent_swaps_req = async move |base: String, rel: String| MyRecentSwapsReq {
        paging_options: PagingOptions {
            limit: 1000,
            page_number: NonZeroUsize::new(1).unwrap(),
            from_uuid: None,
        },
        filter: MySwapsFilter {
            my_coin: Some(base),
            other_coin: Some(rel),
            from_timestamp: None,
            to_timestamp: None,
        },
    };
    let base_swaps = my_recent_swaps(
        ctx.clone(),
        my_recent_swaps_req(cfg.base.clone(), cfg.rel.clone()).await,
    )
    .await?;
    let rel_swaps = my_recent_swaps(
        ctx.clone(),
        my_recent_swaps_req(cfg.rel.clone(), cfg.base.clone()).await,
    )
    .await?;
    Ok(vwap(base_swaps, rel_swaps, calculated_price, cfg).await)
}

async fn cancel_pending_orders(ctx: &MmArc) {
    match cancel_all_orders(ctx.clone(), CancelBy::All).await {
        Ok(resp) => info!("Successfully deleted orders: {:?}", resp.cancelled),
        Err(err) => error!("Couldn't cancel pending orders: {}", err),
    }
}

async fn cancel_single_order(ctx: &MmArc, uuid: Uuid) {
    match cancel_order(ctx.clone(), CancelOrderReq { uuid }).await {
        Ok(_) => info!("Order with uuid: {} successfully cancelled", uuid),
        Err(_) => warn!("Couldn't cancel the order with uuid: {}", uuid),
    };
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

async fn prepare_order(
    rates: RateInfos,
    cfg: SimpleCoinMarketMakerCfg,
    key_trade_pair: String,
    ctx: &MmArc,
) -> OrderPreparationResult {
    checks_order_prerequisites(&rates, &cfg, key_trade_pair.clone()).await?;
    let base_coin = lp_coinfind(ctx, cfg.base.as_str())
        .await?
        .ok_or_else(|| MmError::new(OrderProcessingError::AssetNotEnabled))?;
    let base_balance = get_non_zero_balance(base_coin).await?;
    lp_coinfind(ctx, cfg.rel.as_str())
        .await?
        .ok_or_else(|| MmError::new(OrderProcessingError::AssetNotEnabled))?;

    info!("balance for {} is {}", cfg.base, base_balance);

    let mut calculated_price = rates.price * cfg.spread.clone();
    info!("calculated price is: {}", calculated_price);
    if cfg.check_last_bidirectional_trade_thresh_hold.unwrap_or(false) {
        calculated_price = vwap_calculator(calculated_price.clone(), ctx, &cfg).await?;
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
    Ok((min_vol, volume, calculated_price))
}

async fn update_single_order(
    rates: RateInfos,
    cfg: SimpleCoinMarketMakerCfg,
    uuid: Uuid,
    _order: MakerOrder,
    key_trade_pair: String,
    ctx: &MmArc,
) -> OrderProcessingResult {
    info!("need to update order: {} of {} - cfg: {}", uuid, key_trade_pair, cfg);
    let (min_vol, _, calculated_price) = prepare_order(rates, cfg.clone(), key_trade_pair.clone(), ctx).await?;

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
            return MmError::err(OrderProcessingError::OrderCreationError);
        },
    };
    info!("Successfully update order for {} - uuid: {}", key_trade_pair, resp.uuid);
    Ok(true)
}

async fn create_single_order(
    rates: RateInfos,
    cfg: SimpleCoinMarketMakerCfg,
    key_trade_pair: String,
    ctx: &MmArc,
) -> OrderProcessingResult {
    info!("need to create order for: {} - cfg: {}", key_trade_pair, cfg);
    let (min_vol, volume, calculated_price) = prepare_order(rates, cfg.clone(), key_trade_pair.clone(), ctx).await?;

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
    let rates_registry = match fetch_price_tickers().await {
        Ok(model) => {
            info!("price successfully fetched");
            model
        },
        Err(err) => {
            error!("error during fetching price: {:?}", err);
            cancel_pending_orders(ctx).await;
            return;
        },
    };
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
    for (uuid, value) in maker_orders.iter() {
        let key_trade_pair = TradingPair::new(value.base.clone(), value.rel.clone());
        match cfg.get(&key_trade_pair.as_combination()) {
            Some(coin_cfg) => {
                match update_single_order(
                    rates_registry.get_cex_rates(coin_cfg.base.clone(), coin_cfg.rel.clone()),
                    coin_cfg.clone(),
                    *uuid,
                    value.clone(),
                    key_trade_pair.as_combination(),
                    ctx,
                )
                .await
                {
                    Ok(_) => info!("Order with uuid: {} successfully updated", uuid),
                    Err(err) => {
                        error!(
                            "Order with uuid: {} for {} cannot be updated - {}",
                            uuid,
                            key_trade_pair.as_combination(),
                            err
                        );
                        cancel_single_order(ctx, *uuid).await;
                    },
                };
                memoization_pair_registry.insert(key_trade_pair.as_combination());
            },
            _ => continue,
        }
    }

    // Now iterate over the registry and for every pairs that are not hit let's create an order
    for (trading_pair, cur_cfg) in cfg.iter() {
        match memoization_pair_registry.get(trading_pair) {
            Some(_) => continue,
            None => {
                // res will be used later for reporting error to the users, also usefullt o be coupled with a telegram service to send notification to the user
                match create_single_order(
                    rates_registry.get_cex_rates(cur_cfg.base.clone(), cur_cfg.rel.clone()),
                    cur_cfg.clone(),
                    trading_pair.clone(),
                    ctx,
                )
                .await
                {
                    Ok(_) => {},
                    Err(err) => error!("{} order cannot be created - {}", trading_pair, err),
                };
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

async fn fetch_price_tickers() -> Result<TickerInfosRegistry, MmError<PriceServiceRequestError>> {
    let model = process_price_request().await?;
    info!("price registry size: {}", model.0.len());
    Ok(model)
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

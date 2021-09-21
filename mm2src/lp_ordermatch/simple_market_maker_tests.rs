use crate::mm2::lp_swap::{MakerSavedSwap, SavedSwap};
use crate::{mm2::lp_ordermatch::lp_bot::SimpleCoinMarketMakerCfg,
            mm2::{lp_ordermatch::lp_bot::simple_market_maker_bot::vwap_intro, lp_ordermatch::lp_bot::Provider,
                  lp_ordermatch::lp_bot::TickerInfos, lp_ordermatch::lp_bot::TradingBotContext,
                  lp_swap::MyRecentSwapsAnswer}};
use common::{block_on, log::UnifiedLoggerBuilder, mm_ctx::MmCtxBuilder, mm_number::MmNumber,
             privkey::key_pair_from_seed};

use std::num::NonZeroUsize;

fn generate_swaps_from_values(swaps_value: Vec<(MmNumber, MmNumber)>) -> MyRecentSwapsAnswer {
    let mut swaps: Vec<SavedSwap> = Vec::with_capacity(swaps_value.len());
    for (base_amount, other_amount) in swaps_value.iter() {
        swaps.push(SavedSwap::Maker(MakerSavedSwap::new(base_amount, other_amount)));
    }
    MyRecentSwapsAnswer {
        from_uuid: None,
        limit: 0,
        skipped: 0,
        total: 0,
        found_records: 0,
        page_number: NonZeroUsize::new(1).unwrap(),
        total_pages: 0,
        swaps,
    }
}

fn generate_cfg_from_params(base: String, rel: String, spread: MmNumber) -> SimpleCoinMarketMakerCfg {
    SimpleCoinMarketMakerCfg {
        base,
        rel,
        min_volume: None,
        spread,
        base_confs: None,
        base_nota: None,
        rel_confs: None,
        rel_nota: None,
        enable: true,
        price_elapsed_validity: None,
        check_last_bidirectional_trade_thresh_hold: Some(true),
        max: Some(true),
        balance_percent: None,
    }
}

mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_empty_base_rel() {
        let base_swaps = generate_swaps_from_values(vec![]);
        let rel_swaps = generate_swaps_from_values(vec![]);
        let mut calculated_price = MmNumber::from("7.14455729");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap_intro(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        assert_eq!(calculated_price, MmNumber::from("7.14455729"));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_single_base_side() {
        UnifiedLoggerBuilder::default().try_init().unwrap_or(());
        let base_swaps =
            generate_swaps_from_values(vec![(MmNumber::from("29.99997438"), MmNumber::from("222.76277576"))]);
        let rel_swaps = generate_swaps_from_values(vec![]);
        let mut calculated_price = MmNumber::from("7.14455729");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap_intro(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        let expected_price = MmNumber::from(
            "7.425432199985765454510364818518221681227982435363666467237829687773220024996568013735750396997505703",
        );
        assert_eq!(calculated_price.to_decimal(), expected_price.to_decimal());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_single_reversed_side() {
        UnifiedLoggerBuilder::default().try_init().unwrap_or(());
        let base_swaps = generate_swaps_from_values(vec![]);
        let rel_swaps = generate_swaps_from_values(vec![(MmNumber::from("219.4709"), MmNumber::from("29.99999"))]);
        let mut calculated_price = MmNumber::from("7.14455729");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap_intro(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        let expected_price = MmNumber::from(
            "7.3156991052330350776783592261197420399140133046711015570338523446174482058160686053562017854005951340",
        );
        assert_eq!(calculated_price.to_decimal(), expected_price.to_decimal());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_single_reversed_side_forced_price() {
        let base_swaps = generate_swaps_from_values(vec![]);
        let rel_swaps = generate_swaps_from_values(vec![(MmNumber::from("219.4709"), MmNumber::from("29.99999"))]);
        let mut calculated_price = MmNumber::from("7.6");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap_intro(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        assert_eq!(calculated_price, MmNumber::from("7.6"));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_multiple_reversed_side() {
        let base_swaps = generate_swaps_from_values(vec![]);
        let rel_swaps = generate_swaps_from_values(vec![
            (MmNumber::from("219.4709"), MmNumber::from("29.99999")),
            (MmNumber::from("222.762"), MmNumber::from("29.99999")),
        ]);
        let mut calculated_price = MmNumber::from("7.14455729");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap_intro(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        let expected_price = MmNumber::from(
            "7.370550790183596727865575955191985063995021331673777224592408197469399156466385488795162931720977240",
        );
        assert_eq!(calculated_price.to_decimal(), expected_price.to_decimal());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_multiple_reversed_side_forced_price() {
        let base_swaps = generate_swaps_from_values(vec![]);
        let rel_swaps = generate_swaps_from_values(vec![
            (MmNumber::from("219.4709"), MmNumber::from("29.99999")),
            (MmNumber::from("222.762"), MmNumber::from("29.99999")),
        ]);
        let mut calculated_price = MmNumber::from("7.54455729");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap_intro(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        assert_eq!(calculated_price, MmNumber::from("7.54455729"));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_get_cex_rates() {
        let ctx = MmCtxBuilder::default()
            .with_secp256k1_key_pair(
                key_pair_from_seed("also shoot benefit prefer juice shell elder veteran woman mimic image kidney")
                    .unwrap(),
            )
            .into_mm_arc();
        let trading_bot_ctx = TradingBotContext::from_ctx(&ctx).unwrap();
        let mut registry = block_on(trading_bot_ctx.price_tickers_registry.lock());
        let rates = registry.get_cex_rates("KMD".to_string(), "LTC".to_string());
        assert_eq!(rates.base_provider, Provider::Unknown);
        assert_eq!(rates.rel_provider, Provider::Unknown);

        registry.0.insert("KMD".to_string(), TickerInfos {
            ticker: "KMD".to_string(),
            last_price: MmNumber::from("10"),
            last_updated: "".to_string(),
            last_updated_timestamp: 0,
            volume24_h: MmNumber::from("25000"),
            price_provider: Provider::Binance,
            volume_provider: Provider::Coinpaprika,
            sparkline_7_d: None,
            sparkline_provider: Default::default(),
            change_24_h: MmNumber::default(),
            change_24_h_provider: Default::default(),
        });

        registry.0.insert("LTC".to_string(), TickerInfos {
            ticker: "LTC".to_string(),
            last_price: MmNumber::from("500.0"),
            last_updated: "".to_string(),
            last_updated_timestamp: 0,
            volume24_h: MmNumber::from("25000"),
            price_provider: Provider::Coingecko,
            volume_provider: Provider::Binance,
            sparkline_7_d: None,
            sparkline_provider: Default::default(),
            change_24_h: MmNumber::default(),
            change_24_h_provider: Default::default(),
        });

        let rates = registry.get_cex_rates("KMD".to_string(), "LTC".to_string());
        assert_eq!(rates.base_provider, Provider::Binance);
        assert_eq!(rates.rel_provider, Provider::Coingecko);
        assert_eq!(rates.price, MmNumber::from("0.02"));
    }
}

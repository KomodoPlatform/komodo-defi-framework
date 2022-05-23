use crate::mm2::{lp_ordermatch::lp_bot::simple_market_maker_bot::vwap,
                 lp_ordermatch::lp_bot::Provider,
                 lp_ordermatch::lp_bot::SimpleCoinMarketMakerCfg,
                 lp_ordermatch::lp_bot::TickerInfos,
                 lp_ordermatch::lp_bot::TickerInfosRegistry,
                 lp_swap::{MakerSavedSwap, MyRecentSwapsResponse, SavedSwap}};
use common::{block_on, log::UnifiedLoggerBuilder, mm_number::MmNumber};

use std::num::NonZeroUsize;

fn generate_swaps_from_values(swaps_value: Vec<(MmNumber, MmNumber)>) -> MyRecentSwapsResponse {
    let mut swaps: Vec<SavedSwap> = Vec::with_capacity(swaps_value.len());
    for (base_amount, other_amount) in swaps_value.iter() {
        swaps.push(SavedSwap::Maker(MakerSavedSwap::new(base_amount, other_amount)));
    }
    MyRecentSwapsResponse {
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
        max_volume: None,
        spread,
        base_confs: None,
        base_nota: None,
        rel_confs: None,
        rel_nota: None,
        enable: true,
        price_elapsed_validity: None,
        check_last_bidirectional_trade_thresh_hold: Some(true),
        max: Some(true),
        min_base_price: None,
        min_rel_price: None,
        min_pair_price: None,
    }
}

mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_empty_base_rel() {
        let base_swaps = generate_swaps_from_values(vec![]);
        let rel_swaps = generate_swaps_from_values(vec![]);
        let mut calculated_price = MmNumber::from("7.14455729");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        assert_eq!(calculated_price, MmNumber::from("7.14455729"));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_single_base_side() {
        UnifiedLoggerBuilder::default().try_init().unwrap_or(());
        let base_swaps =
            generate_swaps_from_values(vec![(MmNumber::from("29.99997438"), MmNumber::from("222.76277576"))]);
        let rel_swaps = generate_swaps_from_values(vec![]);
        let mut calculated_price = MmNumber::from("7.6");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        assert_eq!(calculated_price.to_decimal(), MmNumber::from("7.6").to_decimal());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_single_base_side_forced_price() {
        UnifiedLoggerBuilder::default().try_init().unwrap_or(());
        let base_swaps =
            generate_swaps_from_values(vec![(MmNumber::from("29.99997438"), MmNumber::from("222.76277576"))]);
        let rel_swaps = generate_swaps_from_values(vec![]);
        let mut calculated_price = MmNumber::from("7.14455729");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        let expected_price = MmNumber::from(
            "7.425432199985765454510364818518221681227982435363666467237829687773220024996568013735750396997505703",
        );
        assert_eq!(calculated_price.to_decimal(), expected_price.to_decimal());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_multiple_base_side() {
        UnifiedLoggerBuilder::default().try_init().unwrap_or(());
        let base_swaps = generate_swaps_from_values(vec![
            (MmNumber::from("29.99997438"), MmNumber::from("222.76277576")),
            (MmNumber::from("14.99998719"), MmNumber::from("105.38138788")),
        ]);
        let rel_swaps = generate_swaps_from_values(vec![]);
        let mut calculated_price = MmNumber::from("7.14455729");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        let expected_price = MmNumber::from(
            "7.292098752785668212293986632397917401154793919527340609682214001944090993587041856667078927018737007",
        );
        assert_eq!(calculated_price.to_decimal(), expected_price.to_decimal());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_multiple_base_side_forced_price() {
        UnifiedLoggerBuilder::default().try_init().unwrap_or(());
        let base_swaps = generate_swaps_from_values(vec![
            (MmNumber::from("29.99997438"), MmNumber::from("222.76277576")),
            (MmNumber::from("29.99997438"), MmNumber::from("190.76277576")),
        ]);
        let rel_swaps = generate_swaps_from_values(vec![]);
        let mut calculated_price = MmNumber::from("7.14455729");
        let cfg = generate_cfg_from_params("FIRO".to_string(), "KMD".to_string(), MmNumber::from("1.015"));
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        let expected_price = MmNumber::from("7.14455729");
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
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
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
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
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
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
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
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        assert_eq!(calculated_price, MmNumber::from("7.54455729"));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_vwap_multiple_trade_both_side() {
        let base_swaps = generate_swaps_from_values(vec![
            (MmNumber::from("4.102174"), MmNumber::from("752.8892")),
            (MmNumber::from("1.719676"), MmNumber::from("316.4945")),
            (MmNumber::from("0.052971"), MmNumber::from("10.24148")),
            (MmNumber::from("0.133952"), MmNumber::from("25.98481")),
        ]);
        let rel_swaps = generate_swaps_from_values(vec![
            (MmNumber::from("27.18649"), MmNumber::from("0.161065")),
            (MmNumber::from("13.17659"), MmNumber::from("0.077728")),
            (MmNumber::from("37.00451"), MmNumber::from("0.208161")),
            (MmNumber::from("743.0625"), MmNumber::from("4.266546")),
            (MmNumber::from("2819.370"), MmNumber::from("15.68015")),
            (MmNumber::from("2633.345"), MmNumber::from("14.61151")),
            (MmNumber::from("559.7940"), MmNumber::from("3.006166")),
        ]);
        let mut calculated_price = MmNumber::from("174.1375");
        let cfg = generate_cfg_from_params("LTC".to_string(), "KMD".to_string(), MmNumber::from("1.013"));
        calculated_price = block_on(vwap(base_swaps, rel_swaps, calculated_price.clone(), &cfg));
        let expected_price = MmNumber::from(
            "181.8799512443925929123148242562683065552235106947031231789891984094078454989954152150057794418761367",
        );
        assert_eq!(calculated_price.to_decimal(), expected_price.to_decimal());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_get_cex_rates() {
        let mut registry = TickerInfosRegistry::default();
        let rates = registry
            .get_cex_rates("KMD".to_string(), "LTC".to_string())
            .unwrap_or_default();
        assert_eq!(rates.base_provider, Provider::Unknown);
        assert_eq!(rates.rel_provider, Provider::Unknown);

        registry.0.insert("KMD".to_string(), TickerInfos {
            ticker: "KMD".to_string(),
            last_price: MmNumber::from("10"),
            last_updated: "".to_string(),
            last_updated_timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
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
            last_updated_timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            volume24_h: MmNumber::from("25000"),
            price_provider: Provider::Coingecko,
            volume_provider: Provider::Binance,
            sparkline_7_d: None,
            sparkline_provider: Default::default(),
            change_24_h: MmNumber::default(),
            change_24_h_provider: Default::default(),
        });

        registry.0.insert("USDT".to_string(), TickerInfos {
            ticker: "USDT".to_string(),
            last_price: MmNumber::from("1"),
            last_updated: "".to_string(),
            last_updated_timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            volume24_h: MmNumber::from("25000"),
            price_provider: Provider::Coingecko,
            volume_provider: Provider::Binance,
            sparkline_7_d: None,
            sparkline_provider: Default::default(),
            change_24_h: MmNumber::default(),
            change_24_h_provider: Default::default(),
        });

        let rates = registry
            .get_cex_rates("KMD".to_string(), "LTC".to_string())
            .unwrap_or_default();
        assert_eq!(rates.base_provider, Provider::Binance);
        assert_eq!(rates.rel_provider, Provider::Coingecko);
        assert_eq!(rates.price, MmNumber::from("0.02"));

        let usdt_infos = registry.get_infos("USDT-PLG20");
        assert_eq!(usdt_infos.is_some(), true);
        assert_eq!(usdt_infos.unwrap().last_price, MmNumber::from(1));

        let usdt_infos = registry.get_infos("USDT");
        assert_eq!(usdt_infos.is_some(), true);
        assert_eq!(usdt_infos.unwrap().last_price, MmNumber::from(1));
    }
}

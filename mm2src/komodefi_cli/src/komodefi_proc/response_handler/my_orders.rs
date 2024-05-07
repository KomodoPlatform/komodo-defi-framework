use anyhow::{anyhow, Result};
use itertools::Itertools;
use std::collections::HashMap;
use std::io::Write;
use term_table::{row::Row, table_cell::TableCell, TableStyle};
use uuid::Uuid;

use common::log::error;
use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};
use mm2_rpc::data::legacy::{MakerOrderForMyOrdersRpc, Mm2RpcResult, MyOrdersResponse, TakerOrderForRpc};

use super::formatters::{format_confirmation_settings, format_datetime_msec, format_historical_changes, format_ratio,
                        get_matches_rows, taker_order_header_row, taker_order_rows, term_table_blank,
                        write_maker_match, writeln_field, COMMON_INDENT, COMMON_PRECISION};
use crate::logging::error_anyhow;

pub(super) fn on_my_orders(writer: &mut dyn Write, response: Mm2RpcResult<MyOrdersResponse>) -> Result<()> {
    let result = response.result;
    writeln_safe_io!(writer, "{}", format_taker_orders_table(&result.taker_orders)?);
    writeln_safe_io!(writer, "{}", format_maker_orders_table(&result.maker_orders)?);
    Ok(())
}

fn format_taker_orders_table(taker_orders: &HashMap<Uuid, TakerOrderForRpc>) -> Result<String> {
    let mut buff: Vec<u8> = vec![];
    let writer: &mut dyn Write = &mut buff;
    if taker_orders.is_empty() {
        writeln_field(writer, "Taker orders", "empty", COMMON_INDENT);
    } else {
        writeln_field(writer, "Taker orders", "", COMMON_INDENT);
        let mut table = term_table_blank(TableStyle::thin(), false, false, false);
        table.add_row(taker_order_header_row());
        for (_, taker_order) in taker_orders.iter().sorted_by_key(|(_, o)| o.created_at) {
            table.rows.append(taker_order_rows(taker_order)?.as_mut());
        }
        write_safe_io!(writer, "{}", table.render());
    }
    String::from_utf8(buff).map_err(|error| error_anyhow!("Failed to format maker orders table: {error}"))
}

fn format_maker_orders_table(maker_orders: &HashMap<Uuid, MakerOrderForMyOrdersRpc>) -> Result<String> {
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;

    if maker_orders.is_empty() {
        writeln_field(writer, "Maker orders", "empty", COMMON_INDENT);
    } else {
        writeln_field(writer, "Maker orders", "", COMMON_INDENT);
        let mut table = term_table_blank(TableStyle::thin(), false, false, false);
        table.add_row(maker_order_for_my_orders_header_row());

        for (_, maker_order) in maker_orders.iter().sorted_by_key(|(_, o)| o.order.created_at) {
            table.rows.append(maker_order_for_my_orders_row(maker_order)?.as_mut());
        }
        write_safe_io!(writer, "{}", table.render());
    }
    String::from_utf8(buff).map_err(|error| error_anyhow!("Failed to format maker orders table: {error}"))
}

fn maker_order_for_my_orders_header_row() -> Row<'static> {
    Row::new(vec![
        TableCell::new("base,rel"),
        TableCell::new("price"),
        TableCell::new("uuid"),
        TableCell::new("created at,\nupdated at"),
        TableCell::new("min base vol,\nmax base vol"),
        TableCell::new("cancellable"),
        TableCell::new("available\namount"),
        TableCell::new("swaps"),
        TableCell::new("conf_settings"),
        TableCell::new("history changes"),
    ])
}

fn maker_order_for_my_orders_row(maker_order: &MakerOrderForMyOrdersRpc) -> Result<Vec<Row>> {
    let order = &maker_order.order;
    let mut rows = vec![Row::new(vec![
        TableCell::new(format!("{},{}", order.base, order.rel)),
        TableCell::new(format_ratio(&order.price_rat, COMMON_PRECISION)?),
        TableCell::new(order.uuid),
        TableCell::new(format!(
            "{},\n{}",
            format_datetime_msec(order.created_at)?,
            order.updated_at.map_or(Ok("".to_string()), format_datetime_msec)?
        )),
        TableCell::new(format!(
            "{},\n{}",
            format_ratio(&order.min_base_vol_rat, COMMON_PRECISION)?,
            format_ratio(&order.max_base_vol_rat, COMMON_PRECISION)?
        )),
        TableCell::new(maker_order.cancellable),
        TableCell::new(format_ratio(&maker_order.available_amount, COMMON_PRECISION)?),
        TableCell::new(if order.started_swaps.is_empty() {
            "empty".to_string()
        } else {
            order.started_swaps.iter().join(",\n")
        }),
        TableCell::new(
            order
                .conf_settings
                .as_ref()
                .map_or_else(|| "none".to_string(), format_confirmation_settings),
        ),
        TableCell::new(order.changes_history.as_ref().map_or_else(
            || "none".to_string(),
            |val| {
                val.iter()
                    .map(|val| format_historical_changes(val, "\n").unwrap_or_else(|_| "error".to_string()))
                    .join("\n")
            },
        )),
    ])];
    rows.append(get_matches_rows(&order.matches, 10, write_maker_match)?.as_mut());
    Ok(rows)
}

#[cfg(test)]
mod test {
    use rpc::v1::types::H256 as H256Json;
    use std::collections::{HashMap, HashSet};
    use std::str::FromStr;
    use uuid::Uuid;

    use mm2_number::bigdecimal::FromPrimitive;
    use mm2_number::{BigDecimal, BigRational};
    use mm2_rpc::data::legacy::{HistoricalOrder, MakerMatchForRpc, MakerOrderForMyOrdersRpc, MakerOrderForRpc,
                                MakerReservedForRpc, MatchBy, OrderConfirmationsSettings, TakerAction,
                                TakerRequestForRpc};

    use super::format_maker_orders_table;

    #[test]
    fn test_print_maker_orders_with_matches() {
        let taker_request = TakerRequestForRpc {
            uuid: Uuid::from_str("d9e1aaf6-eb5c-4550-a1d3-15bf4dc8727c").unwrap(),
            base: "DFG".to_string(),
            rel: "GGG".to_string(),
            base_amount: BigDecimal::from_f64(0.0023).unwrap(),
            base_amount_rat: BigRational::from_f64(0.0023).unwrap(),
            rel_amount: BigDecimal::from_f64(0.11).unwrap(),
            rel_amount_rat: BigRational::from_f64(0.11).unwrap(),
            action: TakerAction::Sell,
            method: "deprecated".to_string(),
            sender_pubkey: H256Json::from_str("15d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732")
                .unwrap(),
            dest_pub_key: H256Json::from_str("0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d0447")
                .unwrap(),
            match_by: MatchBy::Orders(HashSet::from([
                Uuid::from_str("d9e1aaf6-eb5c-4550-a1d3-15bf4dc8727e").unwrap(),
                Uuid::from_str("d9e1aaf6-eb5c-4550-a1d3-15bf4dc8727d").unwrap(),
            ])),
            conf_settings: Some(OrderConfirmationsSettings {
                base_confs: 1,
                base_nota: true,
                rel_confs: 11,
                rel_nota: false,
            }),
        };

        let reserve = MakerReservedForRpc {
            base: "TTT".to_string(),
            rel: "GGG".to_string(),
            base_amount: BigDecimal::from_f64(888.1).unwrap(),
            base_amount_rat: BigRational::from_f64(888.1).unwrap(),
            rel_amount: BigDecimal::from_f64(9921.1).unwrap(),
            rel_amount_rat: BigRational::from_f64(9912.1).unwrap(),
            taker_order_uuid: Uuid::from_str("a0e1aaf6-eb5c-4550-a1d3-15bf4dc8727d").unwrap(),
            maker_order_uuid: Uuid::from_str("b1e1aaf6-eb5c-4550-a1d3-15bf4dc8727d").unwrap(),
            sender_pubkey: H256Json::from_str("022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c8508346")
                .unwrap(),
            dest_pub_key: H256Json::from_str("022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c8508348")
                .unwrap(),
            conf_settings: Some(OrderConfirmationsSettings {
                base_confs: 1,
                base_nota: true,
                rel_confs: 11,
                rel_nota: false,
            }),
            method: "deprecated".to_string(),
        };

        let maker_match_for_rpc = MakerMatchForRpc {
            request: taker_request,
            reserved: reserve,
            connect: None,
            connected: None,
            last_updated: 1223112311114,
        };

        let mut maker_order_matches = HashMap::new();
        maker_order_matches.insert(
            Uuid::from_str("99e1aaf6-eb5c-4550-a1d3-15bf4dc8727d").unwrap(),
            maker_match_for_rpc,
        );

        let hisorical_order = HistoricalOrder {
            max_base_vol: BigRational::from_f64(775.123).take(),
            min_base_vol: BigRational::from_f64(0.0004).take(),
            price: BigRational::from_f64(0.12).take(),
            updated_at: Some(22222222222),
            conf_settings: None,
        };

        let maker_order_for_rpc = MakerOrderForRpc {
            uuid: Uuid::from_str("99777af6-eb5c-4550-a1d3-15bf4dc8727d").unwrap(),
            base: "AAA".to_string(),
            rel: "BBB".to_string(),
            price: BigDecimal::from_f64(11.22).unwrap(),
            price_rat: BigRational::from_f64(11.22).unwrap(),
            max_base_vol: BigDecimal::from_f64(10000.000003).unwrap(),
            max_base_vol_rat: BigRational::from_f64(10000.000003).unwrap(),
            min_base_vol: BigDecimal::from_f64(0.5).unwrap(),
            min_base_vol_rat: BigRational::from_f64(0.5).unwrap(),
            created_at: 1223112311114,
            updated_at: Some(1223112311114),
            matches: maker_order_matches,
            started_swaps: vec![
                Uuid::from_str("8f4ebdec-4d86-467f-ba8e-94256783eb17").unwrap(),
                Uuid::from_str("1efb18ab-2e0e-4511-9b9d-ea8fb9ec19ef").unwrap(),
            ],
            conf_settings: Some(OrderConfirmationsSettings {
                base_confs: 87,
                base_nota: true,
                rel_confs: 78,
                rel_nota: false,
            }),
            changes_history: Some(vec![hisorical_order]),
            base_orderbook_ticker: Some("CCC".to_string()),
            rel_orderbook_ticker: Some("DDD".to_string()),
        };

        let maker_order_for_rpc = MakerOrderForMyOrdersRpc {
            order: maker_order_for_rpc,
            cancellable: true,
            available_amount: BigDecimal::from_f64(18828.12333).unwrap(),
        };
        assert_eq!(
            MAKER_WITH_MATCHES_OUT,
            format_maker_orders_table(&HashMap::from([(
                Uuid::from_str("1e94c6ca-a766-4c4f-b819-858ff1e4f107").unwrap(),
                maker_order_for_rpc
            )]))
            .unwrap()
        );
    }

    const MAKER_WITH_MATCHES_OUT: &str = "        Maker orders: 
│ base,rel      │ price         │ uuid                                 │ created at,        │ min base vol, │ cancellable   │ available     │ swaps                                 │ conf_settings    │ history changes               │
│               │               │                                      │ updated at         │ max base vol  │               │ amount        │                                       │                  │                               │
│ AAA,BBB       │ 11.22         │ 99777af6-eb5c-4550-a1d3-15bf4dc8727d │ 08-10-04 09:25:11, │ 0.50,         │ true          │ 18828.12      │ 8f4ebdec-4d86-467f-ba8e-94256783eb17, │ 87,true:78,false │ min_base_vol: 0.00040         │
│               │               │                                      │ 08-10-04 09:25:11  │ 10000.00      │               │               │ 1efb18ab-2e0e-4511-9b9d-ea8fb9ec19ef  │                  │ max_base_vol: 775.12          │
│               │               │                                      │                    │               │               │               │                                       │                  │ price: 0.12                   │
│               │               │                                      │                    │               │               │               │                                       │                  │ updated_at: 70-09-15 04:50:22 │
│ matches                                                                                                                                                                                                                              │
│                       uuid: 99e1aaf6-eb5c-4550-a1d3-15bf4dc8727d                                                                                                                                                                     │
│                   req.uuid: d9e1aaf6-eb5c-4550-a1d3-15bf4dc8727c                                                                                                                                                                     │
│             req.(base,rel): DFG(0.0023), GGG(0.11)                                                                                                                                                                                   │
│               req.match_by: d9e1aaf6-eb5c-4550-a1d3-15bf4dc8727d, d9e1aaf6-eb5c-4550-a1d3-15bf4dc8727e                                                                                                                               │
│                 req.action: Sell                                                                                                                                                                                                     │
│          req.conf_settings: 1,true:11,false                                                                                                                                                                                          │
│         req.(sender, dest): 15d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732,0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d0447                                                                        │
│        reserved.(base,rel): TTT(888.10), GGG(9921.10)                                                                                                                                                                                │
│    reserved.(taker, maker): a0e1aaf6-eb5c-4550-a1d3-15bf4dc8727d,b1e1aaf6-eb5c-4550-a1d3-15bf4dc8727d                                                                                                                                │
│    reserved.(sender, dest): 022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c8508346,022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c8508348                                                                        │
│     reserved.conf_settings: 1,true:11,false                                                                                                                                                                                          │
│               last_updated: 08-10-04 09:25:11                                                                                                                                                                                        │
│                                                                                                                                                                                                                                      │
";
}

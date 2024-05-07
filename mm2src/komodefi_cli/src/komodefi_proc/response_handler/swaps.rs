use anyhow::{anyhow, Result};
use itertools::Itertools;
use rpc::v1::types::H264;
use std::io::Write;
use term_table::{row::Row, TableStyle};

use common::log::error;
use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};

use super::formatters::{format_bytes, format_datetime_msec, format_ratio, term_table_blank, write_field_option,
                        writeln_field, COMMON_PRECISION, ZERO_INDENT};
use crate::error_anyhow;
use crate::komodefi_proc::response_handler::formatters::format_datetime_sec;
use crate::rpc_data::{ActiveSwapsResponse, MakerNegotiationData, MakerSavedEvent, MakerSavedSwap, MakerSwapData,
                      MakerSwapEvent, MyRecentSwapResponse, MySwapStatusResponse, PaymentInstructions,
                      RecoverFundsOfSwapResponse, SavedSwap, SavedTradeFee, SwapError, TakerNegotiationData,
                      TakerPaymentSpentData, TakerSavedEvent, TakerSavedSwap, TakerSwapData, TakerSwapEvent,
                      TransactionIdentifier};

const DATA_COLUMN_WIDTH: usize = 120;

pub(super) fn on_active_swaps(writer: &mut dyn Write, response: ActiveSwapsResponse, uuids_only: bool) -> Result<()> {
    if uuids_only && response.uuids.is_empty() {
        writeln_safe_io!(writer, "No swaps found");
    } else if uuids_only && !response.uuids.is_empty() {
        writeln_field(writer, "uuids", "", ZERO_INDENT);
        writeln_safe_io!(writer, "{}", response.uuids.iter().join("\n"));
    } else if let Some(statuses) = response.statuses {
        for (_uuid, swap) in statuses {
            writeln_safe_io!(writer, "");
            match swap {
                SavedSwap::Taker(taker_swap) => write_taker_swap(writer, taker_swap)?,
                SavedSwap::Maker(maker_swap) => write_maker_swap(writer, maker_swap)?,
            }
        }
    } else {
        writeln_safe_io!(writer, "No swaps found");
    };
    Ok(())
}

pub(super) fn on_my_swap_status(writer: &mut dyn Write, response: MySwapStatusResponse) -> Result<()> {
    if let Some(my_info) = response.my_info {
        writeln_field(writer, "my_coin", my_info.my_coin, ZERO_INDENT);
        writeln_field(writer, "other_coin", my_info.other_coin, ZERO_INDENT);
        writeln_field(
            writer,
            "my_amount",
            format_ratio(&my_info.my_amount, COMMON_PRECISION)?,
            0,
        );
        writeln_field(
            writer,
            "other_amount",
            format_ratio(&my_info.other_amount, COMMON_PRECISION)?,
            0,
        );
        writeln_field(
            writer,
            "started_at",
            format_datetime_sec(my_info.started_at)?,
            ZERO_INDENT,
        );
    }
    writeln_field(writer, "recoverable", response.recoverable, ZERO_INDENT);
    match response.swap {
        SavedSwap::Taker(taker_swap) => write_taker_swap(writer, taker_swap)?,
        SavedSwap::Maker(maker_swap) => write_maker_swap(writer, maker_swap)?,
    }
    Ok(())
}

pub(super) fn on_my_recent_swaps(writer: &mut dyn Write, response: MyRecentSwapResponse) -> Result<()> {
    write_field_option(writer, "from_uuid", response.from_uuid, ZERO_INDENT);

    writeln_field(writer, "skipped", response.skipped, ZERO_INDENT);
    writeln_field(writer, "limit", response.limit, ZERO_INDENT);
    writeln_field(writer, "total", response.total, ZERO_INDENT);
    writeln_field(writer, "page_number", response.page_number, ZERO_INDENT);
    writeln_field(writer, "total_pages", response.total_pages, ZERO_INDENT);
    writeln_field(writer, "found_records", response.found_records, ZERO_INDENT);

    for swap in response.swaps {
        writeln_safe_io!(writer, "");
        match swap {
            SavedSwap::Taker(taker_swap) => write_taker_swap(writer, taker_swap)?,
            SavedSwap::Maker(maker_swap) => write_maker_swap(writer, maker_swap)?,
        }
    }

    Ok(())
}

pub(super) fn on_recover_funds(writer: &mut dyn Write, response: RecoverFundsOfSwapResponse) -> Result<()> {
    writeln_field(writer, "action", response.action, ZERO_INDENT);
    writeln_field(writer, "coin", response.coin, ZERO_INDENT);
    writeln_field(writer, "tx_hash", format_bytes(response.tx_hash), ZERO_INDENT);
    writeln_field(writer, "tx_hash", format_bytes(response.tx_hex), ZERO_INDENT);
    Ok(())
}

fn write_taker_swap(writer: &mut dyn Write, taker_swap: TakerSavedSwap) -> Result<()> {
    writeln_field(writer, "TakerSwap", taker_swap.uuid, ZERO_INDENT);
    write_field_option(writer, "my_order_uuid", taker_swap.my_order_uuid, ZERO_INDENT);
    write_field_option(writer, "gui", taker_swap.gui, ZERO_INDENT);
    write_field_option(writer, "mm_version", taker_swap.mm_version, ZERO_INDENT);
    write_field_option(writer, "taker_coin", taker_swap.taker_coin, ZERO_INDENT);

    let taker_amount = taker_swap
        .taker_amount
        .map(|value| format_ratio(&value, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "taker_amount", taker_amount, ZERO_INDENT);
    let taker_coin_usd_price = taker_swap
        .taker_coin_usd_price
        .map(|value| format_ratio(&value, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "taker_coin_usd_price", taker_coin_usd_price, ZERO_INDENT);
    write_field_option(writer, "maker_coin", taker_swap.maker_coin, ZERO_INDENT);
    let maker_amount = taker_swap
        .maker_amount
        .map(|value| format_ratio(&value, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "maker_amount", maker_amount, ZERO_INDENT);
    let maker_coin_usd_price = taker_swap
        .maker_coin_usd_price
        .map(|value| format_ratio(&value, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "maker_coin_usd_price", maker_coin_usd_price, ZERO_INDENT);
    write_taker_swap_events(writer, taker_swap.events)
}

fn write_taker_swap_events(writer: &mut dyn Write, taker_swap_events: Vec<TakerSavedEvent>) -> Result<()> {
    let mut term_table = term_table_blank(TableStyle::thin(), false, false, false);
    term_table.set_max_width_for_column(1, DATA_COLUMN_WIDTH);
    if taker_swap_events.is_empty() {
        writeln_field(writer, "events", "empty", ZERO_INDENT);
        return Ok(());
    }
    for event in taker_swap_events {
        let row = match event.event {
            TakerSwapEvent::Started(taker_swap_data) => taker_swap_started_row(event.timestamp, taker_swap_data)?,
            TakerSwapEvent::StartFailed(error) => swap_error_row("StartFailed", event.timestamp, error)?,
            TakerSwapEvent::Negotiated(maker_neg_data) => maker_negotiated_data_row(event.timestamp, maker_neg_data)?,
            TakerSwapEvent::NegotiateFailed(error) => swap_error_row("NegotiateFailed", event.timestamp, error)?,
            TakerSwapEvent::TakerFeeSent(tx_id) => tx_id_row("TakerFeeSent", event.timestamp, tx_id)?,
            TakerSwapEvent::TakerFeeSendFailed(error) => swap_error_row("TakerFeeSendFailed", event.timestamp, error)?,
            TakerSwapEvent::TakerPaymentInstructionsReceived(payment_instrs) => get_opt_value_row(
                "TakerPaymentInstructionsReceived",
                event.timestamp,
                payment_instrs,
                payment_instructions_row,
            )?,
            TakerSwapEvent::MakerPaymentReceived(tx_id) => tx_id_row("MakerPaymentReceived", event.timestamp, tx_id)?,
            TakerSwapEvent::MakerPaymentWaitConfirmStarted => {
                named_event_row("MakerPaymentWaitConfirmStarted", event.timestamp)?
            },
            TakerSwapEvent::MakerPaymentValidatedAndConfirmed => {
                named_event_row("MakerPaymentValidatedAndConfirmed", event.timestamp)?
            },
            TakerSwapEvent::MakerPaymentValidateFailed(error) => {
                swap_error_row("MakerPaymentValidateFailed", event.timestamp, error)?
            },
            TakerSwapEvent::MakerPaymentWaitConfirmFailed(error) => {
                swap_error_row("MakerPaymentWaitConfirmFailed", event.timestamp, error)?
            },
            TakerSwapEvent::TakerPaymentSent(tx_id) => tx_id_row("TakerPaymentSent", event.timestamp, tx_id)?,
            TakerSwapEvent::WatcherMessageSent(maker_spend_preimage, taker_refund_preimage) => {
                watcher_message_row(event.timestamp, maker_spend_preimage, taker_refund_preimage)?
            },
            TakerSwapEvent::TakerPaymentTransactionFailed(error) => {
                swap_error_row("TakerPaymentTransactionFailed", event.timestamp, error)?
            },
            TakerSwapEvent::TakerPaymentDataSendFailed(error) => {
                swap_error_row("TakerPaymentDataSendFailed", event.timestamp, error)?
            },
            TakerSwapEvent::TakerPaymentWaitConfirmFailed(error) => {
                swap_error_row("TakerPaymentWaitConfirmFailed", event.timestamp, error)?
            },
            TakerSwapEvent::TakerPaymentSpent(taker_spent_data) => {
                taker_spent_data_row(event.timestamp, taker_spent_data)?
            },
            TakerSwapEvent::TakerPaymentWaitForSpendFailed(error) => {
                swap_error_row("TakerPaymentWaitForSpendFailed", event.timestamp, error)?
            },
            TakerSwapEvent::MakerPaymentSpent(tx_id) => tx_id_row("MakerPaymentSpent", event.timestamp, tx_id)?,
            TakerSwapEvent::MakerPaymentSpendFailed(error) => {
                swap_error_row("MakerPaymentSpendFailed", event.timestamp, error)?
            },
            TakerSwapEvent::TakerPaymentWaitRefundStarted { wait_until } => {
                wait_refund_row("TakerPaymentWaitRefundStarted", event.timestamp, wait_until)?
            },
            TakerSwapEvent::TakerPaymentRefundStarted => named_event_row("TakerPaymentRefundStarted", event.timestamp)?,
            TakerSwapEvent::TakerPaymentRefunded(opt_tx_id) => {
                get_opt_value_row("TakerPaymentRefunded", event.timestamp, opt_tx_id, tx_id_row)?
            },
            TakerSwapEvent::TakerPaymentRefundFailed(error) => {
                swap_error_row("TakerPaymentRefundFailed", event.timestamp, error)?
            },
            TakerSwapEvent::TakerPaymentRefundFinished => {
                named_event_row("TakerPaymentRefundFinished", event.timestamp)?
            },
            TakerSwapEvent::Finished => named_event_row("Finished", event.timestamp)?,
        };
        term_table.add_row(row);
    }
    writeln_field(writer, "events", "", ZERO_INDENT);
    write_safe_io!(writer, "{}", term_table.render().replace('\0', ""));
    Ok(())
}

fn taker_swap_started_row(timestamp: u64, swap_data: TakerSwapData) -> Result<Row<'static>> {
    let caption = format!("Started\n{}\n", format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    writeln_field(writer, "uuid", swap_data.uuid, ZERO_INDENT);
    writeln_field(
        writer,
        "started_at",
        format_datetime_sec(swap_data.started_at)?,
        ZERO_INDENT,
    );
    writeln_field(writer, "taker_coin", swap_data.taker_coin, ZERO_INDENT);
    writeln_field(writer, "maker_coin", swap_data.maker_coin, ZERO_INDENT);
    writeln_field(writer, "maker", hex::encode(swap_data.maker.0), ZERO_INDENT);
    writeln_field(
        writer,
        "my_persistent_pub",
        hex::encode(swap_data.my_persistent_pub.0),
        0,
    );
    writeln_field(writer, "lock_duration", swap_data.lock_duration, ZERO_INDENT);
    let maker_amount = format_ratio(&swap_data.maker_amount, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string());
    writeln_field(writer, "maker_amount", maker_amount, ZERO_INDENT);
    let taker_amount = format_ratio(&swap_data.taker_amount, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string());
    writeln_field(writer, "taker_amount", taker_amount, ZERO_INDENT);

    writeln_field(
        writer,
        "maker_payment_confirmations",
        swap_data.maker_payment_confirmations,
        0,
    );
    write_field_option(
        writer,
        "maker_payment_requires_nota",
        swap_data.maker_payment_requires_nota,
        0,
    );
    writeln_field(
        writer,
        "taker_payment_confirmations",
        swap_data.taker_payment_confirmations,
        0,
    );
    write_field_option(
        writer,
        "taker_payment_requires_nota",
        swap_data.taker_payment_requires_nota,
        0,
    );
    writeln_field(
        writer,
        "tacker_payment_lock",
        format_datetime_sec(swap_data.taker_payment_lock)?,
        0,
    );
    writeln_field(
        writer,
        "maker_payment_wait",
        format_datetime_sec(swap_data.maker_payment_wait)?,
        0,
    );
    writeln_field(
        writer,
        "maker_coin_start_block",
        swap_data.maker_coin_start_block,
        ZERO_INDENT,
    );
    writeln_field(
        writer,
        "taker_coin_start_block",
        swap_data.taker_coin_start_block,
        ZERO_INDENT,
    );
    let fee_to_send_taker_fee = swap_data
        .fee_to_send_taker_fee
        .map(|value| format_saved_trade_fee(value).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "fee_to_send_taker_fee", fee_to_send_taker_fee, ZERO_INDENT);
    let taker_payment_trade_fee = swap_data
        .taker_payment_trade_fee
        .map(|value| format_saved_trade_fee(value).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "taker_payment_trade_fee", taker_payment_trade_fee, ZERO_INDENT);
    let maker_spend_trade_fee = swap_data
        .maker_payment_spend_trade_fee
        .map(|value| format_saved_trade_fee(value).unwrap_or_else(|_| "error".to_string()));
    write_field_option(
        writer,
        "maker_payment_spend_trade_fee",
        maker_spend_trade_fee,
        ZERO_INDENT,
    );
    let maker_contract = swap_data
        .maker_coin_swap_contract_address
        .map(|v| hex::encode(v.as_slice()));
    write_field_option(writer, "maker_coin_swap_contract", maker_contract, ZERO_INDENT);
    let taker_contract = swap_data
        .taker_coin_swap_contract_address
        .map(|v| hex::encode(v.as_slice()));
    write_field_option(writer, "taker_coin_swap_contract", taker_contract, ZERO_INDENT);
    let maker_coin_htlc_pubkey = swap_data.maker_coin_htlc_pubkey.map(|v| hex::encode(v.0));
    write_field_option(writer, "maker_coin_htlc_pubkey", maker_coin_htlc_pubkey, ZERO_INDENT);
    let taker_coin_htlc_pubkey = swap_data.taker_coin_htlc_pubkey.map(|v| hex::encode(v.0));
    write_field_option(writer, "taker_coin_htlc_pubkey", taker_coin_htlc_pubkey, ZERO_INDENT);
    let p2p_pkey = swap_data.p2p_privkey.map(|v| v.inner);
    write_field_option(writer, "p2p_privkey", p2p_pkey, ZERO_INDENT);

    let data =
        String::from_utf8(buff).map_err(|error| error_anyhow!("Failed to get taker_swap_data from buffer: {error}"))?;

    Ok(Row::new([caption, data]))
}

fn swap_error_row(caption: &str, timestamp: u64, swap_error: SwapError) -> Result<Row<'static>> {
    let caption = format!("{}\n{}\n", caption, format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    writeln_field(writer, "swap_error", swap_error.error, ZERO_INDENT);
    let data =
        String::from_utf8(buff).map_err(|error| error_anyhow!("Failed to get swap_error from buffer: {error}"))?;
    Ok(Row::new([caption, data]))
}

fn maker_negotiated_data_row(timestamp: u64, neg_data: MakerNegotiationData) -> Result<Row<'static>> {
    let caption = format!("Negotiated\n{}\n", format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    writeln_field(
        writer,
        "maker_payment_locktime",
        format_datetime_sec(neg_data.maker_payment_locktime)?,
        0,
    );
    writeln_field(writer, "maker_pubkey", format_h264(neg_data.maker_pubkey), ZERO_INDENT);
    writeln_field(writer, "secret_hash", format_bytes(neg_data.secret_hash), ZERO_INDENT);
    write_field_option(
        writer,
        "maker_swap_contract",
        neg_data.maker_coin_swap_contract_addr.map(format_bytes),
        0,
    );
    write_field_option(
        writer,
        "taker_swap_contract",
        neg_data.taker_coin_swap_contract_addr.map(format_bytes),
        0,
    );
    write_field_option(
        writer,
        "maker_coin_htlc_pubkey",
        neg_data.maker_coin_htlc_pubkey.map(format_h264),
        0,
    );
    write_field_option(
        writer,
        "taker_coin_htlc_pubkey",
        neg_data.taker_coin_htlc_pubkey.map(format_h264),
        0,
    );

    let data = String::from_utf8(buff)
        .map_err(|error| error_anyhow!("Failed to get  maker_negotiated_data from buffer: {error}"))?;
    Ok(Row::new([caption, data]))
}

fn tx_id_row(caption: &str, timestamp: u64, tx_id: TransactionIdentifier) -> Result<Row<'static>> {
    let caption = format!("{}\n{}\n", caption, format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    writeln_field(writer, "tx_hex", format_bytes(tx_id.tx_hex), ZERO_INDENT);
    writeln_field(writer, "tx_hash", format_bytes(tx_id.tx_hash), ZERO_INDENT);
    let data = String::from_utf8(buff)
        .map_err(|error| error_anyhow!("Failed to get transaction_identifier from buffer: {error}"))?;
    Ok(Row::new([caption, data]))
}

fn payment_instructions_row(
    caption: &str,
    timestamp: u64,
    payment_instrs: PaymentInstructions,
) -> Result<Row<'static>> {
    let caption = format!("{}\n{}\n", caption, format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    match payment_instrs {
        PaymentInstructions::Lightning(invoice) => {
            writeln_field(writer, "Lightning: {}", invoice.to_string(), ZERO_INDENT)
        },
        PaymentInstructions::WatcherReward(reward) => writeln_field(
            writer,
            "WatcherReward: {}",
            format_ratio(&reward, COMMON_PRECISION)?,
            ZERO_INDENT,
        ),
    }
    let data = String::from_utf8(buff)
        .map_err(|error| error_anyhow!("Failed to get payment_instructions from buffer: {error}"))?;
    Ok(Row::new([caption, data]))
}

fn named_event_row(caption: &str, timestamp: u64) -> Result<Row<'static>> {
    let caption = format!("{}\n{}\n", caption, format_datetime_msec(timestamp)?);
    Ok(Row::new([caption, "".to_string()]))
}

fn watcher_message_row(
    timestamp: u64,
    maker_spend_preimage: Option<Vec<u8>>,
    taker_refund_preimage: Option<Vec<u8>>,
) -> Result<Row<'static>> {
    let caption = format!("WatcherMessageSent\n{}\n", format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    write_field_option(
        writer,
        "maker_spend_preimage",
        maker_spend_preimage.map(hex::encode),
        ZERO_INDENT,
    );
    write_field_option(
        writer,
        "taker_refund_preimage",
        taker_refund_preimage.map(hex::encode),
        0,
    );
    let data =
        String::from_utf8(buff).map_err(|error| error_anyhow!("Failed to get watcher_message from buffer: {error}"))?;
    Ok(Row::new([caption, data]))
}

fn taker_spent_data_row(timestamp: u64, taker_spent_data: TakerPaymentSpentData) -> Result<Row<'static>> {
    let caption = format!("TakerPaymentSpent\n{}\n", format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    writeln_field(
        writer,
        "tx_hex",
        format_bytes(taker_spent_data.transaction.tx_hex),
        ZERO_INDENT,
    );
    writeln_field(
        writer,
        "tx_hash",
        format_bytes(taker_spent_data.transaction.tx_hash),
        ZERO_INDENT,
    );
    writeln_field(writer, "secret", hex::encode(taker_spent_data.secret.0), ZERO_INDENT);
    let data = String::from_utf8(buff)
        .map_err(|error| error_anyhow!("Failed to get taker_spent_data from buffer: {error}"))?;
    Ok(Row::new([caption, data]))
}

fn wait_refund_row(caption: &str, timestamp: u64, wait_until: u64) -> Result<Row<'static>> {
    let caption = format!("{}\n{}\n", caption, format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    writeln_field(writer, "wait_until", format_datetime_msec(wait_until)?, ZERO_INDENT);
    let data = String::from_utf8(buff)
        .map_err(|error| error_anyhow!("Failed to get taker_spent_data from buffer: {error}"))?;
    Ok(Row::new([caption, data]))
}

fn format_saved_trade_fee(trade_fee: SavedTradeFee) -> Result<String> {
    let saved_trade_fee = format!(
        "coin: {}, amount: {}, paid_from_trading_vol: {}",
        trade_fee.coin,
        format_ratio(&trade_fee.amount, COMMON_PRECISION)?,
        trade_fee.paid_from_trading_vol
    );
    Ok(saved_trade_fee)
}

fn write_maker_swap(writer: &mut dyn Write, maker_swap: MakerSavedSwap) -> Result<()> {
    writeln_field(writer, "MakerSwap", maker_swap.uuid, ZERO_INDENT);
    write_field_option(writer, "my_order_uuid", maker_swap.my_order_uuid, ZERO_INDENT);
    write_field_option(writer, "gui", maker_swap.gui, ZERO_INDENT);
    write_field_option(writer, "mm_version", maker_swap.mm_version, ZERO_INDENT);
    write_field_option(writer, "taker_coin", maker_swap.taker_coin, ZERO_INDENT);

    let taker_amount = maker_swap
        .taker_amount
        .map(|value| format_ratio(&value, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "taker_amount", taker_amount, ZERO_INDENT);
    let taker_coin_usd_price = maker_swap
        .taker_coin_usd_price
        .map(|value| format_ratio(&value, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "taker_coin_usd_price", taker_coin_usd_price, ZERO_INDENT);

    write_field_option(writer, "maker_coin", maker_swap.maker_coin, ZERO_INDENT);
    let maker_amount = maker_swap
        .maker_amount
        .map(|value| format_ratio(&value, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "maker_amount", maker_amount, ZERO_INDENT);
    let maker_coin_usd_price = maker_swap
        .maker_coin_usd_price
        .map(|value| format_ratio(&value, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string()));
    write_field_option(writer, "maker_coin_usd_price", maker_coin_usd_price, ZERO_INDENT);
    write_maker_swap_events(writer, maker_swap.events)
}

fn write_maker_swap_events(writer: &mut dyn Write, maker_swap_event: Vec<MakerSavedEvent>) -> Result<()> {
    let mut term_table = term_table_blank(TableStyle::thin(), false, false, false);
    term_table.set_max_width_for_column(1, DATA_COLUMN_WIDTH);
    if maker_swap_event.is_empty() {
        writeln_field(writer, "events", "empty", ZERO_INDENT);
        return Ok(());
    }
    for event in maker_swap_event {
        let row = match event.event {
            MakerSwapEvent::Started(maker_swap_data) => maker_swap_started_row(event.timestamp, maker_swap_data)?,
            MakerSwapEvent::StartFailed(error) => swap_error_row("StartFailed", event.timestamp, error)?,
            MakerSwapEvent::Negotiated(taker_neg_data) => taker_negotiated_data_row(event.timestamp, taker_neg_data)?,
            MakerSwapEvent::NegotiateFailed(error) => swap_error_row("NegotiateFailed", event.timestamp, error)?,
            MakerSwapEvent::MakerPaymentInstructionsReceived(opt_payment_instr) => get_opt_value_row(
                "MakerPaymentInstructionsReceived",
                event.timestamp,
                opt_payment_instr,
                payment_instructions_row,
            )?,
            MakerSwapEvent::TakerFeeValidated(tx_id) => tx_id_row("TakerFeeValidated", event.timestamp, tx_id)?,
            MakerSwapEvent::TakerFeeValidateFailed(error) => {
                swap_error_row("TakerFeeValidateFailed", event.timestamp, error)?
            },
            MakerSwapEvent::MakerPaymentSent(tx_id) => tx_id_row("MakerPaymentSent", event.timestamp, tx_id)?,
            MakerSwapEvent::MakerPaymentTransactionFailed(error) => {
                swap_error_row("MakerPaymentTransactionFailed", event.timestamp, error)?
            },
            MakerSwapEvent::MakerPaymentDataSendFailed(error) => {
                swap_error_row("MakerPaymentDataSendFailed", event.timestamp, error)?
            },
            MakerSwapEvent::MakerPaymentWaitConfirmFailed(error) => {
                swap_error_row("MakerPaymentWaitConfirmFailed", event.timestamp, error)?
            },
            MakerSwapEvent::TakerPaymentReceived(tx_id) => tx_id_row("TakerPaymentReceived", event.timestamp, tx_id)?,
            MakerSwapEvent::TakerPaymentWaitConfirmStarted => {
                named_event_row("TakerPaymentWaitConfirmStarted", event.timestamp)?
            },
            MakerSwapEvent::TakerPaymentValidatedAndConfirmed => {
                named_event_row("TakerPaymentValidatedAndConfirmed", event.timestamp)?
            },
            MakerSwapEvent::TakerPaymentValidateFailed(error) => {
                swap_error_row("TakerPaymentValidateFailed", event.timestamp, error)?
            },
            MakerSwapEvent::TakerPaymentWaitConfirmFailed(error) => {
                swap_error_row("TakerPaymentWaitConfirmFailed", event.timestamp, error)?
            },
            MakerSwapEvent::TakerPaymentSpent(tx_id) => tx_id_row("TakerPaymentSpent", event.timestamp, tx_id)?,
            MakerSwapEvent::TakerPaymentSpendFailed(error) => {
                swap_error_row("TakerPaymentSpendFailed", event.timestamp, error)?
            },
            MakerSwapEvent::TakerPaymentSpendConfirmStarted => {
                named_event_row("TakerPaymentSpendConfirmStarted", event.timestamp)?
            },
            MakerSwapEvent::TakerPaymentSpendConfirmed => {
                named_event_row("TakerPaymentSpendConfirmed", event.timestamp)?
            },
            MakerSwapEvent::TakerPaymentSpendConfirmFailed(error) => {
                swap_error_row("TakerPaymentSpendConfirmFailed", event.timestamp, error)?
            },
            MakerSwapEvent::MakerPaymentWaitRefundStarted { wait_until } => {
                wait_refund_row("MakerPaymentWaitRefundStarted", event.timestamp, wait_until)?
            },
            MakerSwapEvent::MakerPaymentRefundStarted => named_event_row("MakerPaymentRefundStarted", event.timestamp)?,
            MakerSwapEvent::MakerPaymentRefunded(opt_tx_id) => {
                get_opt_value_row("MakerPaymentRefunded", event.timestamp, opt_tx_id, tx_id_row)?
            },
            MakerSwapEvent::MakerPaymentRefundFailed(error) => {
                swap_error_row("MakerPaymentRefundFailed", event.timestamp, error)?
            },
            MakerSwapEvent::MakerPaymentRefundFinished => {
                named_event_row("MakerPaymentRefundFinished", event.timestamp)?
            },
            MakerSwapEvent::Finished => named_event_row("Finished", event.timestamp)?,
        };
        term_table.add_row(row);
    }
    writeln_field(writer, "events", "", ZERO_INDENT);
    write_safe_io!(writer, "{}", term_table.render());
    Ok(())
}

fn maker_swap_started_row(timestamp: u64, swap_data: MakerSwapData) -> Result<Row<'static>> {
    let caption = format!("Started\n{}\n", format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    writeln_field(writer, "uuid", swap_data.uuid, ZERO_INDENT);
    writeln_field(
        writer,
        "started_at",
        format_datetime_msec(swap_data.started_at)?,
        ZERO_INDENT,
    );
    writeln_field(writer, "taker_coin", swap_data.taker_coin, ZERO_INDENT);
    writeln_field(writer, "maker_coin", swap_data.maker_coin, ZERO_INDENT);
    writeln_field(writer, "taker", hex::encode(swap_data.taker.0), ZERO_INDENT);
    writeln_field(writer, "secret", hex::encode(swap_data.secret.0), ZERO_INDENT);
    write_field_option(
        writer,
        "secret_hash",
        swap_data.secret_hash.map(format_bytes),
        ZERO_INDENT,
    );

    writeln_field(
        writer,
        "my_persistent_pub",
        hex::encode(swap_data.my_persistent_pub.0),
        0,
    );
    writeln_field(writer, "lock_duration", swap_data.lock_duration, ZERO_INDENT);
    let maker_amount = format_ratio(&swap_data.maker_amount, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string());
    writeln_field(writer, "maker_amount", maker_amount, ZERO_INDENT);
    let taker_amount = format_ratio(&swap_data.taker_amount, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string());
    writeln_field(writer, "taker_amount", taker_amount, ZERO_INDENT);

    writeln_field(
        writer,
        "maker_payment_confirmations",
        swap_data.maker_payment_confirmations,
        0,
    );
    write_field_option(
        writer,
        "maker_payment_requires_nota",
        swap_data.maker_payment_requires_nota,
        0,
    );
    writeln_field(
        writer,
        "taker_payment_confirmations",
        swap_data.taker_payment_confirmations,
        0,
    );
    write_field_option(
        writer,
        "taker_payment_requires_nota",
        swap_data.taker_payment_requires_nota,
        0,
    );
    writeln_field(
        writer,
        "macker_payment_lock",
        format_datetime_msec(swap_data.maker_payment_lock)?,
        0,
    );

    writeln_field(
        writer,
        "maker_coin_start_block",
        swap_data.maker_coin_start_block,
        ZERO_INDENT,
    );
    writeln_field(
        writer,
        "taker_coin_start_block",
        swap_data.taker_coin_start_block,
        ZERO_INDENT,
    );
    write_field_option(
        writer,
        "maker_payment_trade_fee",
        swap_data
            .maker_payment_trade_fee
            .map(|v| format_saved_trade_fee(v).unwrap_or_else(|_| "error".to_string())),
        0,
    );
    write_field_option(
        writer,
        "taker_payment_spend_trade_fee",
        swap_data
            .taker_payment_spend_trade_fee
            .map(|v| format_saved_trade_fee(v).unwrap_or_else(|_| "error".to_string())),
        0,
    );
    let maker_contract = swap_data
        .maker_coin_swap_contract_address
        .map(|v| hex::encode(v.as_slice()));
    write_field_option(writer, "maker_coin_swap_contract", maker_contract, ZERO_INDENT);
    let taker_contract = swap_data
        .taker_coin_swap_contract_address
        .map(|v| hex::encode(v.as_slice()));
    write_field_option(writer, "taker_coin_swap_contract", taker_contract, ZERO_INDENT);
    let maker_coin_htlc_pubkey = swap_data.maker_coin_htlc_pubkey.map(|v| hex::encode(v.0));
    write_field_option(writer, "maker_coin_htlc_pubkey", maker_coin_htlc_pubkey, ZERO_INDENT);
    let taker_coin_htlc_pubkey = swap_data.taker_coin_htlc_pubkey.map(|v| hex::encode(v.0));
    write_field_option(writer, "taker_coin_htlc_pubkey", taker_coin_htlc_pubkey, ZERO_INDENT);
    let p2p_pkey = swap_data.p2p_privkey.map(|v| v.inner);
    write_field_option(writer, "p2p_privkey", p2p_pkey, ZERO_INDENT);
    let data =
        String::from_utf8(buff).map_err(|error| error_anyhow!("Failed to get maker_swap_data from buffer: {error}"))?;
    Ok(Row::new([caption, data]))
}

fn taker_negotiated_data_row(timestamp: u64, neg_data: TakerNegotiationData) -> Result<Row<'static>> {
    let caption = format!("Negotiated\n{}\n", format_datetime_msec(timestamp)?);
    let mut buff = vec![];
    let writer: &mut dyn Write = &mut buff;
    writeln_field(
        writer,
        "taker_payment_locktime",
        format_datetime_msec(neg_data.taker_payment_locktime)?,
        0,
    );
    writeln_field(writer, "taker_pubkey", format_h264(neg_data.taker_pubkey), ZERO_INDENT);
    write_field_option(
        writer,
        "maker_swap_contract",
        neg_data.maker_coin_swap_contract_addr.map(format_bytes),
        0,
    );
    write_field_option(
        writer,
        "taker_swap_contract",
        neg_data.taker_coin_swap_contract_addr.map(format_bytes),
        0,
    );
    write_field_option(
        writer,
        "maker_coin_htlc_pubkey",
        neg_data.maker_coin_htlc_pubkey.map(format_h264),
        0,
    );
    write_field_option(
        writer,
        "taker_coin_htlc_pubkey",
        neg_data.taker_coin_htlc_pubkey.map(format_h264),
        0,
    );

    let data = String::from_utf8(buff)
        .map_err(|error| error_anyhow!("Failed to get  taker_negotiated_data from buffer: {error}"))?;
    Ok(Row::new([caption, data]))
}

fn get_opt_value_row<T, F: Fn(&str, u64, T) -> Result<Row<'static>>>(
    caption: &str,
    timestamp: u64,
    value: Option<T>,
    delegate: F,
) -> Result<Row<'static>> {
    if let Some(value) = value {
        delegate(caption, timestamp, value)
    } else {
        get_none_value_row(caption, timestamp)
    }
}

fn get_none_value_row(caption: &str, timestamp: u64) -> Result<Row<'static>> {
    let caption = format!("{}\n{}\n", caption, format_datetime_msec(timestamp)?);
    Ok(Row::new([caption, "none".to_string()]))
}

fn format_h264(bytes: H264) -> String { hex::encode(bytes.0) }

use rpc::v1::types::H256 as H256Json;
use std::io::Write;
use term_table::{row::Row, TableStyle};

use crate::komodefi_proc::response_handler::formatters::{format_datetime_msec, term_table_blank, write_sequence,
                                                         writeln_field};
use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};

use super::ZERO_INDENT;
use crate::rpc_data::utility::{GetCurrentMtpError, GetCurrentMtpResponse};
use crate::rpc_data::{BanReason, ListBannedPubkeysResponse, UnbanPubkeysResponse};

pub(super) fn on_list_banned_pubkeys(writer: &mut dyn Write, response: ListBannedPubkeysResponse) {
    if response.is_empty() {
        writeln_field(writer, "banned_pubkeys", "empty", ZERO_INDENT);
        return;
    }

    let mut term_table = term_table_blank(TableStyle::thin(), false, false, false);
    term_table.add_row(Row::new(["pubkey", "reason", "comment"]));
    for (pubkey, ban_reason) in response {
        match ban_reason {
            BanReason::Manual { reason } => {
                term_table.add_row(Row::new([hex::encode(pubkey.0), "manual".to_string(), reason]))
            },
            BanReason::FailedSwap { caused_by_swap } => term_table.add_row(Row::new([
                hex::encode(pubkey.0),
                "swap".to_string(),
                caused_by_swap.to_string(),
            ])),
        }
    }
    writeln_safe_io!(writer, "{}", term_table.render());
}

pub(super) fn on_unban_pubkeys(writer: &mut dyn Write, response: UnbanPubkeysResponse) {
    let still_banned = response.still_banned.iter().map(format_ban_reason);
    write_sequence(writer, "still_banned", still_banned, ZERO_INDENT);
    let unbanned = response.unbanned.iter().map(format_ban_reason);
    write_sequence(writer, "unbanned", unbanned, ZERO_INDENT);
    let were_not_banned = response.were_not_banned.iter();
    write_sequence(writer, "were_not_banned", were_not_banned, ZERO_INDENT);
}

pub(super) fn on_current_mtp(writer: &mut dyn Write, response: GetCurrentMtpResponse) {
    writeln_field(
        writer,
        "Current mtp",
        format_datetime_msec(response.mtp as u64 * 1000).unwrap(),
        ZERO_INDENT,
    );
}

pub(super) fn on_get_current_mtp_error(writer: &mut dyn Write, error: GetCurrentMtpError) {
    writeln_field(writer, "Failed to get current mtp", error, ZERO_INDENT);
}

fn format_ban_reason((pubkey, ban_reason): (&H256Json, &BanReason)) -> String {
    match ban_reason {
        BanReason::Manual { reason } => {
            format!("{}(manually \"{}\")", hex::encode(pubkey.0), reason)
        },
        BanReason::FailedSwap { caused_by_swap } => {
            format!("{}(caused_by_swap {})", hex::encode(pubkey.0), caused_by_swap)
        },
    }
}

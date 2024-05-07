use anyhow::{anyhow, Result};
use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};
use itertools::Itertools;
use std::io::Write;
use term_table::row::Row;
use term_table::TableStyle;

use common::log::error;
use mm2_rpc::data::legacy::Status;

use super::super::activation::format_coin_balance;
use super::super::formatters::{term_table_blank, writeln_field, ZERO_INDENT};
use crate::error_anyhow;
use crate::komodefi_proc::response_handler::formatters::{format_bytes, write_field_option, COMMON_INDENT};
use crate::rpc_data::activation::zcoin::{CheckPointBlockInfo, CoinBalanceReport, CoinProtocol, HDAddressBalance,
                                         HDWalletBalance, IguanaWalletBalance, InitStandaloneCoinError,
                                         RpcDerivationPath, StandardHDPathToCoin, ZCoinStatus, ZcoinActivationResult,
                                         ZcoinAwaitingStatus, ZcoinConsensusParams, ZcoinInProgressStatus};
use crate::rpc_data::activation::{InitRpcTaskResponse, TaskId};
use crate::rpc_data::CancelRpcTaskError;

pub(in super::super) fn on_enable_zcoin(writer: &mut dyn Write, response: InitRpcTaskResponse) -> TaskId {
    writeln_field(writer, "Enabling zcoin started, task_id", response.task_id, ZERO_INDENT);
    response.task_id
}

pub(in super::super) fn on_enable_zcoin_status(writer: &mut dyn Write, response: ZCoinStatus) -> Result<bool> {
    match response {
        ZCoinStatus::Ok(ok_status) => on_enable_zcoin_status_ok(writer, ok_status),
        ZCoinStatus::InProgress(progress_status) => on_enable_zcoin_status_progress(writer, progress_status),
        ZCoinStatus::UserActionRequired(user_action_status) => {
            on_enable_zcoin_status_user_action(writer, user_action_status)
        },
        ZCoinStatus::Error(error_status) => on_enable_zcoin_status_error(writer, error_status),
    }
}

fn on_enable_zcoin_status_ok(writer: &mut dyn Write, response: ZcoinActivationResult) -> Result<bool> {
    writeln_field(writer, "status", "OK", ZERO_INDENT);
    writeln_field(writer, "current_block", response.current_block, ZERO_INDENT);
    writeln_field(writer, "ticker", response.ticker, ZERO_INDENT);
    match response.wallet_balance {
        CoinBalanceReport::Iguana(balance) => write_iguana_balance(writer, balance)?,
        CoinBalanceReport::HD(balance) => write_hd_balance(writer, balance)?,
    }
    Ok(false)
}

fn on_enable_zcoin_status_progress(writer: &mut dyn Write, response: ZcoinInProgressStatus) -> Result<bool> {
    writeln_field(writer, "In progress", response, ZERO_INDENT);
    Ok(true)
}

fn on_enable_zcoin_status_error(writer: &mut dyn Write, response: InitStandaloneCoinError) -> Result<bool> {
    match response {
        InitStandaloneCoinError::UnexpectedCoinProtocol { ticker, protocol } => {
            writeln_field(writer, "ticker", ticker, ZERO_INDENT);
            write_coin_protocol(writer, protocol)
        },
        response => writeln_field(writer, "Error", response, ZERO_INDENT),
    };

    Ok(false)
}

fn on_enable_zcoin_status_user_action(writer: &mut dyn Write, response: ZcoinAwaitingStatus) -> Result<bool> {
    writeln_field(writer, "Awaiting for action", response, ZERO_INDENT);
    Ok(true)
}

pub(in super::super) fn on_enable_zcoin_canceled(writer: &mut dyn Write, response: Status) {
    writeln_safe_io!(writer, "canceled: {}", response);
}

pub(in super::super) fn on_enable_zcoin_cancel_error(writer: &mut dyn Write, error: CancelRpcTaskError) {
    writeln_field(writer, "rpc task error", error, ZERO_INDENT);
}

fn write_coin_protocol(writer: &mut dyn Write, protocol: CoinProtocol) {
    let CoinProtocol::ZHTLC(protocol) = protocol;
    write_consensus_params(writer, protocol.consensus_params);
    if let Some(check_point_block) = protocol.check_point_block {
        write_check_point_block(writer, check_point_block);
    }
    if let Some(z_derivation_path) = protocol.z_derivation_path {
        write_z_deriavation_path(writer, z_derivation_path);
    }
}

fn write_consensus_params(writer: &mut dyn Write, params: ZcoinConsensusParams) {
    writeln_field(writer, "consensus_params", "", ZERO_INDENT);
    writeln_field(
        writer,
        "overwinter_activation_height",
        params.overwinter_activation_height,
        COMMON_INDENT,
    );
    writeln_field(
        writer,
        "sapling_activation_height",
        params.sapling_activation_height,
        COMMON_INDENT,
    );
    write_field_option(
        writer,
        "blossom_activation_height",
        params.blossom_activation_height,
        COMMON_INDENT,
    );
    write_field_option(
        writer,
        "heartwood_activation_height",
        params.heartwood_activation_height,
        COMMON_INDENT,
    );
    write_field_option(
        writer,
        "canopy_activation_height",
        params.canopy_activation_height,
        COMMON_INDENT,
    );
    writeln_field(writer, "coin_type", params.coin_type, COMMON_INDENT);
    writeln_field(
        writer,
        "hrp_sapling_extended_spending_key",
        params.hrp_sapling_extended_spending_key,
        COMMON_INDENT,
    );
    writeln_field(
        writer,
        "hrp_sapling_extended_full_viewing_key",
        params.hrp_sapling_extended_full_viewing_key,
        COMMON_INDENT,
    );
    writeln_field(
        writer,
        "hrp_sapling_payment_address",
        params.hrp_sapling_payment_address,
        COMMON_INDENT,
    );
    writeln_field(
        writer,
        "b58_pubkey_address_prefix",
        hex::encode(params.b58_pubkey_address_prefix),
        COMMON_INDENT,
    );
    writeln_field(
        writer,
        "b58_script_address_prefix",
        hex::encode(params.b58_script_address_prefix),
        COMMON_INDENT,
    );
}

fn write_check_point_block(writer: &mut dyn Write, check_point_block: CheckPointBlockInfo) {
    writeln_field(writer, "check_point_block", "", ZERO_INDENT);
    writeln_field(writer, "height", check_point_block.height, COMMON_INDENT);
    writeln_field(writer, "hash", check_point_block.hash, COMMON_INDENT);
    writeln_field(writer, "timestamp", check_point_block.time, COMMON_INDENT);
    writeln_field(
        writer,
        "sapling_tree",
        format_bytes(check_point_block.sapling_tree),
        COMMON_INDENT,
    );
}

fn write_z_deriavation_path(writer: &mut dyn Write, z_derivation_path: StandardHDPathToCoin) {
    writeln_field(writer, "z_derivation_path", "", ZERO_INDENT);

    format!(
        "Bip32Child: {{Value: {},  Child: {{ Value: {} , Child: {} }} }}",
        z_derivation_path.value.purpose, z_derivation_path.child.value.number, z_derivation_path.child.child
    );
}

fn write_iguana_balance(writer: &mut dyn Write, balance: IguanaWalletBalance) -> Result<()> {
    writeln_field(writer, "iguana wallet", "", ZERO_INDENT);
    writeln_field(writer, "address", balance.address, COMMON_INDENT);
    writeln_field(writer, "balance", format_coin_balance(balance.balance)?, COMMON_INDENT);
    Ok(())
}

fn write_hd_balance(writer: &mut dyn Write, balance: HDWalletBalance) -> Result<()> {
    writeln_field(writer, "hd wallet", "", ZERO_INDENT);
    if balance.accounts.is_empty() {
        writeln_field(writer, "accounts", "none", COMMON_INDENT);
    } else {
        writeln_field(writer, "accounts", "", COMMON_INDENT);
        let mut term_table = term_table_blank(TableStyle::thin(), false, false, false);
        term_table.add_row(Row::new(["index", "derivation", "balance(spend:unspend)", "adresses"]));

        for account in balance.accounts {
            term_table.add_row(Row::new([
                account.account_index.to_string(),
                format_derivation_path(account.derivation_path),
                format_coin_balance(account.total_balance)?,
                format_hd_addresses(account.addresses)?,
            ]))
        }
        writeln_safe_io!(writer, "{}", term_table.render());
    };
    Ok(())
}

fn format_hd_addresses(addresses: Vec<HDAddressBalance>) -> Result<String> {
    let mut buff: Vec<u8> = vec![];
    let writer: &mut dyn Write = &mut buff;
    for address in addresses {
        writeln_field(writer, "address", address.address, ZERO_INDENT);
        writeln_field(writer, "balance", format_coin_balance(address.balance)?, ZERO_INDENT);
        writeln_field(
            writer,
            "der path",
            format_derivation_path(address.derivation_path),
            ZERO_INDENT,
        );
        writeln_field(writer, "chain", address.chain.to_string(), ZERO_INDENT);
        writeln_field(writer, "", "", ZERO_INDENT);
    }
    String::from_utf8(buff).map_err(|error| error_anyhow!("Failed to format hd_address: {error}"))
}

fn format_derivation_path(derivation_path: RpcDerivationPath) -> String {
    derivation_path.0.path.iter().map(|v| v.0).join(",")
}

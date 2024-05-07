use std::io::Write;

use mm2_rpc::data::legacy::Status;

use super::formatters::{writeln_field, ZERO_INDENT};
use crate::rpc_data::version_stat::NodeVersionError;

pub(super) fn on_vstat_add_node(writer: &mut dyn Write, response: Status) {
    writeln_field(writer, "Add node to version stat", response, ZERO_INDENT);
}

pub(super) fn on_vstat_rem_node(writer: &mut dyn Write, response: Status) {
    writeln_field(writer, "Remove node from version stat", response, ZERO_INDENT);
}

pub(super) fn on_node_version_error(writer: &mut dyn Write, error: NodeVersionError) {
    writeln_field(writer, "Failed to add node", error, ZERO_INDENT);
}

pub(super) fn on_vstat_start_collection(writer: &mut dyn Write, response: Status) {
    writeln_field(writer, "Start stat collection", response, ZERO_INDENT);
}

pub(super) fn on_vstat_stop_collection(writer: &mut dyn Write, response: Status) {
    writeln_field(writer, "Stop stat collection", response, ZERO_INDENT);
}

pub(super) fn on_vstat_update_collection(writer: &mut dyn Write, response: Status) {
    writeln_field(writer, "Update stat collection", response, ZERO_INDENT);
}

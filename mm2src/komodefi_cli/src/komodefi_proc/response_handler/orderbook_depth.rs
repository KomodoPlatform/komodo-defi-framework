use std::io::Write;
use term_table::{row::Row,
                 table_cell::{Alignment, TableCell},
                 TableStyle};

use common::{write_safe::io::WriteSafeIO, write_safe_io};
use mm2_rpc::data::legacy::{Mm2RpcResult, PairWithDepth};

use super::formatters::term_table_blank;

pub(super) fn on_orderbook_depth(
    writer: &mut dyn Write,
    response: Mm2RpcResult<Vec<PairWithDepth>>,
) -> anyhow::Result<()> {
    let mut term_table = term_table_blank(TableStyle::empty(), false, false, false);
    term_table.add_row(orderbook_depth_header_row());
    for data in response.result {
        term_table.add_row(orderbook_depth_row(data))
    }
    write_safe_io!(writer, "{}", term_table.render().replace('\0', ""));
    Ok(())
}

fn orderbook_depth_header_row() -> Row<'static> {
    Row::new(vec![
        TableCell::new(""),
        TableCell::new_with_alignment_and_padding("Bids", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Asks", 1, Alignment::Left, false),
    ])
}

fn orderbook_depth_row(data: PairWithDepth) -> Row<'static> {
    Row::new(vec![
        TableCell::new_with_alignment_and_padding(
            format!("{}/{}:", data.pair.0, data.pair.1),
            1,
            Alignment::Right,
            false,
        ),
        TableCell::new_with_alignment_and_padding(data.depth.bids, 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding(data.depth.asks, 1, Alignment::Left, false),
    ])
}

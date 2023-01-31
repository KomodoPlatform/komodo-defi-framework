use mm2_db::indexed_db::{DbUpgrader, OnUpgradeResult, TableSignature};

pub const HEIGHT_TICKER_INDEX: &str = "block_height_ticker_index";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BlockHeaderStorageTable {
    pub height: u64,
    pub bits: u32,
    pub hash: String,
    pub raw_header: String,
    pub ticker: String,
}

impl TableSignature for BlockHeaderStorageTable {
    fn table_name() -> &'static str { "block_header_storage_cache_table" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        match (old_version, new_version) {
            (0, 1) => {
                let table = upgrader.create_table(Self::table_name())?;
                table.create_multi_index(HEIGHT_TICKER_INDEX, &["height", "ticker"], false)?;
                table.create_index("height", false)?;
                table.create_index("bits", true)?;
                table.create_index("hash", true)?;
                table.create_index("raw_header", true)?;
                table.create_index("ticker", false)?;
            },
            _ => (),
        }
        Ok(())
    }
}

use async_trait::async_trait;
use mm2_db::indexed_db::{BeBigUint, DbIdentifier, DbInstance, DbUpgrader, IndexedDb, IndexedDbBuilder, InitDbResult,
                         OnUpgradeResult, TableSignature};

const DB_NAME: &str = "wallet_db_cache";
const DB_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbAccountsTable {
    account: BeBigUint,
    extfvk: String,
    address: String,
    ticker: String,
}

impl WalletDbAccountsTable {
    /// A **unique** index that consists of the following properties:
    /// * ticker
    /// * account
    pub const TICKER_ACCOUNT_INDEX: &str = "ticker_account_index";
}

impl TableSignature for WalletDbAccountsTable {
    fn table_name() -> &'static str { "walletdb_accounts" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(Self::TICKER_ACCOUNT_INDEX, &["ticker", "account"], true)?;
            table.create_index("ticker", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbBlocksTable {
    height: BeBigUint,
    hash: String,
    time: BeBigUint,
    sapling_tree: String,
    ticker: String,
}

impl WalletDbBlocksTable {
    /// A **unique** index that consists of the following properties:
    /// * ticker
    /// * height
    pub const TICKER_HEIGHT_INDEX: &str = "ticker_height_index";
    /// A **unique** index that consists of the following properties:
    /// * ticker
    /// * hash
    pub const TICKER_HASH_INDEX: &str = "ticker_hash_index";
}

impl TableSignature for WalletDbBlocksTable {
    fn table_name() -> &'static str { "walletdb_blocks" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(Self::TICKER_HEIGHT_INDEX, &["ticker", "height"], true)?;
            table.create_multi_index(Self::TICKER_HASH_INDEX, &["ticker", "hash"], true)?;
            table.create_index("ticker", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbTransactionsTable {
    id_tx: BeBigUint,
    txid: String, // unique
    created: String,
    block: BeBigUint,
    tx_index: BeBigUint,
    expiry_height: BeBigUint,
    raw: String,
    ticker: String,
}

impl WalletDbTransactionsTable {
    /// A **unique** index that consists of the following properties:
    /// * ticker
    /// * id_tx
    /// * txid
    pub const TICKER_ID_TX_INDEX: &'static str = "ticker_id_tx_index";
}

impl TableSignature for WalletDbTransactionsTable {
    fn table_name() -> &'static str { "walletdb_transactions" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(Self::TICKER_ID_TX_INDEX, &["ticker", "id_tx", "txid"], true)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbReceivedNotesTable {
    id_note: BeBigUint,
    tx: BeBigUint,
    output_index: BeBigUint,
    account: BeBigUint,
    diversifier: String,
    value: BeBigUint,
    rcm: String,
    nf: String, // unique
    is_change: BeBigUint,
    memo: String,
    spent: BeBigUint,
    ticker: String,
}

impl WalletDbReceivedNotesTable {
    /// A **unique** index that consists of the following properties:
    /// * ticker
    /// * note_id
    /// * nf
    pub const TICKER_NOTES_ID_NF_INDEX: &'static str = "ticker_note_id_nf_index";
    /// A **unique** index that consists of the following properties:
    /// * ticker
    /// * tx
    /// * output_index
    pub const TICKER_NOTES_TX_OUTPUT_INDEX: &'static str = "ticker_notes_tx_output_index";
}

impl TableSignature for WalletDbReceivedNotesTable {
    fn table_name() -> &'static str { "walletdb_received_notes" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(Self::TICKER_NOTES_ID_NF_INDEX, &["ticker", "id_note", "nf"], true)?;
            table.create_multi_index(
                Self::TICKER_NOTES_TX_OUTPUT_INDEX,
                &["ticker", "tx", "output_index"],
                true,
            )?;
            table.create_index("ticker", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbSaplingWitnessesTable {
    id_witness: BeBigUint,
    note: BeBigUint,
    block: BeBigUint,
    witness: String,
    ticker: String,
}

impl WalletDbSaplingWitnessesTable {
    /// A **unique** index that consists of the following properties:
    /// * ticker
    /// * note
    /// * block
    pub const TICKER_NOTE_BLOCK_INDEX: &'static str = "ticker_note_block_index";
    /// A **unique** index that consists of the following properties:
    /// * ticker
    /// * id_witness
    pub const TICKER_ID_WITNESS_INDEX: &'static str = "ticker_id_witness_index";
}

impl TableSignature for WalletDbSaplingWitnessesTable {
    fn table_name() -> &'static str { "walletdb_sapling_witness" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(Self::TICKER_NOTE_BLOCK_INDEX, &["ticker", "note", "block"], true)?;
            table.create_multi_index(Self::TICKER_ID_WITNESS_INDEX, &["ticker", "id_witness"], true)?;
            table.create_index("ticker", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbSentNotesTable {
    id_note: BeBigUint,
    tx: BeBigUint,
    output_index: BeBigUint,
    from_account: BeBigUint,
    address: String,
    value: BeBigUint,
    memo: String,
    ticker: String,
}

impl WalletDbSentNotesTable {
    /// A **unique** index that consists of the following properties:
    /// * ticker
    /// * tx
    /// * output_index
    pub const TICKER_TX_OUTPUT_INDEX: &'static str = "ticker_tx_output_index";
}

impl TableSignature for WalletDbSentNotesTable {
    fn table_name() -> &'static str { "walletdb_sent_notes" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(Self::TICKER_TX_OUTPUT_INDEX, &["ticker", "tx", "output_index"], true)?;
            table.create_index("ticker", false)?;
        }
        Ok(())
    }
}

pub struct WalletDbInner {
    pub inner: IndexedDb,
}

impl WalletDbInner {
    pub fn _get_inner(&self) -> &IndexedDb { &self.inner }
}

#[async_trait]
impl DbInstance for WalletDbInner {
    fn db_name() -> &'static str { DB_NAME }

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<WalletDbAccountsTable>()
            .with_table::<WalletDbBlocksTable>()
            .with_table::<WalletDbSaplingWitnessesTable>()
            .with_table::<WalletDbSentNotesTable>()
            .with_table::<WalletDbTransactionsTable>()
            .with_table::<WalletDbReceivedNotesTable>()
            .build()
            .await?;

        Ok(Self { inner })
    }
}

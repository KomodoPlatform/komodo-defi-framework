use async_trait::async_trait;
use mm2_db::indexed_db::{BeBigUint, DbIdentifier, DbInstance, DbLocked, DbUpgrader, IndexedDb, IndexedDbBuilder,
                         InitDbResult, OnUpgradeResult, TableSignature};

const DB_NAME: &str = "wallet_db_cache";
const DB_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbAccounts {
    account: BeBigUint,
    extfvk: String,
    address: String,
}

impl WalletDbAccounts {
    pub const ACCOUNT_ACCOUNT_INDEX: &str = "account_account_index";
}

impl TableSignature for WalletDbAccounts {
    fn table_name() -> &'static str { "walletdb_accounts" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_index(WalletDbAccounts::ACCOUNT_ACCOUNT_INDEX, true)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbBlocks {
    height: BeBigUint,
    hash: String,
    time: BeBigUint,
    sapling_tree: String,
}

impl WalletDbBlocks {
    pub const BLOCK_HEIGHT_INDEX: &str = "height";
}

impl TableSignature for WalletDbBlocks {
    fn table_name() -> &'static str { "walletdb_blocks" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_index(WalletDbBlocks::BLOCK_HEIGHT_INDEX, true)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbTransactions {
    id_tx: BeBigUint,
    txid: String, // unique
    created: String,
    block: BeBigUint,
    tx_index: BeBigUint,
    expiry_height: BeBigUint,
    raw: String,
}

impl WalletDbTransactions {
    /// A **unique** index that consists of the following properties:
    /// * id_tx
    /// * txid
    pub const TRANSACTION_ID_TX_INDEX: &'static str = "transaction_id_tx_index";
}

impl TableSignature for WalletDbTransactions {
    fn table_name() -> &'static str { "walletdb_transactions" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(WalletDbTransactions::TRANSACTION_ID_TX_INDEX, &["id_tx", "txid"], true)?;
            table.create_index("id_tx", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbReceivedNotes {
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
}

impl WalletDbReceivedNotes {
    /// A **unique** index that consists of the following properties:
    /// * note_id
    /// * nf
    pub const RECEIVED_NOTES_ID_NF_INDEX: &'static str = "received_note_id_nf_index";
    /// A **unique** index that consists of the following properties:
    /// * tx
    /// * output_index
    pub const RECEIVED_NOTES_TX_OUTPUT_INDEX: &'static str = "received_notes_tx_output_index";
}

impl TableSignature for WalletDbReceivedNotes {
    fn table_name() -> &'static str { "walletdb_received_notes" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(
                WalletDbReceivedNotes::RECEIVED_NOTES_ID_NF_INDEX,
                &["id_note", "nf"],
                true,
            )?;
            table.create_multi_index(
                WalletDbReceivedNotes::RECEIVED_NOTES_TX_OUTPUT_INDEX,
                &["tx", "output_index"],
                true,
            )?;
            table.create_index("id_note", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbSaplingWitnesses {
    id_witness: BeBigUint,
    note: BeBigUint,
    block: BeBigUint,
    witness: String,
}

impl WalletDbSaplingWitnesses {
    /// A **unique** index that consists of the following properties:
    /// * note
    /// * block
    pub const SAPLING_WITNESS_NOTE_BLOCK_INDEX: &'static str = "sapling_witness_note_block_index";
}

impl TableSignature for WalletDbSaplingWitnesses {
    fn table_name() -> &'static str { "walletdb_sapling_witness" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(
                WalletDbSaplingWitnesses::SAPLING_WITNESS_NOTE_BLOCK_INDEX,
                &["note", "block"],
                true,
            )?;
            table.create_index("id_witness", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletDbSentNotes {
    id_note: BeBigUint,
    tx: BeBigUint,
    output_index: BeBigUint,
    from_account: BeBigUint,
    address: String,
    value: BeBigUint,
    memo: String,
}

impl WalletDbSentNotes {
    /// A **unique** index that consists of the following properties:
    /// * transaction
    /// * output_index
    pub const SENT_NOTES_TX_OUTPUT_INDEX: &'static str = "sent_notes_tx_output_index";
}

impl TableSignature for WalletDbSentNotes {
    fn table_name() -> &'static str { "walletdb_sent_notes" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_index("id_note", true)?;
            table.create_multi_index(
                WalletDbSentNotes::SENT_NOTES_TX_OUTPUT_INDEX,
                &["tx", "output_index"],
                true,
            )?;
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
            .with_table::<WalletDbAccounts>()
            .with_table::<WalletDbBlocks>()
            .with_table::<WalletDbSaplingWitnesses>()
            .with_table::<WalletDbSentNotes>()
            .with_table::<WalletDbTransactions>()
            .with_table::<WalletDbReceivedNotes>()
            .build()
            .await?;

        Ok(Self { inner })
    }
}

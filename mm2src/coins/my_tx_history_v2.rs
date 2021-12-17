use crate::sql_tx_history_storage::SqliteTxHistoryStorage;
use crate::{lp_coinfind_or_err, BlockHeightAndTime, CoinFindError, HistorySyncState, Transaction, TransactionDetails,
            TransactionType, TxFeeDetails};
use async_trait::async_trait;
use bitcrypto::sha256;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::mm_number::BigDecimal;
use common::{ten, NotSame, PagingOptionsEnum};
use keys::{Address, CashAddress};
use rpc::v1::types::Bytes as BytesJson;
use std::collections::HashSet;

#[derive(Debug)]
pub enum RemoveTxResult {
    TxRemoved,
    TxDidNotExist,
}

impl RemoveTxResult {
    pub fn tx_existed(&self) -> bool { matches!(self, RemoveTxResult::TxRemoved) }
}

pub trait TxHistoryStorageError: std::fmt::Debug + NotMmError + NotSame + Send {}

#[async_trait]
pub trait TxHistoryStorage: Send + Sync + 'static {
    type Error: TxHistoryStorageError;

    /// Initializes collection/tables in storage for a specified coin
    async fn init(&self, for_coin: &str) -> Result<(), MmError<Self::Error>>;

    async fn is_initialized_for(&self, for_coin: &str) -> Result<bool, MmError<Self::Error>>;

    /// Adds multiple transactions to the selected coin's history
    /// Also consider adding tx_hex to the cache during this operation
    async fn add_transactions_to_history(
        &self,
        for_coin: &str,
        transactions: impl IntoIterator<Item = TransactionDetails> + Send + 'static,
    ) -> Result<(), MmError<Self::Error>>;

    /// Removes the transaction by internal_id from the selected coin's history
    async fn remove_tx_from_history(
        &self,
        for_coin: &str,
        internal_id: &BytesJson,
    ) -> Result<RemoveTxResult, MmError<Self::Error>>;

    /// Gets the transaction by internal_id from the selected coin's history
    async fn get_tx_from_history(
        &self,
        for_coin: &str,
        internal_id: &BytesJson,
    ) -> Result<Option<TransactionDetails>, MmError<Self::Error>>;

    /// Returns whether the history contains unconfirmed transactions
    async fn history_contains_unconfirmed_txes(&self, for_coin: &str) -> Result<bool, MmError<Self::Error>>;

    /// Gets the unconfirmed transactions from the history
    async fn get_unconfirmed_txes_from_history(
        &self,
        for_coin: &str,
    ) -> Result<Vec<TransactionDetails>, MmError<Self::Error>>;

    /// Updates transaction in the selected coin's history
    async fn update_tx_in_history(&self, for_coin: &str, tx: &TransactionDetails) -> Result<(), MmError<Self::Error>>;

    async fn history_has_tx_hash(&self, for_coin: &str, tx_hash: &str) -> Result<bool, MmError<Self::Error>>;

    async fn unique_tx_hashes_num_in_history(&self, for_coin: &str) -> Result<usize, MmError<Self::Error>>;

    async fn add_tx_to_cache(
        &self,
        for_coin: &str,
        tx_hash: &BytesJson,
        tx_hex: &BytesJson,
    ) -> Result<(), MmError<Self::Error>>;

    async fn tx_bytes_from_cache(
        &self,
        for_coin: &str,
        tx_hash: &BytesJson,
    ) -> Result<Option<BytesJson>, MmError<Self::Error>>;
}

pub trait DisplayAddress {
    fn display_address(&self) -> String;
}

impl DisplayAddress for Address {
    fn display_address(&self) -> String { self.to_string() }
}

impl DisplayAddress for CashAddress {
    fn display_address(&self) -> String { self.encode().expect("A valid cash address") }
}

pub struct TxDetailsBuilder<'a, Addr: DisplayAddress, Tx: Transaction> {
    coin: String,
    tx: &'a Tx,
    my_addresses: HashSet<Addr>,
    total_amount: BigDecimal,
    received_by_me: BigDecimal,
    spent_by_me: BigDecimal,
    from_addresses: HashSet<Addr>,
    to_addresses: HashSet<Addr>,
    transaction_type: TransactionType,
    block_height_and_time: Option<BlockHeightAndTime>,
    tx_fee: Option<TxFeeDetails>,
}

impl<'a, Addr: Clone + DisplayAddress + Eq + std::hash::Hash, Tx: Transaction> TxDetailsBuilder<'a, Addr, Tx> {
    pub fn new(
        coin: String,
        tx: &'a Tx,
        block_height_and_time: Option<BlockHeightAndTime>,
        my_addresses: impl IntoIterator<Item = Addr>,
    ) -> Self {
        TxDetailsBuilder {
            coin,
            tx,
            my_addresses: my_addresses.into_iter().collect(),
            total_amount: Default::default(),
            received_by_me: Default::default(),
            spent_by_me: Default::default(),
            from_addresses: Default::default(),
            to_addresses: Default::default(),
            block_height_and_time,
            transaction_type: TransactionType::StandardTransfer,
            tx_fee: None,
        }
    }

    pub fn set_tx_fee(&mut self, tx_fee: Option<TxFeeDetails>) { self.tx_fee = tx_fee; }

    pub fn set_transaction_type(&mut self, tx_type: TransactionType) { self.transaction_type = tx_type; }

    pub fn transferred_to(&mut self, address: Addr, amount: &BigDecimal) {
        if self.my_addresses.contains(&address) {
            self.received_by_me += amount;
        }
        self.to_addresses.insert(address);
    }

    pub fn transferred_from(&mut self, address: Addr, amount: &BigDecimal) {
        if self.my_addresses.contains(&address) {
            self.spent_by_me += amount;
        }
        self.total_amount += amount;
        self.from_addresses.insert(address);
    }

    pub fn build(self) -> TransactionDetails {
        let (block_height, timestamp) = match self.block_height_and_time {
            Some(height_with_time) => (height_with_time.height, height_with_time.timestamp),
            None => (0, 0),
        };

        let mut from: Vec<_> = self
            .from_addresses
            .iter()
            .map(DisplayAddress::display_address)
            .collect();
        from.sort();

        let mut to: Vec<_> = self.to_addresses.iter().map(DisplayAddress::display_address).collect();
        to.sort();

        let tx_hash = self.tx.tx_hash();
        let internal_id = match &self.transaction_type {
            TransactionType::TokenTransfer(token_id) => {
                let mut bytes_for_hash = tx_hash.0.clone();
                bytes_for_hash.extend_from_slice(&token_id.0);
                sha256(&bytes_for_hash).to_vec().into()
            },
            TransactionType::StakingDelegation
            | TransactionType::RemoveDelegation
            | TransactionType::StandardTransfer => tx_hash.clone(),
        };

        TransactionDetails {
            coin: self.coin,
            tx_hex: self.tx.tx_hex().into(),
            tx_hash,
            from,
            to,
            total_amount: self.total_amount,
            my_balance_change: &self.received_by_me - &self.spent_by_me,
            spent_by_me: self.spent_by_me,
            received_by_me: self.received_by_me,
            block_height,
            timestamp,
            fee_details: self.tx_fee,
            internal_id,
            kmd_rewards: None,
            transaction_type: self.transaction_type,
        }
    }
}

#[derive(Deserialize)]
pub struct MyTxHistoryRequestV2 {
    coin: String,
    #[serde(default = "ten")]
    limit: usize,
    #[serde(default)]
    paging_options: PagingOptionsEnum<BytesJson>,
}

#[derive(Serialize)]
pub struct MyHistoryTxDetails {
    #[serde(flatten)]
    details: TransactionDetails,
    confirmations: u64,
}

#[derive(Serialize)]
pub struct MyTxHistoryResponseV2 {
    coin: String,
    current_block: u64,
    transactions: Vec<MyHistoryTxDetails>,
    sync_status: HistorySyncState,
    limit: usize,
    skipped: usize,
    total: usize,
    total_pages: usize,
    paging_options: PagingOptionsEnum<BytesJson>,
}

pub enum MyTxHistoryErrorV2 {
    CoinIsNotActive(String),
    StorageIsNotInitialized(String),
    StorageError(String),
}

impl From<CoinFindError> for MyTxHistoryErrorV2 {
    fn from(err: CoinFindError) -> Self {
        match err {
            CoinFindError::NoSuchCoin { coin } => MyTxHistoryErrorV2::CoinIsNotActive(coin),
        }
    }
}

impl<T: TxHistoryStorageError> From<T> for MyTxHistoryErrorV2 {
    fn from(err: T) -> Self {
        let msg = format!("{:?}", err);
        MyTxHistoryErrorV2::StorageError(msg)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn my_tx_history_v2_rpc(
    ctx: MmArc,
    request: MyTxHistoryRequestV2,
) -> Result<MyTxHistoryResponseV2, MmError<MyTxHistoryErrorV2>> {
    let coin = lp_coinfind_or_err(&ctx, &request.coin).await?;
    let tx_history_storage = SqliteTxHistoryStorage(
        ctx.sqlite_connection
            .ok_or(MmError::new(MyTxHistoryErrorV2::StorageIsNotInitialized(
                "sqlite_connection is not initialized".into(),
            )))?
            .clone(),
    );
    let is_storage_init = tx_history_storage.is_initialized_for(coin.ticker()).await?;
    if !is_storage_init {
        let msg = format!("Storage is not initialized for {}", coin.ticker());
        return MmError::err(MyTxHistoryErrorV2::StorageIsNotInitialized(msg));
    }
    unimplemented!()
}

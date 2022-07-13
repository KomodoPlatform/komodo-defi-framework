use crate::tx_history_storage::{CreateTxHistoryStorageError, GetTxHistoryFilters, TxHistoryStorageBuilder, WalletId};
use crate::{lp_coinfind_or_err, BlockHeightAndTime, CoinFindError, HistorySyncState, MmCoin, MmCoinEnum, Transaction,
            TransactionDetails, TransactionType, TxFeeDetails, UtxoRpcError};
use async_trait::async_trait;
use bitcrypto::sha256;
use common::{calc_total_pages, ten, HttpStatusCode, PagingOptionsEnum, StatusCode};
use derive_more::Display;
use futures::compat::Future01CompatExt;
use keys::{Address, CashAddress};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use rpc::v1::types::{Bytes as BytesJson, ToTxHash};
use std::collections::HashSet;

#[derive(Debug)]
pub enum RemoveTxResult {
    TxRemoved,
    TxDidNotExist,
}

impl RemoveTxResult {
    pub fn tx_existed(&self) -> bool { matches!(self, RemoveTxResult::TxRemoved) }
}

pub struct GetHistoryResult {
    pub transactions: Vec<TransactionDetails>,
    pub skipped: usize,
    pub total: usize,
}

pub trait TxHistoryStorageError: std::fmt::Debug + NotMmError + NotEqual + Send {}

#[async_trait]
pub trait TxHistoryStorage: Send + Sync + 'static {
    type Error: TxHistoryStorageError;

    /// Initializes collection/tables in storage for the specified wallet.
    async fn init(&self, wallet_id: &WalletId) -> Result<(), MmError<Self::Error>>;

    /// Whether collections/tables are initialized for the specified wallet.
    async fn is_initialized_for(&self, wallet_id: &WalletId) -> Result<bool, MmError<Self::Error>>;

    /// Adds multiple transactions to the selected wallet's history.
    /// Also consider adding tx_hex to the cache during this operation.
    async fn add_transactions_to_history<I>(
        &self,
        wallet_id: &WalletId,
        transactions: I,
    ) -> Result<(), MmError<Self::Error>>
    where
        I: IntoIterator<Item = TransactionDetails> + Send + 'static,
        I::IntoIter: Send;

    /// Removes the transaction by internal_id from the selected wallet's history.
    async fn remove_tx_from_history(
        &self,
        wallet_id: &WalletId,
        internal_id: &BytesJson,
    ) -> Result<RemoveTxResult, MmError<Self::Error>>;

    /// Gets the transaction by internal_id from the selected wallet's history
    async fn get_tx_from_history(
        &self,
        wallet_id: &WalletId,
        internal_id: &BytesJson,
    ) -> Result<Option<TransactionDetails>, MmError<Self::Error>>;

    /// Returns whether the history contains unconfirmed transactions.
    async fn history_contains_unconfirmed_txes(&self, wallet_id: &WalletId) -> Result<bool, MmError<Self::Error>>;

    /// Gets the unconfirmed transactions from the wallet's history.
    async fn get_unconfirmed_txes_from_history(
        &self,
        wallet_id: &WalletId,
    ) -> Result<Vec<TransactionDetails>, MmError<Self::Error>>;

    /// Updates transaction in the selected wallet's history
    async fn update_tx_in_history(
        &self,
        wallet_id: &WalletId,
        tx: &TransactionDetails,
    ) -> Result<(), MmError<Self::Error>>;

    /// Whether the selected wallet's history contains a transaction with the given `tx_hash`.
    async fn history_has_tx_hash(&self, wallet_id: &WalletId, tx_hash: &str) -> Result<bool, MmError<Self::Error>>;

    /// Returns the number of unique transaction hashes.
    async fn unique_tx_hashes_num_in_history(&self, wallet_id: &WalletId) -> Result<usize, MmError<Self::Error>>;

    /// Adds the given `tx_hex` transaction to the selected wallet's cache.
    async fn add_tx_to_cache(
        &self,
        wallet_id: &WalletId,
        tx_hash: &str,
        tx_hex: &BytesJson,
    ) -> Result<(), MmError<Self::Error>>;

    /// Gets transaction hexes from the wallet's cache.
    async fn tx_bytes_from_cache(
        &self,
        wallet_id: &WalletId,
        tx_hash: &str,
    ) -> Result<Option<BytesJson>, MmError<Self::Error>>;

    /// Gets transaction history for the selected wallet according to the specified `filters`.
    async fn get_history(
        &self,
        wallet_id: &WalletId,
        filters: GetTxHistoryFilters,
        paging: PagingOptionsEnum<BytesJson>,
        limit: usize,
    ) -> Result<GetHistoryResult, MmError<Self::Error>>;
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
            tx_hash: tx_hash.to_tx_hash(),
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
pub struct MyTxHistoryRequestV2<T> {
    coin: String,
    #[serde(default = "ten")]
    pub(crate) limit: usize,
    #[serde(default)]
    pub(crate) paging_options: PagingOptionsEnum<T>,
}

#[derive(Serialize)]
pub struct MyTxHistoryDetails {
    #[serde(flatten)]
    pub(crate) details: TransactionDetails,
    pub(crate) confirmations: u64,
}

#[derive(Serialize)]
pub struct MyTxHistoryResponseV2<Tx, Id> {
    pub(crate) coin: String,
    pub(crate) current_block: u64,
    pub(crate) transactions: Vec<Tx>,
    pub(crate) sync_status: HistorySyncState,
    pub(crate) limit: usize,
    pub(crate) skipped: usize,
    pub(crate) total: usize,
    pub(crate) total_pages: usize,
    pub(crate) paging_options: PagingOptionsEnum<Id>,
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum MyTxHistoryErrorV2 {
    CoinIsNotActive(String),
    StorageIsNotInitialized(String),
    StorageError(String),
    RpcError(String),
    NotSupportedFor(String),
    Internal(String),
}

impl HttpStatusCode for MyTxHistoryErrorV2 {
    fn status_code(&self) -> StatusCode {
        match self {
            MyTxHistoryErrorV2::CoinIsNotActive(_) => StatusCode::PRECONDITION_REQUIRED,
            MyTxHistoryErrorV2::StorageIsNotInitialized(_)
            | MyTxHistoryErrorV2::StorageError(_)
            | MyTxHistoryErrorV2::RpcError(_)
            | MyTxHistoryErrorV2::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            MyTxHistoryErrorV2::NotSupportedFor(_) => StatusCode::BAD_REQUEST,
        }
    }
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

impl From<CreateTxHistoryStorageError> for MyTxHistoryErrorV2 {
    fn from(e: CreateTxHistoryStorageError) -> Self { MyTxHistoryErrorV2::StorageError(e.to_string()) }
}

impl From<UtxoRpcError> for MyTxHistoryErrorV2 {
    fn from(err: UtxoRpcError) -> Self { MyTxHistoryErrorV2::RpcError(err.to_string()) }
}

pub trait CoinWithTxHistoryV2 {
    fn history_wallet_id(&self) -> WalletId;

    fn get_tx_history_filters(&self) -> GetTxHistoryFilters;
}

/// According to the [comment](https://github.com/KomodoPlatform/atomicDEX-API/pull/1285#discussion_r888410390),
/// it's worth to add [`MmCoin::my_tx_history_v2`] when most coins support transaction history V2.
pub async fn my_tx_history_v2_rpc(
    ctx: MmArc,
    request: MyTxHistoryRequestV2<BytesJson>,
) -> Result<MyTxHistoryResponseV2<MyTxHistoryDetails, BytesJson>, MmError<MyTxHistoryErrorV2>> {
    match lp_coinfind_or_err(&ctx, &request.coin).await? {
        MmCoinEnum::Bch(bch) => my_tx_history_v2_impl(ctx, &bch, request).await,
        MmCoinEnum::SlpToken(slp_token) => my_tx_history_v2_impl(ctx, &slp_token, request).await,
        other => MmError::err(MyTxHistoryErrorV2::NotSupportedFor(other.ticker().to_owned())),
    }
}

pub(crate) async fn my_tx_history_v2_impl<Coin>(
    ctx: MmArc,
    coin: &Coin,
    request: MyTxHistoryRequestV2<BytesJson>,
) -> Result<MyTxHistoryResponseV2<MyTxHistoryDetails, BytesJson>, MmError<MyTxHistoryErrorV2>>
where
    Coin: CoinWithTxHistoryV2 + MmCoin,
{
    let tx_history_storage = TxHistoryStorageBuilder::new(&ctx).build()?;

    let wallet_id = coin.history_wallet_id();
    let is_storage_init = tx_history_storage.is_initialized_for(&wallet_id).await?;
    if !is_storage_init {
        let msg = format!("Storage is not initialized for {:?}", wallet_id);
        return MmError::err(MyTxHistoryErrorV2::StorageIsNotInitialized(msg));
    }
    let current_block = coin
        .current_block()
        .compat()
        .await
        .map_to_mm(MyTxHistoryErrorV2::RpcError)?;

    let filters = coin.get_tx_history_filters();
    let history = tx_history_storage
        .get_history(&wallet_id, filters, request.paging_options.clone(), request.limit)
        .await?;

    let transactions = history
        .transactions
        .into_iter()
        .map(|mut details| {
            // it can be the platform ticker instead of the token ticker for a pre-saved record
            if details.coin != request.coin {
                details.coin = request.coin.clone();
            }
            let confirmations = if details.block_height == 0 || details.block_height > current_block {
                0
            } else {
                current_block + 1 - details.block_height
            };
            MyTxHistoryDetails { confirmations, details }
        })
        .collect();

    Ok(MyTxHistoryResponseV2 {
        coin: request.coin,
        current_block,
        transactions,
        sync_status: coin.history_sync_status(),
        limit: request.limit,
        skipped: history.skipped,
        total: history.total,
        total_pages: calc_total_pages(history.total, request.limit),
        paging_options: request.paging_options,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn z_coin_tx_history_rpc(
    ctx: MmArc,
    request: MyTxHistoryRequestV2<i64>,
) -> Result<MyTxHistoryResponseV2<crate::z_coin::ZcoinTxDetails, i64>, MmError<MyTxHistoryErrorV2>> {
    match lp_coinfind_or_err(&ctx, &request.coin).await? {
        MmCoinEnum::ZCoin(z_coin) => z_coin.tx_history(request).await,
        other => MmError::err(MyTxHistoryErrorV2::NotSupportedFor(other.ticker().to_owned())),
    }
}

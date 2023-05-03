use super::{rpc::*, AllBalancesResult, TendermintCoin, TendermintCommons, TendermintToken};

use crate::my_tx_history_v2::{CoinWithTxHistoryV2, MyTxHistoryErrorV2, MyTxHistoryTarget, TxHistoryStorage};
use crate::tendermint::{CustomTendermintMsgType, TendermintFeeDetails};
use crate::tx_history_storage::{GetTxHistoryFilters, WalletId};
use crate::utxo::utxo_common::big_decimal_from_sat_unsigned;
use crate::{HistorySyncState, MarketCoinOps, MmCoin, TransactionDetails, TransactionType, TxFeeDetails};
use async_trait::async_trait;
use bitcrypto::sha256;
use common::executor::Timer;
use common::log;
use common::state_machine::prelude::*;
use cosmrs::tendermint::abci::Code as TxCode;
use cosmrs::tendermint::abci::Event;
use cosmrs::tx::Fee;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmResult;
use mm2_number::BigDecimal;
use primitives::hash::H256;
use rpc::v1::types::Bytes as BytesJson;
use std::cmp;

macro_rules! try_or_return_stopped_as_err {
    ($exp:expr, $reason: expr, $fmt:literal) => {
        match $exp {
            Ok(t) => t,
            Err(e) => {
                return Err(Stopped {
                    phantom: Default::default(),
                    stop_reason: $reason(format!("{}: {}", $fmt, e)),
                })
            },
        }
    };
}

macro_rules! try_or_continue {
    ($exp:expr, $fmt:literal) => {
        match $exp {
            Ok(t) => t,
            Err(e) => {
                log::debug!("{}: {}", $fmt, e);
                continue;
            },
        }
    };
}

macro_rules! some_or_continue {
    ($exp:expr) => {
        match $exp {
            Some(t) => t,
            None => {
                continue;
            },
        }
    };
}

macro_rules! some_or_return {
    ($exp:expr) => {
        match $exp {
            Some(t) => t,
            None => {
                return;
            },
        }
    };
}

trait CoinCapabilities: TendermintCommons + CoinWithTxHistoryV2 + MmCoin + MarketCoinOps {}
impl CoinCapabilities for TendermintCoin {}

#[async_trait]
impl CoinWithTxHistoryV2 for TendermintCoin {
    fn history_wallet_id(&self) -> WalletId { WalletId::new(self.ticker().into()) }

    async fn get_tx_history_filters(
        &self,
        _target: MyTxHistoryTarget,
    ) -> MmResult<GetTxHistoryFilters, MyTxHistoryErrorV2> {
        Ok(GetTxHistoryFilters::for_address(self.account_id.to_string()))
    }
}

#[async_trait]
impl CoinWithTxHistoryV2 for TendermintToken {
    fn history_wallet_id(&self) -> WalletId { WalletId::new(self.platform_ticker().into()) }

    async fn get_tx_history_filters(
        &self,
        _target: MyTxHistoryTarget,
    ) -> MmResult<GetTxHistoryFilters, MyTxHistoryErrorV2> {
        let denom_hash = sha256(self.denom.to_string().as_bytes());
        let id = H256::from(denom_hash.as_slice());

        Ok(GetTxHistoryFilters::for_address(self.platform_coin.account_id.to_string()).with_token_id(id.to_string()))
    }
}

struct TendermintTxHistoryCtx<Coin: CoinCapabilities, Storage: TxHistoryStorage> {
    coin: Coin,
    storage: Storage,
    balances: AllBalancesResult,
    last_received_page: u32,
    last_spent_page: u32,
}

struct TendermintInit<Coin, Storage> {
    phantom: std::marker::PhantomData<(Coin, Storage)>,
}

impl<Coin, Storage> TendermintInit<Coin, Storage> {
    fn new() -> Self {
        TendermintInit {
            phantom: Default::default(),
        }
    }
}

#[derive(Debug)]
enum StopReason {
    StorageError(String),
    RpcClient(String),
}

struct Stopped<Coin, Storage> {
    phantom: std::marker::PhantomData<(Coin, Storage)>,
    stop_reason: StopReason,
}

impl<Coin, Storage> Stopped<Coin, Storage> {
    fn storage_error<E>(e: E) -> Self
    where
        E: std::fmt::Debug,
    {
        Stopped {
            phantom: Default::default(),
            stop_reason: StopReason::StorageError(format!("{:?}", e)),
        }
    }
}

struct WaitForHistoryUpdateTrigger<Coin, Storage> {
    address: String,
    last_height_state: u64,
    phantom: std::marker::PhantomData<(Coin, Storage)>,
}

impl<Coin, Storage> WaitForHistoryUpdateTrigger<Coin, Storage> {
    fn new(address: String, last_height_state: u64) -> Self {
        WaitForHistoryUpdateTrigger {
            address,
            last_height_state,
            phantom: Default::default(),
        }
    }
}

struct OnIoErrorCooldown<Coin, Storage> {
    address: String,
    last_block_height: u64,
    phantom: std::marker::PhantomData<(Coin, Storage)>,
}

impl<Coin, Storage> OnIoErrorCooldown<Coin, Storage> {
    fn new(address: String, last_block_height: u64) -> Self {
        OnIoErrorCooldown {
            address,
            last_block_height,
            phantom: Default::default(),
        }
    }
}

impl<Coin, Storage> TransitionFrom<FetchingTransactionsData<Coin, Storage>> for OnIoErrorCooldown<Coin, Storage> {}

#[async_trait]
impl<Coin, Storage> State for OnIoErrorCooldown<Coin, Storage>
where
    Coin: CoinCapabilities,
    Storage: TxHistoryStorage,
{
    type Ctx = TendermintTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(mut self: Box<Self>, _ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        Timer::sleep(30.).await;

        // retry history fetching process from last saved block
        return Self::change_state(FetchingTransactionsData::new(self.address, self.last_block_height));
    }
}

struct FetchingTransactionsData<Coin, Storage> {
    /// The list of addresses for those we have requested [`UpdatingUnconfirmedTxes::all_tx_ids_with_height`] TX hashes
    /// at the `FetchingTxHashes` state.
    address: String,
    from_block_height: u64,
    phantom: std::marker::PhantomData<(Coin, Storage)>,
}

impl<Coin, Storage> FetchingTransactionsData<Coin, Storage> {
    fn new(address: String, from_block_height: u64) -> Self {
        FetchingTransactionsData {
            address,
            phantom: Default::default(),
            from_block_height,
        }
    }
}

impl<Coin, Storage> TransitionFrom<TendermintInit<Coin, Storage>> for Stopped<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<TendermintInit<Coin, Storage>> for FetchingTransactionsData<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<OnIoErrorCooldown<Coin, Storage>> for FetchingTransactionsData<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<WaitForHistoryUpdateTrigger<Coin, Storage>> for OnIoErrorCooldown<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<WaitForHistoryUpdateTrigger<Coin, Storage>> for Stopped<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<FetchingTransactionsData<Coin, Storage>> for Stopped<Coin, Storage> {}

impl<Coin, Storage> TransitionFrom<WaitForHistoryUpdateTrigger<Coin, Storage>>
    for FetchingTransactionsData<Coin, Storage>
{
}

impl<Coin, Storage> TransitionFrom<FetchingTransactionsData<Coin, Storage>>
    for WaitForHistoryUpdateTrigger<Coin, Storage>
{
}

#[async_trait]
impl<Coin, Storage> State for WaitForHistoryUpdateTrigger<Coin, Storage>
where
    Coin: CoinCapabilities,
    Storage: TxHistoryStorage,
{
    type Ctx = TendermintTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        loop {
            Timer::sleep(30.).await;

            let ctx_balances = ctx.balances.clone();

            let balances = match ctx.coin.all_balances().await {
                Ok(balances) => balances,
                Err(_) => {
                    return Self::change_state(OnIoErrorCooldown::new(self.address.clone(), self.last_height_state));
                },
            };

            if balances != ctx_balances {
                // Update balances
                ctx.balances = balances;

                return Self::change_state(FetchingTransactionsData::new(
                    self.address.clone(),
                    self.last_height_state,
                ));
            }
        }
    }
}

#[async_trait]
impl<Coin, Storage> State for FetchingTransactionsData<Coin, Storage>
where
    Coin: CoinCapabilities,
    Storage: TxHistoryStorage,
{
    type Ctx = TendermintTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        const TX_PAGE_SIZE: u8 = 50;

        const DEFAULT_TRANSFER_EVENT_COUNT: usize = 1;
        const CREATE_HTLC_EVENT: &str = "create_htlc";
        const CLAIM_HTLC_EVENT: &str = "claim_htlc";
        const TRANSFER_EVENT: &str = "transfer";
        const ACCEPTED_EVENTS: &[&str] = &[CREATE_HTLC_EVENT, CLAIM_HTLC_EVENT, TRANSFER_EVENT];
        const RECIPIENT_TAG_KEY: &str = "recipient";
        const SENDER_TAG_KEY: &str = "sender";
        const RECEIVER_TAG_KEY: &str = "receiver";
        const AMOUNT_TAG_KEY: &str = "amount";

        struct TxAmounts {
            total: BigDecimal,
            spent_by_me: BigDecimal,
            received_by_me: BigDecimal,
        }

        fn get_tx_amounts(
            transfer_details: &TransferDetails,
            is_self_transfer: bool,
            sent_by_me: bool,
            is_sign_claim_htlc: bool,
            fee_details: Option<&TendermintFeeDetails>,
        ) -> TxAmounts {
            let amount = BigDecimal::from(transfer_details.amount);

            let total = if is_sign_claim_htlc && !is_self_transfer {
                BigDecimal::default()
            } else {
                amount.clone()
            };

            let spent_by_me =
                if sent_by_me && !matches!(transfer_details.transfer_event_type, TransferEventType::ClaimHtlc) {
                    amount.clone()
                } else {
                    BigDecimal::default()
                };

            let received_by_me = if !sent_by_me || is_self_transfer {
                amount
            } else {
                BigDecimal::default()
            };

            let mut tx_amounts = TxAmounts {
                total,
                spent_by_me,
                received_by_me,
            };

            if let Some(fee_details) = fee_details {
                tx_amounts.total += BigDecimal::from(fee_details.uamount);
                tx_amounts.spent_by_me += BigDecimal::from(fee_details.uamount);
            }

            tx_amounts
        }

        fn get_fee_details<Coin>(fee: Fee, coin: &Coin) -> Result<TendermintFeeDetails, String>
        where
            Coin: CoinCapabilities,
        {
            let fee_coin = fee
                .amount
                .first()
                .ok_or_else(|| "fee coin can't be empty".to_string())?;
            let fee_uamount: u64 = fee_coin.amount.to_string().parse().map_err(|e| format!("{:?}", e))?;

            Ok(TendermintFeeDetails {
                coin: coin.platform_ticker().to_string(),
                amount: big_decimal_from_sat_unsigned(fee_uamount, coin.decimals()),
                uamount: fee_uamount,
                gas_limit: fee.gas_limit.value(),
            })
        }

        #[derive(Default, Clone)]
        enum TransferEventType {
            #[default]
            Standard,
            CreateHtlc,
            ClaimHtlc,
        }

        #[derive(Clone)]
        struct TransferDetails {
            from: String,
            to: String,
            denom: String,
            amount: u64,
            transfer_event_type: TransferEventType,
        }

        // updates sender and receiver addresses if tx is htlc, and if not leaves as it is.
        fn read_real_htlc_addresses(transfer_details: &mut TransferDetails, msg_event: &&Event) {
            match msg_event.type_str.as_str() {
                CREATE_HTLC_EVENT => {
                    transfer_details.from = some_or_return!(msg_event
                        .attributes
                        .iter()
                        .find(|tag| tag.key.to_string() == SENDER_TAG_KEY))
                    .value
                    .to_string();

                    transfer_details.to = some_or_return!(msg_event
                        .attributes
                        .iter()
                        .find(|tag| tag.key.to_string() == RECEIVER_TAG_KEY))
                    .value
                    .to_string();

                    transfer_details.transfer_event_type = TransferEventType::CreateHtlc;
                },
                CLAIM_HTLC_EVENT => {
                    transfer_details.from = some_or_return!(msg_event
                        .attributes
                        .iter()
                        .find(|tag| tag.key.to_string() == SENDER_TAG_KEY))
                    .value
                    .to_string();

                    transfer_details.transfer_event_type = TransferEventType::ClaimHtlc;
                },
                _ => {},
            }
        }

        fn parse_transfer_values_from_events(tx_events: Vec<&Event>) -> Vec<TransferDetails> {
            let mut transfer_details_list: Vec<TransferDetails> = vec![];

            for (index, event) in tx_events.iter().enumerate() {
                if event.type_str.as_str() == TRANSFER_EVENT {
                    let amount_with_denoms = some_or_continue!(event
                        .attributes
                        .iter()
                        .find(|tag| tag.key.to_string() == AMOUNT_TAG_KEY))
                    .value
                    .to_string();
                    let amount_with_denoms = amount_with_denoms.split(',');

                    for amount_with_denom in amount_with_denoms {
                        let extracted_amount: String =
                            amount_with_denom.chars().take_while(|c| c.is_numeric()).collect();
                        let denom = &amount_with_denom[extracted_amount.len()..];
                        let amount = some_or_continue!(extracted_amount.parse().ok());

                        let from = some_or_continue!(event
                            .attributes
                            .iter()
                            .find(|tag| tag.key.to_string() == SENDER_TAG_KEY))
                        .value
                        .to_string();

                        let to = some_or_continue!(event
                            .attributes
                            .iter()
                            .find(|tag| tag.key.to_string() == RECIPIENT_TAG_KEY))
                        .value
                        .to_string();

                        let mut tx_details = TransferDetails {
                            from,
                            to,
                            denom: denom.to_owned(),
                            amount,
                            // Default is Standard, can be changed later in read_real_htlc_addresses
                            transfer_event_type: TransferEventType::default(),
                        };

                        if index != 0 {
                            // If previous message is htlc related, that means current transfer
                            // addresses will be wrong.
                            if let Some(prev_event) = tx_events.get(index - 1) {
                                if [CREATE_HTLC_EVENT, CLAIM_HTLC_EVENT].contains(&prev_event.type_str.as_str()) {
                                    read_real_htlc_addresses(&mut tx_details, prev_event);
                                }
                            };
                        }

                        // sum the amounts coins and pairs are same
                        let mut duplicated_details = transfer_details_list.iter_mut().find(|details| {
                            details.from == tx_details.from
                                && details.to == tx_details.to
                                && details.denom == tx_details.denom
                        });

                        if let Some(duplicated_details) = &mut duplicated_details {
                            duplicated_details.amount += tx_details.amount;
                        } else {
                            transfer_details_list.push(tx_details);
                        }
                    }
                }
            }

            transfer_details_list
        }

        fn get_transfer_details(tx_events: Vec<Event>, fee_amount_with_denom: String) -> Vec<TransferDetails> {
            // Filter out irrelevant events
            let mut events: Vec<&Event> = tx_events
                .iter()
                .filter(|event| ACCEPTED_EVENTS.contains(&event.type_str.as_str()))
                .collect();

            events.reverse();

            if events.len() > DEFAULT_TRANSFER_EVENT_COUNT {
                // Retain fee related events
                events.retain(|event| {
                    if event.type_str == TRANSFER_EVENT {
                        let amount_with_denom = event
                            .attributes
                            .iter()
                            .find(|tag| tag.key.to_string() == AMOUNT_TAG_KEY)
                            .map(|t| t.value.to_string());

                        amount_with_denom != Some(fee_amount_with_denom.clone())
                    } else {
                        true
                    }
                });
            }

            parse_transfer_values_from_events(events)
        }

        fn get_transaction_type(
            transfer_event_type: &TransferEventType,
            token_id: Option<BytesJson>,
            is_sign_claim_htlc: bool,
        ) -> TransactionType {
            match (transfer_event_type, token_id) {
                (TransferEventType::CreateHtlc, token_id) => TransactionType::CustomTendermintMsg {
                    msg_type: CustomTendermintMsgType::SendHtlcAmount,
                    token_id,
                },
                (TransferEventType::ClaimHtlc, token_id) => TransactionType::CustomTendermintMsg {
                    msg_type: if is_sign_claim_htlc {
                        CustomTendermintMsgType::SignClaimHtlc
                    } else {
                        CustomTendermintMsgType::ClaimHtlcAmount
                    },
                    token_id,
                },
                (_, Some(token_id)) => TransactionType::TokenTransfer(token_id),
                _ => TransactionType::StandardTransfer,
            }
        }

        fn get_pair_addresses(
            my_address: String,
            tx_sent_by_me: bool,
            transfer_details: &TransferDetails,
        ) -> Option<(Vec<String>, Vec<String>)> {
            match transfer_details.transfer_event_type {
                TransferEventType::CreateHtlc => {
                    if tx_sent_by_me {
                        Some((vec![my_address], vec![]))
                    } else {
                        // This shouldn't happen if rpc node properly executes the tx search query.
                        None
                    }
                },
                TransferEventType::ClaimHtlc => Some((vec![my_address], vec![])),
                TransferEventType::Standard => {
                    Some((vec![transfer_details.from.clone()], vec![transfer_details.to.clone()]))
                },
            }
        }

        async fn fetch_and_insert_txs<Coin, Storage>(
            address: String,
            coin: &Coin,
            storage: &Storage,
            query: String,
            from_height: u64,
            page: &mut u32,
        ) -> Result<u64, Stopped<Coin, Storage>>
        where
            Coin: CoinCapabilities,
            Storage: TxHistoryStorage,
        {
            let mut highest_height = from_height;

            let client = try_or_return_stopped_as_err!(
                coin.rpc_client().await,
                StopReason::RpcClient,
                "could not get rpc client"
            );

            loop {
                let response = try_or_return_stopped_as_err!(
                    client
                        .perform(TxSearchRequest::new(
                            query.clone(),
                            false,
                            *page,
                            TX_PAGE_SIZE,
                            TendermintResultOrder::Ascending.into(),
                        ))
                        .await,
                    StopReason::RpcClient,
                    "tx search rpc call failed"
                );

                let mut tx_details = vec![];
                let current_page_is_full = response.txs.len() == TX_PAGE_SIZE as usize;
                for tx in response.txs {
                    if tx.tx_result.code != TxCode::Ok {
                        continue;
                    }

                    let timestamp = try_or_return_stopped_as_err!(
                        coin.get_block_timestamp(i64::from(tx.height)).await,
                        StopReason::RpcClient,
                        "could not get block_timestamp over rpc node"
                    );
                    let timestamp = some_or_continue!(timestamp);

                    let tx_hash = tx.hash.to_string();

                    highest_height = cmp::max(highest_height, tx.height.into());

                    let deserialized_tx = try_or_continue!(
                        cosmrs::Tx::from_bytes(tx.tx.as_bytes()),
                        "Could not deserialize transaction"
                    );

                    let msg = try_or_continue!(
                        deserialized_tx.body.messages.first().ok_or("Tx body couldn't be read."),
                        "Tx body messages is empty"
                    )
                    .value
                    .as_slice();

                    let fee_data = match deserialized_tx.auth_info.fee.amount.first() {
                        Some(data) => data,
                        None => {
                            log::debug!("Could not read transaction fee for tx '{}', skipping it", &tx_hash);
                            continue;
                        },
                    };

                    let fee_amount_with_denom = format!("{}{}", fee_data.amount, fee_data.denom);

                    let transfer_details_list = get_transfer_details(tx.tx_result.events, fee_amount_with_denom);

                    if transfer_details_list.is_empty() {
                        log::debug!(
                            "Could not find transfer details in events for tx '{}', skipping it",
                            &tx_hash
                        );
                        continue;
                    }

                    let fee_details = try_or_continue!(
                        get_fee_details(deserialized_tx.auth_info.fee, coin),
                        "get_fee_details failed"
                    );

                    let mut fee_added = false;
                    for (index, transfer_details) in transfer_details_list.iter().enumerate() {
                        let mut internal_id_hash = index.to_le_bytes().to_vec();
                        internal_id_hash.extend_from_slice(tx_hash.as_bytes());
                        drop_mutability!(internal_id_hash);

                        let internal_id = H256::from(internal_id_hash.as_slice()).reversed().to_vec().into();

                        if let Ok(Some(_)) = storage
                            .get_tx_from_history(&coin.history_wallet_id(), &internal_id)
                            .await
                        {
                            log::debug!("Tx '{}' already exists in tx_history. Skipping it.", &tx_hash);
                            continue;
                        }

                        let tx_sent_by_me = address == transfer_details.from;
                        let is_platform_coin_tx = transfer_details.denom == coin.platform_denom().to_string();
                        let is_self_tx = transfer_details.to == transfer_details.from && tx_sent_by_me;
                        let is_sign_claim_htlc = tx_sent_by_me
                            && matches!(transfer_details.transfer_event_type, TransferEventType::ClaimHtlc);

                        let (from, to) =
                            some_or_continue!(get_pair_addresses(address.clone(), tx_sent_by_me, transfer_details));

                        let maybe_add_fees = if !fee_added
                        // if tx is platform coin tx and sent by me
                            && is_platform_coin_tx && tx_sent_by_me
                        {
                            fee_added = true;
                            Some(&fee_details)
                        } else {
                            None
                        };

                        let tx_amounts = get_tx_amounts(
                            transfer_details,
                            is_self_tx,
                            tx_sent_by_me,
                            is_sign_claim_htlc,
                            maybe_add_fees,
                        );

                        let token_id: Option<BytesJson> = match !is_platform_coin_tx {
                            true => {
                                let denom_hash = sha256(transfer_details.denom.clone().as_bytes());
                                Some(H256::from(denom_hash.as_slice()).to_vec().into())
                            },
                            false => None,
                        };

                        let transaction_type = get_transaction_type(
                            &transfer_details.transfer_event_type,
                            token_id.clone(),
                            is_sign_claim_htlc,
                        );

                        let details = TransactionDetails {
                            from,
                            to,
                            total_amount: tx_amounts.total,
                            spent_by_me: tx_amounts.spent_by_me,
                            received_by_me: tx_amounts.received_by_me,
                            // This can be 0 since it gets remapped in `coins::my_tx_history_v2`
                            my_balance_change: BigDecimal::default(),
                            tx_hash: tx_hash.to_string(),
                            tx_hex: msg.into(),
                            fee_details: Some(TxFeeDetails::Tendermint(fee_details.clone())),
                            block_height: tx.height.into(),
                            coin: transfer_details.denom.clone(),
                            internal_id,
                            timestamp,
                            kmd_rewards: None,
                            transaction_type,
                            memo: Some(deserialized_tx.body.memo.clone()),
                        };
                        tx_details.push(details.clone());

                        // Display fees as extra transactions for asset txs sent by user
                        if tx_sent_by_me && !fee_added && !is_platform_coin_tx {
                            let fee_details = fee_details.clone();
                            let mut fee_tx_details = details;
                            fee_tx_details.to = vec![];
                            fee_tx_details.total_amount = fee_details.amount.clone();
                            fee_tx_details.spent_by_me = fee_details.amount.clone();
                            fee_tx_details.received_by_me = BigDecimal::default();
                            fee_tx_details.my_balance_change = BigDecimal::default() - &fee_details.amount;
                            fee_tx_details.coin = coin.platform_ticker().to_string();
                            // Non-reversed version of original internal id
                            fee_tx_details.internal_id = H256::from(internal_id_hash.as_slice()).to_vec().into();
                            fee_tx_details.transaction_type = TransactionType::FeeForTokenTx;

                            tx_details.push(fee_tx_details);
                            fee_added = true;
                        }
                    }

                    log::debug!("Tx '{}' successfully parsed.", tx.hash);
                }

                try_or_return_stopped_as_err!(
                    storage
                        .add_transactions_to_history(&coin.history_wallet_id(), tx_details)
                        .await
                        .map_err(|e| format!("{:?}", e)),
                    StopReason::StorageError,
                    "add_transactions_to_history failed"
                );

                if (*page * TX_PAGE_SIZE as u32) >= response.total_count {
                    // if last page is full, we can start with next page on next iteration
                    if current_page_is_full {
                        *page += 1;
                    }
                    break Ok(highest_height);
                }
                *page += 1;
            }
        }

        let q = format!("coin_spent.spender = '{}'", self.address);
        let highest_send_tx_height = match fetch_and_insert_txs(
            self.address.clone(),
            &ctx.coin,
            &ctx.storage,
            q,
            self.from_block_height,
            &mut ctx.last_spent_page,
        )
        .await
        {
            Ok(block) => block,
            Err(stopped) => {
                if let StopReason::RpcClient(e) = &stopped.stop_reason {
                    log::error!("Sent tx history process turned into cooldown mode due to rpc error: {e}");
                    return Self::change_state(OnIoErrorCooldown::new(self.address.clone(), self.from_block_height));
                }

                return Self::change_state(stopped);
            },
        };

        let q = format!("coin_received.receiver = '{}'", self.address);
        let highest_received_tx_height = match fetch_and_insert_txs(
            self.address.clone(),
            &ctx.coin,
            &ctx.storage,
            q,
            self.from_block_height,
            &mut ctx.last_received_page,
        )
        .await
        {
            Ok(block) => block,
            Err(stopped) => {
                if let StopReason::RpcClient(e) = &stopped.stop_reason {
                    log::error!("Received tx history process turned into cooldown mode due to rpc error: {e}");
                    return Self::change_state(OnIoErrorCooldown::new(self.address.clone(), self.from_block_height));
                }

                return Self::change_state(stopped);
            },
        };

        let last_fetched_block = cmp::max(highest_send_tx_height, highest_received_tx_height);

        log::info!(
            "Tx history fetching finished for {}. Last fetched block {}",
            ctx.coin.platform_ticker(),
            last_fetched_block
        );

        ctx.coin.set_history_sync_state(HistorySyncState::Finished);
        Self::change_state(WaitForHistoryUpdateTrigger::new(
            self.address.clone(),
            last_fetched_block,
        ))
    }
}

#[async_trait]
impl<Coin, Storage> State for TendermintInit<Coin, Storage>
where
    Coin: CoinCapabilities,
    Storage: TxHistoryStorage,
{
    type Ctx = TendermintTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        const INITIAL_SEARCH_HEIGHT: u64 = 0;

        ctx.coin.set_history_sync_state(HistorySyncState::NotStarted);

        if let Err(e) = ctx.storage.init(&ctx.coin.history_wallet_id()).await {
            return Self::change_state(Stopped::storage_error(e));
        }

        let search_from = match ctx
            .storage
            .get_highest_block_height(&ctx.coin.history_wallet_id())
            .await
        {
            Ok(Some(height)) if height > 0 => height as u64 - 1,
            _ => INITIAL_SEARCH_HEIGHT,
        };

        Self::change_state(FetchingTransactionsData::new(
            ctx.coin.my_address().expect("my_address can't fail"),
            search_from,
        ))
    }
}

#[async_trait]
impl<Coin, Storage> LastState for Stopped<Coin, Storage>
where
    Coin: CoinCapabilities,
    Storage: TxHistoryStorage,
{
    type Ctx = TendermintTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> Self::Result {
        log::info!(
            "Stopping tx history fetching for {}. Reason: {:?}",
            ctx.coin.ticker(),
            self.stop_reason
        );

        let new_state_json = json!({
            "message": format!("{:?}", self.stop_reason),
        });

        ctx.coin.set_history_sync_state(HistorySyncState::Error(new_state_json));
    }
}

pub async fn tendermint_history_loop(
    coin: TendermintCoin,
    storage: impl TxHistoryStorage,
    _ctx: MmArc,
    _current_balance: Option<BigDecimal>,
) {
    let balances = match coin.all_balances().await {
        Ok(balances) => balances,
        Err(e) => {
            log::error!("{}", e);
            return;
        },
    };

    let ctx = TendermintTxHistoryCtx {
        coin,
        storage,
        balances,
        last_received_page: 1,
        last_spent_page: 1,
    };

    let state_machine: StateMachine<_, ()> = StateMachine::from_ctx(ctx);
    state_machine.run(TendermintInit::new()).await;
}

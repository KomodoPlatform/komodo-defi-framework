use crate::coin_errors::MyAddressError;
use crate::my_tx_history_v2::{MyTxHistoryErrorV2, MyTxHistoryRequestV2, MyTxHistoryResponseV2};
use crate::rpc_command::init_withdraw::{InitWithdrawCoin, WithdrawInProgressStatus, WithdrawTaskHandle};
use crate::utxo::rpc_clients::{ElectrumRpcRequest, UnspentInfo, UtxoRpcClientEnum, UtxoRpcError, UtxoRpcFut,
                               UtxoRpcResult};
use crate::utxo::utxo_builder::{UtxoCoinBuildError, UtxoCoinBuilder, UtxoCoinBuilderCommonOps,
                                UtxoFieldsWithGlobalHDBuilder, UtxoFieldsWithHardwareWalletBuilder,
                                UtxoFieldsWithIguanaSecretBuilder};
use crate::utxo::utxo_common::{addresses_from_script, big_decimal_from_sat, big_decimal_from_sat_unsigned,
                               payment_script};
use crate::utxo::{sat_from_big_decimal, utxo_common, ActualTxFee, AdditionalTxData, AddrFromStrError, Address,
                  BroadcastTxErr, FeePolicy, GetUtxoListOps, HistoryUtxoTx, HistoryUtxoTxMap, MatureUnspentList,
                  RecentlySpentOutPointsGuard, UtxoActivationParams, UtxoAddressFormat, UtxoArc, UtxoCoinFields,
                  UtxoCommonOps, UtxoFeeDetails, UtxoRpcMode, UtxoTxBroadcastOps, UtxoTxGenerationOps,
                  VerboseTransactionFrom};
use crate::{BalanceError, BalanceFut, CheckIfMyPaymentSentArgs, CoinBalance, CoinFutSpawner, FeeApproxStage,
            FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin, NegotiateSwapContractAddrErr, NumConversError,
            PaymentInstructions, PaymentInstructionsErr, PrivKeyActivationPolicy, PrivKeyBuildPolicy,
            PrivKeyPolicyNotAllowed, RawTransactionFut, RawTransactionRequest, SearchForSwapTxSpendInput,
            SendMakerPaymentArgs, SendMakerRefundsPaymentArgs, SendMakerSpendsTakerPaymentArgs, SendTakerPaymentArgs,
            SendTakerRefundsPaymentArgs, SendTakerSpendsMakerPaymentArgs, SignatureError, SignatureResult, SwapOps,
            TradeFee, TradePreimageFut, TradePreimageResult, TradePreimageValue, TransactionDetails, TransactionEnum,
            TransactionFut, TxFeeDetails, TxMarshalingErr, UnexpectedDerivationMethod, ValidateAddressResult,
            ValidateFeeArgs, ValidateInstructionsErr, ValidateOtherPubKeyErr, ValidatePaymentError,
            ValidatePaymentFut, ValidatePaymentInput, VerificationError, VerificationResult, WatcherOps,
            WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput, WatcherValidateTakerFeeInput, WithdrawFut,
            WithdrawRequest};
use crate::{Transaction, WithdrawError};
use async_trait::async_trait;
use bitcrypto::dhash256;
use chain::constants::SEQUENCE_FINAL;
use chain::{Transaction as UtxoTx, TransactionOutput};
use common::executor::{AbortableSystem, AbortedError};
use common::sha256_digest;
use common::{async_blocking, calc_total_pages, log, PagingOptionsEnum};
use crypto::privkey::{key_pair_from_secret, secp_privkey_from_hash};
use crypto::{Bip32DerPathOps, GlobalHDAccountArc, StandardHDPathToCoin};
use db_common::sqlite::offset_by_id;
use db_common::sqlite::rusqlite::{Error as SqlError, Row, NO_PARAMS};
use db_common::sqlite::sql_builder::{name, SqlBuilder, SqlName};
use futures::compat::Future01CompatExt;
use futures::lock::Mutex as AsyncMutex;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::hash::H256;
use keys::{KeyPair, Message, Public};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumber};
#[cfg(test)] use mocktopus::macros::*;
use parking_lot::Mutex;
use primitives::bytes::Bytes;
use rpc::v1::types::{Bytes as BytesJson, Transaction as RpcTransaction, H256 as H256Json};
use script::{Builder as ScriptBuilder, Opcode, Script, TransactionInputSigner};
use serde_json::Value as Json;
use serialization::CoinVariant;
use std::collections::{HashMap, HashSet};
use std::iter;
use std::path::PathBuf;
use std::sync::Arc;
use zcash_client_backend::data_api::WalletRead;
use zcash_client_backend::encoding::{decode_payment_address, encode_extended_spending_key, encode_payment_address};
use zcash_client_backend::wallet::{AccountId, SpendableNote};
use zcash_client_sqlite::error::SqliteClientError as ZcashClientError;
use zcash_client_sqlite::error::SqliteClientError;
use zcash_client_sqlite::wallet::get_balance;
use zcash_client_sqlite::wallet::transact::get_spendable_notes;
use zcash_primitives::consensus::{BlockHeight, NetworkUpgrade, Parameters, H0};
use zcash_primitives::memo::MemoBytes;
use zcash_primitives::sapling::keys::OutgoingViewingKey;
use zcash_primitives::sapling::note_encryption::try_sapling_output_recovery;
use zcash_primitives::transaction::builder::Builder as ZTxBuilder;
use zcash_primitives::transaction::components::{Amount, TxOut};
use zcash_primitives::transaction::Transaction as ZTransaction;
use zcash_primitives::zip32::ChildIndex as Zip32Child;
use zcash_primitives::{consensus, constants::mainnet as z_mainnet_constants, sapling::PaymentAddress,
                       zip32::ExtendedFullViewingKey, zip32::ExtendedSpendingKey};
use zcash_proofs::default_params_folder;
use zcash_proofs::prover::LocalTxProver;

mod z_htlc;
use z_htlc::{z_p2sh_spend, z_send_dex_fee, z_send_htlc};

mod z_rpc;
pub use z_rpc::SyncStatus;
use z_rpc::{init_light_client, init_native_client, SaplingSyncConnector, SaplingSyncGuard, WalletDbShared};

mod z_coin_errors;
use crate::z_coin::z_rpc::{create_wallet_db, BlockDb};
pub use z_coin_errors::*;

#[cfg(all(test, feature = "zhtlc-native-tests"))]
mod z_coin_native_tests;

/// `ZP2SHSpendError` compatible `TransactionErr` handling macro.
macro_rules! try_ztx_s {
    ($e: expr) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => {
                if let Some(tx) = err.get_inner().get_tx() {
                    return Err(crate::TransactionErr::TxRecoverable(
                        tx,
                        format!("{}:{}] {:?}", file!(), line!(), err),
                    ));
                }

                return Err(crate::TransactionErr::Plain(ERRL!("{:?}", err)));
            },
        }
    };
}

const DEX_FEE_OVK: OutgoingViewingKey = OutgoingViewingKey([7; 32]);
const DEX_FEE_Z_ADDR: &str = "zs1rp6426e9r6jkq2nsanl66tkd34enewrmr0uvj0zelhkcwmsy0uvxz2fhm9eu9rl3ukxvgzy2v9f";
const TRANSACTIONS_TABLE: &str = "transactions";
const BLOCKS_TABLE: &str = "blocks";
const SAPLING_SPEND_NAME: &str = "sapling-spend.params";
const SAPLING_OUTPUT_NAME: &str = "sapling-output.params";
const SAPLING_SPEND_EXPECTED_HASH: &str = "8e48ffd23abb3a5fd9c5589204f32d9c31285a04b78096ba40a79b75677efc13";
const SAPLING_OUTPUT_EXPECTED_HASH: &str = "2f0ebbcbb9bb0bcffe95a397e7eba89c29eb4dde6191c339db88570e3f3fb0e4";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZcoinConsensusParams {
    // we don't support coins without overwinter and sapling active so these are mandatory
    overwinter_activation_height: u32,
    sapling_activation_height: u32,
    // optional upgrades that we will possibly support in the future
    blossom_activation_height: Option<u32>,
    heartwood_activation_height: Option<u32>,
    canopy_activation_height: Option<u32>,
    coin_type: u32,
    hrp_sapling_extended_spending_key: String,
    hrp_sapling_extended_full_viewing_key: String,
    hrp_sapling_payment_address: String,
    b58_pubkey_address_prefix: [u8; 2],
    b58_script_address_prefix: [u8; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckPointBlockInfo {
    height: u32,
    hash: H256Json,
    time: u32,
    sapling_tree: BytesJson,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZcoinProtocolInfo {
    consensus_params: ZcoinConsensusParams,
    check_point_block: Option<CheckPointBlockInfo>,
    // `z_derivation_path` can be the same or different from [`UtxoCoinFields::derivation_path`].
    z_derivation_path: Option<StandardHDPathToCoin>,
}

impl Parameters for ZcoinConsensusParams {
    fn activation_height(&self, nu: NetworkUpgrade) -> Option<BlockHeight> {
        match nu {
            NetworkUpgrade::Overwinter => Some(BlockHeight::from(self.overwinter_activation_height)),
            NetworkUpgrade::Sapling => Some(BlockHeight::from(self.sapling_activation_height)),
            NetworkUpgrade::Blossom => self.blossom_activation_height.map(BlockHeight::from),
            NetworkUpgrade::Heartwood => self.heartwood_activation_height.map(BlockHeight::from),
            NetworkUpgrade::Canopy => self.canopy_activation_height.map(BlockHeight::from),
        }
    }

    fn coin_type(&self) -> u32 { self.coin_type }

    fn hrp_sapling_extended_spending_key(&self) -> &str { &self.hrp_sapling_extended_spending_key }

    fn hrp_sapling_extended_full_viewing_key(&self) -> &str { &self.hrp_sapling_extended_full_viewing_key }

    fn hrp_sapling_payment_address(&self) -> &str { &self.hrp_sapling_payment_address }

    fn b58_pubkey_address_prefix(&self) -> [u8; 2] { self.b58_pubkey_address_prefix }

    fn b58_script_address_prefix(&self) -> [u8; 2] { self.b58_script_address_prefix }
}

pub struct ZCoinFields {
    dex_fee_addr: PaymentAddress,
    my_z_addr: PaymentAddress,
    my_z_addr_encoded: String,
    z_spending_key: ExtendedSpendingKey,
    evk: ExtendedFullViewingKey,
    z_tx_prover: Arc<LocalTxProver>,
    light_wallet_db: WalletDbShared,
    consensus_params: ZcoinConsensusParams,
    sync_state_connector: AsyncMutex<SaplingSyncConnector>,
}

impl Transaction for ZTransaction {
    fn tx_hex(&self) -> Vec<u8> {
        let mut hex = Vec::with_capacity(1024);
        self.write(&mut hex).expect("Writing should not fail");
        hex
    }

    fn tx_hash(&self) -> BytesJson {
        let mut bytes = self.txid().0.to_vec();
        bytes.reverse();
        bytes.into()
    }
}

#[derive(Clone)]
pub struct ZCoin {
    utxo_arc: UtxoArc,
    z_fields: Arc<ZCoinFields>,
}

pub struct ZOutput {
    pub to_addr: PaymentAddress,
    pub amount: Amount,
    pub viewing_key: Option<OutgoingViewingKey>,
    pub memo: Option<MemoBytes>,
}

struct ZCoinSqlTxHistoryItem {
    tx_hash: Vec<u8>,
    internal_id: i64,
    height: i64,
    timestamp: i64,
    received_amount: i64,
    spent_amount: i64,
}

impl ZCoinSqlTxHistoryItem {
    fn try_from_sql_row(row: &Row<'_>) -> Result<Self, SqlError> {
        let mut tx_hash: Vec<u8> = row.get(0)?;
        tx_hash.reverse();
        Ok(ZCoinSqlTxHistoryItem {
            tx_hash,
            internal_id: row.get(1)?,
            height: row.get(2)?,
            timestamp: row.get(3)?,
            received_amount: row.get(4)?,
            spent_amount: row.get(5)?,
        })
    }
}

struct SqlTxHistoryRes {
    transactions: Vec<ZCoinSqlTxHistoryItem>,
    total_tx_count: u32,
    skipped: usize,
}

#[derive(Serialize)]
pub struct ZcoinTxDetails {
    /// Transaction hash in hexadecimal format
    tx_hash: String,
    /// Coins are sent from these addresses
    from: HashSet<String>,
    /// Coins are sent to these addresses
    to: HashSet<String>,
    /// The amount spent from "my" address
    spent_by_me: BigDecimal,
    /// The amount received by "my" address
    received_by_me: BigDecimal,
    /// Resulting "my" balance change
    my_balance_change: BigDecimal,
    /// Block height
    block_height: i64,
    confirmations: i64,
    /// Transaction timestamp
    timestamp: i64,
    transaction_fee: BigDecimal,
    /// The coin transaction belongs to
    coin: String,
    /// Internal MM2 id used for internal transaction identification, for some coins it might be equal to transaction hash
    internal_id: i64,
}

impl ZCoin {
    #[inline]
    pub fn utxo_rpc_client(&self) -> &UtxoRpcClientEnum { &self.utxo_arc.rpc_client }

    #[inline]
    pub fn my_z_address_encoded(&self) -> String { self.z_fields.my_z_addr_encoded.clone() }

    #[inline]
    pub fn consensus_params(&self) -> ZcoinConsensusParams { self.z_fields.consensus_params.clone() }

    #[inline]
    pub fn consensus_params_ref(&self) -> &ZcoinConsensusParams { &self.z_fields.consensus_params }

    #[inline]
    pub async fn sync_status(&self) -> Result<SyncStatus, MmError<BlockchainScanStopped>> {
        self.z_fields
            .sync_state_connector
            .lock()
            .await
            .current_sync_status()
            .await
    }

    #[inline]
    fn secp_keypair(&self) -> &KeyPair {
        self.utxo_arc
            .priv_key_policy
            .key_pair()
            .expect("Zcoin doesn't support HW wallets")
    }

    async fn wait_for_gen_tx_blockchain_sync(&self) -> Result<SaplingSyncGuard<'_>, MmError<BlockchainScanStopped>> {
        let mut connector_guard = self.z_fields.sync_state_connector.lock().await;
        let sync_respawn_guard = connector_guard.wait_for_gen_tx_blockchain_sync().await?;
        Ok(SaplingSyncGuard {
            _connector_guard: connector_guard,
            respawn_guard: sync_respawn_guard,
        })
    }

    async fn my_balance_sat(&self) -> Result<u64, MmError<ZcashClientError>> {
        let db = self.z_fields.light_wallet_db.clone();
        async_blocking(move || {
            let balance = get_balance(&db.lock(), AccountId::default())?.into();
            Ok(balance)
        })
        .await
    }

    async fn get_spendable_notes(&self) -> Result<Vec<SpendableNote>, MmError<ZcashClientError>> {
        let db = self.z_fields.light_wallet_db.clone();
        async_blocking(move || {
            let guard = db.lock();
            let latest_db_block = match guard.block_height_extrema()? {
                Some((_, latest)) => latest,
                None => return Ok(Vec::new()),
            };
            get_spendable_notes(&guard, AccountId::default(), latest_db_block).map_err(MmError::new)
        })
        .await
    }

    /// Returns spendable notes
    async fn spendable_notes_ordered(&self) -> Result<Vec<SpendableNote>, MmError<SqliteClientError>> {
        let mut unspents = self.get_spendable_notes().await?;

        unspents.sort_unstable_by(|a, b| a.note_value.cmp(&b.note_value));
        Ok(unspents)
    }

    async fn get_one_kbyte_tx_fee(&self) -> UtxoRpcResult<BigDecimal> {
        let fee = self.get_tx_fee().await?;
        match fee {
            ActualTxFee::Dynamic(fee) | ActualTxFee::FixedPerKb(fee) => {
                Ok(big_decimal_from_sat_unsigned(fee, self.decimals()))
            },
        }
    }

    /// Generates a tx sending outputs from our address
    async fn gen_tx(
        &self,
        t_outputs: Vec<TxOut>,
        z_outputs: Vec<ZOutput>,
    ) -> Result<(ZTransaction, AdditionalTxData, SaplingSyncGuard<'_>), MmError<GenTxError>> {
        let sync_guard = self.wait_for_gen_tx_blockchain_sync().await?;

        let tx_fee = self.get_one_kbyte_tx_fee().await?;
        let t_output_sat: u64 = t_outputs.iter().fold(0, |cur, out| cur + u64::from(out.value));
        let z_output_sat: u64 = z_outputs.iter().fold(0, |cur, out| cur + u64::from(out.amount));
        let total_output_sat = t_output_sat + z_output_sat;
        let total_output = big_decimal_from_sat_unsigned(total_output_sat, self.utxo_arc.decimals);
        let total_required = &total_output + &tx_fee;

        let spendable_notes = self.spendable_notes_ordered().await?;
        let mut total_input_amount = BigDecimal::from(0);
        let mut change = BigDecimal::from(0);

        let mut received_by_me = 0u64;

        let mut tx_builder = ZTxBuilder::new(self.consensus_params(), sync_guard.respawn_guard.current_block());

        for spendable_note in spendable_notes {
            total_input_amount += big_decimal_from_sat_unsigned(spendable_note.note_value.into(), self.decimals());

            let note = self
                .z_fields
                .my_z_addr
                .create_note(spendable_note.note_value.into(), spendable_note.rseed)
                .or_mm_err(|| GenTxError::FailedToCreateNote)?;
            tx_builder.add_sapling_spend(
                self.z_fields.z_spending_key.clone(),
                *self.z_fields.my_z_addr.diversifier(),
                note,
                spendable_note
                    .witness
                    .path()
                    .or_mm_err(|| GenTxError::FailedToGetMerklePath)?,
            )?;

            if total_input_amount >= total_required {
                change = &total_input_amount - &total_required;
                break;
            }
        }

        if total_input_amount < total_required {
            return MmError::err(GenTxError::InsufficientBalance {
                coin: self.ticker().into(),
                available: total_input_amount,
                required: total_required,
            });
        }

        for z_out in z_outputs {
            if z_out.to_addr == self.z_fields.my_z_addr {
                received_by_me += u64::from(z_out.amount);
            }

            tx_builder.add_sapling_output(z_out.viewing_key, z_out.to_addr, z_out.amount, z_out.memo)?;
        }

        if change > BigDecimal::from(0u8) {
            let change_sat = sat_from_big_decimal(&change, self.utxo_arc.decimals)?;
            received_by_me += change_sat;

            tx_builder.add_sapling_output(
                Some(self.z_fields.evk.fvk.ovk),
                self.z_fields.my_z_addr.clone(),
                Amount::from_u64(change_sat).map_to_mm(|_| {
                    GenTxError::NumConversion(NumConversError(format!(
                        "Failed to get ZCash amount from {}",
                        change_sat
                    )))
                })?,
                None,
            )?;
        }

        for output in t_outputs {
            tx_builder.add_tx_out(output);
        }

        let (tx, _) = async_blocking({
            let prover = self.z_fields.z_tx_prover.clone();
            move || tx_builder.build(consensus::BranchId::Sapling, prover.as_ref())
        })
        .await?;

        let additional_data = AdditionalTxData {
            received_by_me,
            spent_by_me: sat_from_big_decimal(&total_input_amount, self.decimals())?,
            fee_amount: sat_from_big_decimal(&tx_fee, self.decimals())?,
            unused_change: None,
            kmd_rewards: None,
        };
        Ok((tx, additional_data, sync_guard))
    }

    pub async fn send_outputs(
        &self,
        t_outputs: Vec<TxOut>,
        z_outputs: Vec<ZOutput>,
    ) -> Result<ZTransaction, MmError<SendOutputsErr>> {
        let (tx, _, mut sync_guard) = self.gen_tx(t_outputs, z_outputs).await?;
        let mut tx_bytes = Vec::with_capacity(1024);
        tx.write(&mut tx_bytes).expect("Write should not fail");

        self.utxo_rpc_client()
            .send_raw_transaction(tx_bytes.into())
            .compat()
            .await?;

        sync_guard.respawn_guard.watch_for_tx(tx.txid());
        Ok(tx)
    }

    async fn tx_history_from_sql(
        &self,
        limit: usize,
        paging_options: PagingOptionsEnum<i64>,
    ) -> Result<SqlTxHistoryRes, MmError<SqlTxHistoryError>> {
        let wallet_db = self.z_fields.light_wallet_db.clone();
        async_blocking(move || {
            let db_guard = wallet_db.lock();
            let conn = db_guard.sql_conn();

            let total_sql = SqlBuilder::select_from(TRANSACTIONS_TABLE)
                .field("COUNT(id_tx)")
                .sql()
                .expect("valid SQL");
            let total_tx_count = conn.query_row(&total_sql, NO_PARAMS, |row| row.get(0))?;

            let mut sql_builder = SqlBuilder::select_from(name!(TRANSACTIONS_TABLE; "txes"));
            sql_builder
                .field("txes.txid")
                .field("txes.id_tx as internal_id")
                .field("txes.block as block");

            let offset = match paging_options {
                PagingOptionsEnum::PageNumber(page) => (page.get() - 1) * limit,
                PagingOptionsEnum::FromId(id) => {
                    offset_by_id(conn, &sql_builder, [id], "id_tx", "block DESC, id_tx ASC", "id_tx = ?1")?
                        .ok_or(SqlTxHistoryError::FromIdDoesNotExist(id))?
                },
            };

            let sql = sql_builder
                .field("blocks.time")
                .field("COALESCE(rn.received_amount, 0)")
                .field("COALESCE(sn.sent_amount, 0)")
                .left()
                .join("(SELECT tx, SUM(value) as received_amount FROM received_notes GROUP BY tx) as rn")
                .on("txes.id_tx = rn.tx")
                // detecting spent amount by "spent" field in received_notes table
                .join("(SELECT spent, SUM(value) as sent_amount FROM received_notes GROUP BY spent) as sn")
                .on("txes.id_tx = sn.spent")
                .join(BLOCKS_TABLE)
                .on("txes.block = blocks.height")
                .group_by("internal_id")
                .order_by("block", true)
                .order_by("internal_id", false)
                .offset(offset)
                .limit(limit)
                .sql()
                .expect("valid query");

            let sql_items = conn
                .prepare(&sql)?
                .query_map(NO_PARAMS, ZCoinSqlTxHistoryItem::try_from_sql_row)?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(SqlTxHistoryRes {
                transactions: sql_items,
                total_tx_count,
                skipped: offset,
            })
        })
        .await
    }

    async fn z_transactions_from_cache_or_rpc(
        &self,
        hashes: HashSet<H256Json>,
    ) -> UtxoRpcResult<HashMap<H256Json, ZTransaction>> {
        self.get_verbose_transactions_from_cache_or_rpc(hashes)
            .compat()
            .await?
            .into_iter()
            .map(|(hash, tx)| -> Result<_, std::io::Error> {
                Ok((hash, ZTransaction::read(tx.into_inner().hex.as_slice())?))
            })
            .collect::<Result<_, _>>()
            .map_to_mm(|e| UtxoRpcError::InvalidResponse(e.to_string()))
    }

    fn tx_details_from_sql_item(
        &self,
        sql_item: ZCoinSqlTxHistoryItem,
        transactions: &mut HashMap<H256Json, ZTransaction>,
        prev_transactions: &HashMap<H256Json, ZTransaction>,
        current_block: u64,
    ) -> Result<ZcoinTxDetails, MmError<NoInfoAboutTx>> {
        let mut from = HashSet::new();

        let mut confirmations = current_block as i64 - sql_item.height + 1;
        if confirmations < 0 {
            confirmations = 0;
        }

        let mut transparent_input_amount = Amount::zero();
        let hash = H256Json::from(sql_item.tx_hash.as_slice());
        let z_tx = transactions.remove(&hash).or_mm_err(|| NoInfoAboutTx(hash))?;
        for input in z_tx.vin.iter() {
            let mut hash = H256Json::from(*input.prevout.hash());
            hash.0.reverse();
            let prev_tx = prev_transactions.get(&hash).or_mm_err(|| NoInfoAboutTx(hash))?;

            if let Some(spent_output) = prev_tx.vout.get(input.prevout.n() as usize) {
                transparent_input_amount += spent_output.value;
                if let Ok(addresses) = addresses_from_script(self, &spent_output.script_pubkey.0.clone().into()) {
                    from.extend(addresses.into_iter().map(|a| a.to_string()));
                }
            }
        }

        let transparent_output_amount = z_tx
            .vout
            .iter()
            .fold(Amount::zero(), |current, out| current + out.value);

        let mut to = HashSet::new();
        for out in z_tx.vout.iter() {
            if let Ok(addresses) = addresses_from_script(self, &out.script_pubkey.0.clone().into()) {
                to.extend(addresses.into_iter().map(|a| a.to_string()));
            }
        }

        let fee_amount = z_tx.value_balance + transparent_input_amount - transparent_output_amount;
        if sql_item.spent_amount > 0 {
            from.insert(self.my_z_address_encoded());
        }

        if sql_item.received_amount > 0 {
            to.insert(self.my_z_address_encoded());
        }

        for z_out in z_tx.shielded_outputs.iter() {
            if let Some((_, address, _)) = try_sapling_output_recovery(
                self.consensus_params_ref(),
                BlockHeight::from_u32(current_block as u32),
                &self.z_fields.evk.fvk.ovk,
                z_out,
            ) {
                to.insert(encode_payment_address(
                    self.consensus_params_ref().hrp_sapling_payment_address(),
                    &address,
                ));
            }

            if let Some((_, address, _)) = try_sapling_output_recovery(
                self.consensus_params_ref(),
                BlockHeight::from_u32(current_block as u32),
                &DEX_FEE_OVK,
                z_out,
            ) {
                to.insert(encode_payment_address(
                    self.consensus_params_ref().hrp_sapling_payment_address(),
                    &address,
                ));
            }
        }

        let spent_by_me = big_decimal_from_sat(sql_item.spent_amount, self.decimals());
        let received_by_me = big_decimal_from_sat(sql_item.received_amount, self.decimals());
        Ok(ZcoinTxDetails {
            tx_hash: hex::encode(sql_item.tx_hash),
            from,
            to,
            my_balance_change: &received_by_me - &spent_by_me,
            spent_by_me,
            received_by_me,
            block_height: sql_item.height,
            confirmations,
            timestamp: sql_item.timestamp,
            transaction_fee: big_decimal_from_sat(fee_amount.into(), self.decimals()),
            coin: self.ticker().into(),
            internal_id: sql_item.internal_id,
        })
    }

    pub async fn tx_history(
        &self,
        request: MyTxHistoryRequestV2<i64>,
    ) -> Result<MyTxHistoryResponseV2<ZcoinTxDetails, i64>, MmError<MyTxHistoryErrorV2>> {
        let current_block = self.utxo_rpc_client().get_block_count().compat().await?;
        let sql_result = self
            .tx_history_from_sql(request.limit, request.paging_options.clone())
            .await?;

        let hashes_for_verbose = sql_result
            .transactions
            .iter()
            .map(|item| H256Json::from(item.tx_hash.as_slice()))
            .collect();
        let mut transactions = self.z_transactions_from_cache_or_rpc(hashes_for_verbose).await?;

        let prev_tx_hashes: HashSet<_> = transactions
            .iter()
            .flat_map(|(_, tx)| {
                tx.vin.iter().map(|vin| {
                    let mut hash = *vin.prevout.hash();
                    hash.reverse();
                    H256Json::from(hash)
                })
            })
            .collect();
        let prev_transactions = self.z_transactions_from_cache_or_rpc(prev_tx_hashes).await?;

        let transactions = sql_result
            .transactions
            .into_iter()
            .map(|sql_item| {
                self.tx_details_from_sql_item(sql_item, &mut transactions, &prev_transactions, current_block)
            })
            .collect::<Result<_, _>>()?;

        Ok(MyTxHistoryResponseV2 {
            coin: self.ticker().into(),
            target: request.target,
            current_block,
            transactions,
            // Zcoin is activated only after the state is synced
            sync_status: HistorySyncState::Finished,
            limit: request.limit,
            skipped: sql_result.skipped,
            total: sql_result.total_tx_count as usize,
            total_pages: calc_total_pages(sql_result.total_tx_count as usize, request.limit),
            paging_options: request.paging_options,
        })
    }
}

impl AsRef<UtxoCoinFields> for ZCoin {
    fn as_ref(&self) -> &UtxoCoinFields { &self.utxo_arc }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "rpc", content = "rpc_data")]
pub enum ZcoinRpcMode {
    Native,
    Light {
        electrum_servers: Vec<ElectrumRpcRequest>,
        light_wallet_d_servers: Vec<String>,
    },
}

#[derive(Clone, Deserialize)]
pub struct ZcoinActivationParams {
    pub mode: ZcoinRpcMode,
    pub required_confirmations: Option<u64>,
    pub requires_notarization: Option<bool>,
    pub zcash_params_path: Option<String>,
}

pub async fn z_coin_from_conf_and_params(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    params: &ZcoinActivationParams,
    protocol_info: ZcoinProtocolInfo,
    priv_key_policy: PrivKeyBuildPolicy,
) -> Result<ZCoin, MmError<ZCoinBuildError>> {
    let db_dir_path = ctx.dbdir();
    let z_spending_key = None;
    let builder = ZCoinBuilder::new(
        ctx,
        ticker,
        conf,
        params,
        priv_key_policy,
        db_dir_path,
        z_spending_key,
        protocol_info,
    );
    builder.build().await
}

fn verify_checksum_zcash_params(spend_path: &PathBuf, output_path: &PathBuf) -> Result<bool, ZCoinBuildError> {
    let spend_hash = sha256_digest(spend_path)?;
    let out_hash = sha256_digest(output_path)?;
    Ok(spend_hash == SAPLING_SPEND_EXPECTED_HASH && out_hash == SAPLING_OUTPUT_EXPECTED_HASH)
}

fn get_spend_output_paths(params_dir: PathBuf) -> Result<(PathBuf, PathBuf), ZCoinBuildError> {
    if !params_dir.exists() {
        return Err(ZCoinBuildError::ZCashParamsNotFound);
    };
    let spend_path = params_dir.join(SAPLING_SPEND_NAME);
    let output_path = params_dir.join(SAPLING_OUTPUT_NAME);

    if !(spend_path.exists() && output_path.exists()) {
        return Err(ZCoinBuildError::ZCashParamsNotFound);
    }
    Ok((spend_path, output_path))
}

pub struct ZCoinBuilder<'a> {
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    z_coin_params: &'a ZcoinActivationParams,
    utxo_params: UtxoActivationParams,
    priv_key_policy: PrivKeyBuildPolicy,
    db_dir_path: PathBuf,
    /// `Some` if `ZCoin` should be initialized with a forced spending key.
    z_spending_key: Option<ExtendedSpendingKey>,
    protocol_info: ZcoinProtocolInfo,
}

impl<'a> UtxoCoinBuilderCommonOps for ZCoinBuilder<'a> {
    fn ctx(&self) -> &MmArc { self.ctx }

    fn conf(&self) -> &Json { self.conf }

    fn activation_params(&self) -> &UtxoActivationParams { &self.utxo_params }

    fn ticker(&self) -> &str { self.ticker }
}

impl<'a> UtxoFieldsWithIguanaSecretBuilder for ZCoinBuilder<'a> {}

impl<'a> UtxoFieldsWithGlobalHDBuilder for ZCoinBuilder<'a> {}

/// Although, `ZCoin` doesn't support [`PrivKeyBuildPolicy::Trezor`] yet,
/// `UtxoCoinBuilder` trait requires `UtxoFieldsWithHardwareWalletBuilder` to be implemented.
impl<'a> UtxoFieldsWithHardwareWalletBuilder for ZCoinBuilder<'a> {}

#[async_trait]
impl<'a> UtxoCoinBuilder for ZCoinBuilder<'a> {
    type ResultCoin = ZCoin;
    type Error = ZCoinBuildError;

    fn priv_key_policy(&self) -> PrivKeyBuildPolicy { self.priv_key_policy.clone() }

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error> {
        let utxo = self.build_utxo_fields().await?;
        let utxo_arc = UtxoArc::new(utxo);

        let z_spending_key = match self.z_spending_key {
            Some(ref z_spending_key) => z_spending_key.clone(),
            None => extended_spending_key_from_protocol_info_and_policy(&self.protocol_info, &self.priv_key_policy)?,
        };

        let (_, my_z_addr) = z_spending_key
            .default_address()
            .map_err(|_| MmError::new(ZCoinBuildError::GetAddressError))?;

        let dex_fee_addr = decode_payment_address(
            self.protocol_info.consensus_params.hrp_sapling_payment_address(),
            DEX_FEE_Z_ADDR,
        )
        .expect("DEX_FEE_Z_ADDR is a valid z-address")
        .expect("DEX_FEE_Z_ADDR is a valid z-address");

        let params_dir = match &self.z_coin_params.zcash_params_path {
            None => default_params_folder().or_mm_err(|| ZCoinBuildError::ZCashParamsNotFound)?,
            Some(file_path) => PathBuf::from(file_path),
        };

        let z_tx_prover = async_blocking(move || {
            let (spend_path, output_path) = get_spend_output_paths(params_dir)?;
            let verification_successful = verify_checksum_zcash_params(&spend_path, &output_path)?;
            if verification_successful {
                Ok(LocalTxProver::new(&spend_path, &output_path))
            } else {
                MmError::err(ZCoinBuildError::SaplingParamsInvalidChecksum)
            }
        })
        .await?;

        let my_z_addr_encoded = encode_payment_address(
            self.protocol_info.consensus_params.hrp_sapling_payment_address(),
            &my_z_addr,
        );

        let evk = ExtendedFullViewingKey::from(&z_spending_key);
        let cache_db_path = self.db_dir_path.join(format!("{}_cache.db", self.ticker));
        let wallet_db_path = self.db_dir_path.join(format!("{}_wallet.db", self.ticker));
        let blocks_db =
            async_blocking(|| BlockDb::for_path(cache_db_path).map_to_mm(ZcoinClientInitError::BlocksDbInitFailure))
                .await?;
        let wallet_db = create_wallet_db(
            wallet_db_path,
            self.protocol_info.consensus_params.clone(),
            self.protocol_info.check_point_block.clone(),
            evk,
        )
        .await?;
        let wallet_db = Arc::new(Mutex::new(wallet_db));
        let (sync_state_connector, light_wallet_db) = match &self.z_coin_params.mode {
            ZcoinRpcMode::Native => {
                let native_client = self.native_client()?;
                init_native_client(
                    native_client,
                    blocks_db,
                    wallet_db,
                    self.protocol_info.consensus_params.clone(),
                )
                .await?
            },
            ZcoinRpcMode::Light {
                light_wallet_d_servers, ..
            } => {
                init_light_client(
                    light_wallet_d_servers.clone(),
                    blocks_db,
                    wallet_db,
                    self.protocol_info.consensus_params.clone(),
                )
                .await?
            },
        };

        let z_fields = ZCoinFields {
            dex_fee_addr,
            my_z_addr,
            my_z_addr_encoded,
            evk: ExtendedFullViewingKey::from(&z_spending_key),
            z_spending_key,
            z_tx_prover: Arc::new(z_tx_prover),
            light_wallet_db,
            consensus_params: self.protocol_info.consensus_params,
            sync_state_connector,
        };

        let z_coin = ZCoin {
            utxo_arc,
            z_fields: Arc::new(z_fields),
        };

        Ok(z_coin)
    }
}

impl<'a> ZCoinBuilder<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ctx: &'a MmArc,
        ticker: &'a str,
        conf: &'a Json,
        z_coin_params: &'a ZcoinActivationParams,
        priv_key_policy: PrivKeyBuildPolicy,
        db_dir_path: PathBuf,
        z_spending_key: Option<ExtendedSpendingKey>,
        protocol_info: ZcoinProtocolInfo,
    ) -> ZCoinBuilder<'a> {
        let utxo_mode = match &z_coin_params.mode {
            ZcoinRpcMode::Native => UtxoRpcMode::Native,
            ZcoinRpcMode::Light { electrum_servers, .. } => UtxoRpcMode::Electrum {
                servers: electrum_servers.clone(),
            },
        };
        let utxo_params = UtxoActivationParams {
            mode: utxo_mode,
            utxo_merge_params: None,
            tx_history: false,
            required_confirmations: z_coin_params.required_confirmations,
            requires_notarization: z_coin_params.requires_notarization,
            address_format: None,
            gap_limit: None,
            enable_params: Default::default(),
            priv_key_policy: PrivKeyActivationPolicy::ContextPrivKey,
            check_utxo_maturity: None,
        };
        ZCoinBuilder {
            ctx,
            ticker,
            conf,
            z_coin_params,
            utxo_params,
            priv_key_policy,
            db_dir_path,
            z_spending_key,
            protocol_info,
        }
    }
}

/// Initialize `ZCoin` with a forced `z_spending_key`.
#[cfg(all(test, feature = "zhtlc-native-tests"))]
#[allow(clippy::too_many_arguments)]
async fn z_coin_from_conf_and_params_with_z_key(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    params: &ZcoinActivationParams,
    priv_key_policy: PrivKeyBuildPolicy,
    db_dir_path: PathBuf,
    z_spending_key: ExtendedSpendingKey,
    protocol_info: ZcoinProtocolInfo,
) -> Result<ZCoin, MmError<ZCoinBuildError>> {
    let builder = ZCoinBuilder::new(
        ctx,
        ticker,
        conf,
        params,
        priv_key_policy,
        db_dir_path,
        Some(z_spending_key),
        protocol_info,
    );
    builder.build().await
}

impl MarketCoinOps for ZCoin {
    fn ticker(&self) -> &str { &self.utxo_arc.conf.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { Ok(self.z_fields.my_z_addr_encoded.clone()) }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        let pubkey = utxo_common::my_public_key(self.as_ref())?;
        Ok(pubkey.to_string())
    }

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> { None }

    fn sign_message(&self, _message: &str) -> SignatureResult<String> {
        MmError::err(SignatureError::InvalidRequest(
            "Message signing is not supported by the given coin type".to_string(),
        ))
    }

    fn verify_message(&self, _signature_base64: &str, _message: &str, _address: &str) -> VerificationResult<bool> {
        MmError::err(VerificationError::InvalidRequest(
            "Message verification is not supported by the given coin type".to_string(),
        ))
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            let sat = coin
                .my_balance_sat()
                .await
                .mm_err(|e| BalanceError::WalletStorageError(e.to_string()))?;
            Ok(CoinBalance::new(big_decimal_from_sat_unsigned(sat, coin.decimals())))
        };
        Box::new(fut.boxed().compat())
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { utxo_common::base_coin_balance(self) }

    fn platform_ticker(&self) -> &str { self.ticker() }

    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        let tx_bytes = try_fus!(hex::decode(tx));
        let z_tx = try_fus!(ZTransaction::read(tx_bytes.as_slice()));

        let this = self.clone();
        let tx = tx.to_owned();

        let fut = async move {
            let mut sync_guard = try_s!(this.wait_for_gen_tx_blockchain_sync().await);
            let tx_hash = utxo_common::send_raw_tx(this.as_ref(), &tx).compat().await?;
            sync_guard.respawn_guard.watch_for_tx(z_tx.txid());
            Ok(tx_hash)
        };
        Box::new(fut.boxed().compat())
    }

    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        let z_tx = try_fus!(ZTransaction::read(tx));

        let this = self.clone();
        let tx = tx.to_owned();

        let fut = async move {
            let mut sync_guard = try_s!(this.wait_for_gen_tx_blockchain_sync().await);
            let tx_hash = utxo_common::send_raw_tx_bytes(this.as_ref(), &tx).compat().await?;
            sync_guard.respawn_guard.watch_for_tx(z_tx.txid());
            Ok(tx_hash)
        };
        Box::new(fut.boxed().compat())
    }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::wait_for_confirmations(self.as_ref(), tx, confirmations, requires_nota, wait_until, check_every)
    }

    fn wait_for_htlc_tx_spend(
        &self,
        transaction: &[u8],
        _secret_hash: &[u8],
        wait_until: u64,
        from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
        check_every: f64,
    ) -> TransactionFut {
        utxo_common::wait_for_output_spend(
            self.as_ref(),
            transaction,
            utxo_common::DEFAULT_SWAP_VOUT,
            from_block,
            wait_until,
            check_every,
        )
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        ZTransaction::read(bytes)
            .map(TransactionEnum::from)
            .map_to_mm(|e| TxMarshalingErr::InvalidInput(e.to_string()))
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        utxo_common::current_block(&self.utxo_arc)
    }

    fn display_priv_key(&self) -> Result<String, String> {
        Ok(encode_extended_spending_key(
            z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY,
            &self.z_fields.z_spending_key,
        ))
    }

    fn min_tx_amount(&self) -> BigDecimal { utxo_common::min_tx_amount(self.as_ref()) }

    fn min_trading_vol(&self) -> MmNumber { utxo_common::min_trading_vol(self.as_ref()) }

    fn is_privacy(&self) -> bool { true }
}

#[async_trait]
impl SwapOps for ZCoin {
    fn send_taker_fee(&self, _fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut {
        let selfi = self.clone();
        let uuid = uuid.to_owned();
        let fut = async move {
            let tx = try_tx_s!(z_send_dex_fee(&selfi, amount, &uuid).await);
            Ok(tx.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_maker_payment(&self, maker_payment_args: SendMakerPaymentArgs<'_>) -> TransactionFut {
        let selfi = self.clone();
        let maker_key_pair = self.derive_htlc_key_pair(maker_payment_args.swap_unique_data);
        let taker_pub = try_tx_fus!(Public::from_slice(maker_payment_args.other_pubkey));
        let secret_hash = maker_payment_args.secret_hash.to_vec();
        let time_lock = maker_payment_args.time_lock;
        let amount = maker_payment_args.amount;
        let fut = async move {
            let utxo_tx = try_tx_s!(
                z_send_htlc(
                    &selfi,
                    time_lock,
                    maker_key_pair.public(),
                    &taker_pub,
                    &secret_hash,
                    amount
                )
                .await
            );
            Ok(utxo_tx.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_payment(&self, taker_payment_args: SendTakerPaymentArgs<'_>) -> TransactionFut {
        let selfi = self.clone();
        let taker_keypair = self.derive_htlc_key_pair(taker_payment_args.swap_unique_data);
        let maker_pub = try_tx_fus!(Public::from_slice(taker_payment_args.other_pubkey));
        let secret_hash = taker_payment_args.secret_hash.to_vec();
        let time_lock = taker_payment_args.time_lock;
        let amount = taker_payment_args.amount;
        let fut = async move {
            let utxo_tx = try_tx_s!(
                z_send_htlc(
                    &selfi,
                    time_lock,
                    taker_keypair.public(),
                    &maker_pub,
                    &secret_hash,
                    amount
                )
                .await
            );
            Ok(utxo_tx.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_maker_spends_taker_payment(
        &self,
        maker_spends_payment_args: SendMakerSpendsTakerPaymentArgs<'_>,
    ) -> TransactionFut {
        let tx = try_tx_fus!(ZTransaction::read(maker_spends_payment_args.other_payment_tx));
        let key_pair = self.derive_htlc_key_pair(maker_spends_payment_args.swap_unique_data);
        let time_lock = maker_spends_payment_args.time_lock;
        let redeem_script = payment_script(
            time_lock,
            maker_spends_payment_args.secret_hash,
            &try_tx_fus!(Public::from_slice(maker_spends_payment_args.other_pubkey)),
            key_pair.public(),
        );
        let script_data = ScriptBuilder::default()
            .push_data(maker_spends_payment_args.secret)
            .push_opcode(Opcode::OP_0)
            .into_script();
        let selfi = self.clone();
        let fut = async move {
            let tx_fut = z_p2sh_spend(
                &selfi,
                tx,
                time_lock,
                SEQUENCE_FINAL,
                redeem_script,
                script_data,
                &key_pair,
            );
            let tx = try_ztx_s!(tx_fut.await);
            Ok(tx.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_spends_maker_payment(
        &self,
        taker_spends_payment_args: SendTakerSpendsMakerPaymentArgs<'_>,
    ) -> TransactionFut {
        let tx = try_tx_fus!(ZTransaction::read(taker_spends_payment_args.other_payment_tx));
        let key_pair = self.derive_htlc_key_pair(taker_spends_payment_args.swap_unique_data);
        let time_lock = taker_spends_payment_args.time_lock;
        let redeem_script = payment_script(
            time_lock,
            taker_spends_payment_args.secret_hash,
            &try_tx_fus!(Public::from_slice(taker_spends_payment_args.other_pubkey)),
            key_pair.public(),
        );
        let script_data = ScriptBuilder::default()
            .push_data(taker_spends_payment_args.secret)
            .push_opcode(Opcode::OP_0)
            .into_script();
        let selfi = self.clone();
        let fut = async move {
            let tx_fut = z_p2sh_spend(
                &selfi,
                tx,
                time_lock,
                SEQUENCE_FINAL,
                redeem_script,
                script_data,
                &key_pair,
            );
            let tx = try_ztx_s!(tx_fut.await);
            Ok(tx.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_refunds_payment(
        &self,
        taker_refunds_payment_args: SendTakerRefundsPaymentArgs<'_>,
    ) -> TransactionFut {
        let tx = try_tx_fus!(ZTransaction::read(taker_refunds_payment_args.payment_tx));
        let key_pair = self.derive_htlc_key_pair(taker_refunds_payment_args.swap_unique_data);
        let time_lock = taker_refunds_payment_args.time_lock;
        let redeem_script = payment_script(
            time_lock,
            taker_refunds_payment_args.secret_hash,
            key_pair.public(),
            &try_tx_fus!(Public::from_slice(taker_refunds_payment_args.other_pubkey)),
        );
        let script_data = ScriptBuilder::default().push_opcode(Opcode::OP_1).into_script();
        let selfi = self.clone();
        let fut = async move {
            let tx_fut = z_p2sh_spend(
                &selfi,
                tx,
                time_lock,
                SEQUENCE_FINAL - 1,
                redeem_script,
                script_data,
                &key_pair,
            );
            let tx = try_ztx_s!(tx_fut.await);
            Ok(tx.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn send_maker_refunds_payment(
        &self,
        maker_refunds_payment_args: SendMakerRefundsPaymentArgs<'_>,
    ) -> TransactionFut {
        let tx = try_tx_fus!(ZTransaction::read(maker_refunds_payment_args.payment_tx));
        let key_pair = self.derive_htlc_key_pair(maker_refunds_payment_args.swap_unique_data);
        let time_lock = maker_refunds_payment_args.time_lock;
        let redeem_script = payment_script(
            time_lock,
            maker_refunds_payment_args.secret_hash,
            key_pair.public(),
            &try_tx_fus!(Public::from_slice(maker_refunds_payment_args.other_pubkey)),
        );
        let script_data = ScriptBuilder::default().push_opcode(Opcode::OP_1).into_script();
        let selfi = self.clone();
        let fut = async move {
            let tx_fut = z_p2sh_spend(
                &selfi,
                tx,
                time_lock,
                SEQUENCE_FINAL - 1,
                redeem_script,
                script_data,
                &key_pair,
            );
            let tx = try_ztx_s!(tx_fut.await);
            Ok(tx.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn validate_fee(
        &self,
        validate_fee_args: ValidateFeeArgs<'_>,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let z_tx = match validate_fee_args.fee_tx {
            TransactionEnum::ZTransaction(t) => t.clone(),
            _ => panic!("Unexpected tx {:?}", validate_fee_args.fee_tx),
        };
        let amount_sat = try_fus!(sat_from_big_decimal(validate_fee_args.amount, self.utxo_arc.decimals));
        let expected_memo = MemoBytes::from_bytes(validate_fee_args.uuid).expect("Uuid length < 512");
        let min_block_number = validate_fee_args.min_block_number;

        let coin = self.clone();
        let fut = async move {
            let tx_hash = H256::from(z_tx.txid().0).reversed();
            let tx_from_rpc = try_s!(
                coin.utxo_rpc_client()
                    .get_verbose_transaction(&tx_hash.into())
                    .compat()
                    .await
            );
            let mut encoded = Vec::with_capacity(1024);
            z_tx.write(&mut encoded).expect("Writing should not fail");
            if encoded != tx_from_rpc.hex.0 {
                return ERR!(
                    "Encoded transaction {:?} does not match the tx {:?} from RPC",
                    encoded,
                    tx_from_rpc
                );
            }

            let block_height = match tx_from_rpc.height {
                Some(h) => {
                    if h < min_block_number {
                        return ERR!("Dex fee tx {:?} confirmed before min block {}", z_tx, min_block_number);
                    } else {
                        BlockHeight::from_u32(h as u32)
                    }
                },
                None => H0,
            };

            for shielded_out in z_tx.shielded_outputs.iter() {
                if let Some((note, address, memo)) =
                    try_sapling_output_recovery(coin.consensus_params_ref(), block_height, &DEX_FEE_OVK, shielded_out)
                {
                    if address != coin.z_fields.dex_fee_addr {
                        let encoded =
                            encode_payment_address(z_mainnet_constants::HRP_SAPLING_PAYMENT_ADDRESS, &address);
                        let expected = encode_payment_address(
                            z_mainnet_constants::HRP_SAPLING_PAYMENT_ADDRESS,
                            &coin.z_fields.dex_fee_addr,
                        );
                        return ERR!(
                            "Dex fee was sent to the invalid address {}, expected {}",
                            encoded,
                            expected
                        );
                    }

                    if note.value != amount_sat {
                        return ERR!("Dex fee has invalid amount {}, expected {}", note.value, amount_sat);
                    }

                    if memo != expected_memo {
                        return ERR!("Dex fee has invalid memo {:?}, expected {:?}", memo, expected_memo);
                    }

                    return Ok(());
                }
            }

            ERR!(
                "The dex fee tx {:?} has no shielded outputs or outputs decryption failed",
                z_tx
            )
        };

        Box::new(fut.boxed().compat())
    }

    #[inline]
    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        utxo_common::validate_maker_payment(self, input)
    }

    #[inline]
    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        utxo_common::validate_taker_payment(self, input)
    }

    #[inline]
    fn check_if_my_payment_sent(
        &self,
        if_my_payment_spent_args: CheckIfMyPaymentSentArgs<'_>,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        utxo_common::check_if_my_payment_sent(
            self.clone(),
            if_my_payment_spent_args.time_lock,
            if_my_payment_spent_args.other_pub,
            if_my_payment_spent_args.secret_hash,
            if_my_payment_spent_args.swap_unique_data,
        )
    }

    #[inline]
    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_my(self, input, utxo_common::DEFAULT_SWAP_VOUT).await
    }

    #[inline]
    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_other(self, input, utxo_common::DEFAULT_SWAP_VOUT).await
    }

    fn check_tx_signed_by_pub(&self, _tx: &[u8], _expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>> {
        unimplemented!();
    }

    #[inline]
    async fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        utxo_common::extract_secret(secret_hash, spend_tx)
    }

    #[inline]
    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        Ok(None)
    }

    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair {
        let message = Message::from(dhash256(swap_unique_data).take());
        let signature = self.secp_keypair().private().sign(&message).expect("valid privkey");

        let key = secp_privkey_from_hash(dhash256(&signature));
        key_pair_from_secret(key.as_slice()).expect("valid privkey")
    }

    #[inline]
    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> {
        utxo_common::validate_other_pubkey(raw_pubkey)
    }

    async fn maker_payment_instructions(
        &self,
        _secret_hash: &[u8],
        _amount: &BigDecimal,
        _maker_lock_duration: u64,
        _expires_in: u64,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    async fn taker_payment_instructions(
        &self,
        _secret_hash: &[u8],
        _amount: &BigDecimal,
        _expires_in: u64,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    fn validate_maker_payment_instructions(
        &self,
        _instructions: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
        _maker_lock_duration: u64,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn validate_taker_payment_instructions(
        &self,
        _instructions: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn is_supported_by_watchers(&self) -> bool { false }
}

#[async_trait]
impl WatcherOps for ZCoin {
    fn create_maker_payment_spend_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_maker_payment_spend_preimage(&self, _preimage: &[u8], _secret: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn create_taker_payment_refund_preimage(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_payment_refund_preimage(&self, _taker_refunds_payment: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn watcher_validate_taker_fee(&self, _input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        _input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!();
    }
}

#[async_trait]
impl MmCoin for ZCoin {
    fn is_asset_chain(&self) -> bool { self.utxo_arc.conf.asset_chain }

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.as_ref().abortable_system) }

    fn withdraw(&self, _req: WithdrawRequest) -> WithdrawFut {
        Box::new(futures01::future::err(MmError::new(WithdrawError::InternalError(
            "Zcoin doesn't support legacy withdraw".into(),
        ))))
    }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut {
        Box::new(utxo_common::get_raw_transaction(&self.utxo_arc, req).boxed().compat())
    }

    fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut {
        Box::new(
            utxo_common::get_tx_hex_by_hash(&self.utxo_arc, tx_hash)
                .boxed()
                .compat(),
        )
    }

    fn decimals(&self) -> u8 { self.utxo_arc.decimals }

    fn convert_to_address(&self, _from: &str, _to_address_format: Json) -> Result<String, String> {
        Err(MmError::new("Address conversion is not available for ZCoin".to_string()).to_string())
    }

    fn validate_address(&self, address: &str) -> ValidateAddressResult {
        match decode_payment_address(z_mainnet_constants::HRP_SAPLING_PAYMENT_ADDRESS, address) {
            Ok(Some(_)) => ValidateAddressResult {
                is_valid: true,
                reason: None,
            },
            Ok(None) => ValidateAddressResult {
                is_valid: false,
                reason: Some("decode_payment_address returned None".to_owned()),
            },
            Err(e) => ValidateAddressResult {
                is_valid: false,
                reason: Some(format!("Error {} on decode_payment_address", e)),
            },
        }
    }

    fn process_history_loop(&self, _ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> {
        log::warn!("process_history_loop is not implemented for ZCoin yet!");
        Box::new(futures01::future::err(()))
    }

    fn history_sync_status(&self) -> HistorySyncState { HistorySyncState::NotEnabled }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> {
        utxo_common::get_trade_fee(self.clone())
    }

    async fn get_sender_trade_fee(
        &self,
        _value: TradePreimageValue,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        Ok(TradeFee {
            coin: self.ticker().to_owned(),
            amount: self.get_one_kbyte_tx_fee().await?.into(),
            paid_from_trading_vol: false,
        })
    }

    fn get_receiver_trade_fee(&self, _send_amount: BigDecimal, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        utxo_common::get_receiver_trade_fee(self.clone())
    }

    async fn get_fee_to_send_taker_fee(
        &self,
        _dex_fee_amount: BigDecimal,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        Ok(TradeFee {
            coin: self.ticker().to_owned(),
            amount: self.get_one_kbyte_tx_fee().await?.into(),
            paid_from_trading_vol: false,
        })
    }

    fn required_confirmations(&self) -> u64 { utxo_common::required_confirmations(&self.utxo_arc) }

    fn requires_notarization(&self) -> bool { utxo_common::requires_notarization(&self.utxo_arc) }

    fn set_required_confirmations(&self, confirmations: u64) {
        utxo_common::set_required_confirmations(&self.utxo_arc, confirmations)
    }

    fn set_requires_notarization(&self, requires_nota: bool) {
        utxo_common::set_requires_notarization(&self.utxo_arc, requires_nota)
    }

    fn swap_contract_address(&self) -> Option<BytesJson> { utxo_common::swap_contract_address() }

    fn fallback_swap_contract(&self) -> Option<BytesJson> { utxo_common::fallback_swap_contract() }

    fn mature_confirmations(&self) -> Option<u32> { Some(self.utxo_arc.conf.mature_confirmations) }

    fn coin_protocol_info(&self) -> Vec<u8> { utxo_common::coin_protocol_info(self) }

    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool {
        utxo_common::is_coin_protocol_supported(self, info)
    }

    fn on_disabled(&self) -> Result<(), AbortedError> { AbortableSystem::abort_all(&self.as_ref().abortable_system) }

    fn on_token_deactivated(&self, _ticker: &str) {}
}

#[async_trait]
impl UtxoTxGenerationOps for ZCoin {
    async fn get_tx_fee(&self) -> UtxoRpcResult<ActualTxFee> { utxo_common::get_tx_fee(&self.utxo_arc).await }

    async fn calc_interest_if_required(
        &self,
        unsigned: TransactionInputSigner,
        data: AdditionalTxData,
        my_script_pub: Bytes,
    ) -> UtxoRpcResult<(TransactionInputSigner, AdditionalTxData)> {
        utxo_common::calc_interest_if_required(self, unsigned, data, my_script_pub).await
    }
}

#[async_trait]
impl UtxoTxBroadcastOps for ZCoin {
    async fn broadcast_tx(&self, tx: &UtxoTx) -> Result<H256Json, MmError<BroadcastTxErr>> {
        utxo_common::broadcast_tx(self, tx).await
    }
}

/// Please note `ZCoin` is not assumed to work with transparent UTXOs.
/// Remove implementation of the `GetUtxoListOps` trait for `ZCoin`
/// when [`ZCoin::preimage_trade_fee_required_to_send_outputs`] is refactored.
#[async_trait]
#[cfg_attr(test, mockable)]
impl GetUtxoListOps for ZCoin {
    async fn get_unspent_ordered_list(
        &self,
        address: &Address,
    ) -> UtxoRpcResult<(Vec<UnspentInfo>, RecentlySpentOutPointsGuard<'_>)> {
        utxo_common::get_unspent_ordered_list(self, address).await
    }

    async fn get_all_unspent_ordered_list(
        &self,
        address: &Address,
    ) -> UtxoRpcResult<(Vec<UnspentInfo>, RecentlySpentOutPointsGuard<'_>)> {
        utxo_common::get_all_unspent_ordered_list(self, address).await
    }

    async fn get_mature_unspent_ordered_list(
        &self,
        address: &Address,
    ) -> UtxoRpcResult<(MatureUnspentList, RecentlySpentOutPointsGuard<'_>)> {
        utxo_common::get_mature_unspent_ordered_list(self, address).await
    }
}

#[async_trait]
impl UtxoCommonOps for ZCoin {
    async fn get_htlc_spend_fee(&self, tx_size: u64, stage: &FeeApproxStage) -> UtxoRpcResult<u64> {
        utxo_common::get_htlc_spend_fee(self, tx_size, stage).await
    }

    fn addresses_from_script(&self, script: &Script) -> Result<Vec<Address>, String> {
        utxo_common::addresses_from_script(self, script)
    }

    fn denominate_satoshis(&self, satoshi: i64) -> f64 { utxo_common::denominate_satoshis(&self.utxo_arc, satoshi) }

    fn my_public_key(&self) -> Result<&Public, MmError<UnexpectedDerivationMethod>> {
        utxo_common::my_public_key(self.as_ref())
    }

    fn address_from_str(&self, address: &str) -> MmResult<Address, AddrFromStrError> {
        utxo_common::checked_address_from_str(self, address)
    }

    async fn get_current_mtp(&self) -> UtxoRpcResult<u32> {
        utxo_common::get_current_mtp(&self.utxo_arc, CoinVariant::Standard).await
    }

    fn is_unspent_mature(&self, output: &RpcTransaction) -> bool {
        utxo_common::is_unspent_mature(self.utxo_arc.conf.mature_confirmations, output)
    }

    async fn calc_interest_of_tx(
        &self,
        _tx: &UtxoTx,
        _input_transactions: &mut HistoryUtxoTxMap,
    ) -> UtxoRpcResult<u64> {
        MmError::err(UtxoRpcError::Internal(
            "ZCoin doesn't support transaction rewards".to_owned(),
        ))
    }

    async fn get_mut_verbose_transaction_from_map_or_rpc<'a, 'b>(
        &'a self,
        tx_hash: H256Json,
        utxo_tx_map: &'b mut HistoryUtxoTxMap,
    ) -> UtxoRpcResult<&'b mut HistoryUtxoTx> {
        utxo_common::get_mut_verbose_transaction_from_map_or_rpc(self, tx_hash, utxo_tx_map).await
    }

    async fn p2sh_spending_tx(&self, input: utxo_common::P2SHSpendingTxInput<'_>) -> Result<UtxoTx, String> {
        utxo_common::p2sh_spending_tx(self, input).await
    }

    fn get_verbose_transactions_from_cache_or_rpc(
        &self,
        tx_ids: HashSet<H256Json>,
    ) -> UtxoRpcFut<HashMap<H256Json, VerboseTransactionFrom>> {
        let selfi = self.clone();
        let fut = async move { utxo_common::get_verbose_transactions_from_cache_or_rpc(&selfi.utxo_arc, tx_ids).await };
        Box::new(fut.boxed().compat())
    }

    async fn preimage_trade_fee_required_to_send_outputs(
        &self,
        outputs: Vec<TransactionOutput>,
        fee_policy: FeePolicy,
        gas_fee: Option<u64>,
        stage: &FeeApproxStage,
    ) -> TradePreimageResult<BigDecimal> {
        utxo_common::preimage_trade_fee_required_to_send_outputs(
            self,
            self.ticker(),
            outputs,
            fee_policy,
            gas_fee,
            stage,
        )
        .await
    }

    fn increase_dynamic_fee_by_stage(&self, dynamic_fee: u64, stage: &FeeApproxStage) -> u64 {
        utxo_common::increase_dynamic_fee_by_stage(self, dynamic_fee, stage)
    }

    async fn p2sh_tx_locktime(&self, htlc_locktime: u32) -> Result<u32, MmError<UtxoRpcError>> {
        utxo_common::p2sh_tx_locktime(self, self.ticker(), htlc_locktime).await
    }

    fn addr_format(&self) -> &UtxoAddressFormat { utxo_common::addr_format(self) }

    fn addr_format_for_standard_scripts(&self) -> UtxoAddressFormat {
        utxo_common::addr_format_for_standard_scripts(self)
    }

    fn address_from_pubkey(&self, pubkey: &Public) -> Address {
        let conf = &self.utxo_arc.conf;
        utxo_common::address_from_pubkey(
            pubkey,
            conf.pub_addr_prefix,
            conf.pub_t_addr_prefix,
            conf.checksum_type,
            conf.bech32_hrp.clone(),
            self.addr_format().clone(),
        )
    }
}

#[async_trait]
impl InitWithdrawCoin for ZCoin {
    async fn init_withdraw(
        &self,
        _ctx: MmArc,
        req: WithdrawRequest,
        task_handle: &WithdrawTaskHandle,
    ) -> Result<TransactionDetails, MmError<WithdrawError>> {
        if req.fee.is_some() {
            return MmError::err(WithdrawError::InternalError(
                "Setting a custom withdraw fee is not supported for ZCoin yet".to_owned(),
            ));
        }

        let to_addr = decode_payment_address(z_mainnet_constants::HRP_SAPLING_PAYMENT_ADDRESS, &req.to)
            .map_to_mm(|e| WithdrawError::InvalidAddress(format!("{}", e)))?
            .or_mm_err(|| WithdrawError::InvalidAddress(format!("Address {} decoded to None", req.to)))?;
        let amount = if req.max {
            let fee = self.get_one_kbyte_tx_fee().await?;
            let balance = self.my_balance().compat().await?;
            balance.spendable - fee
        } else {
            req.amount
        };

        task_handle.update_in_progress_status(WithdrawInProgressStatus::GeneratingTransaction)?;
        let satoshi = sat_from_big_decimal(&amount, self.decimals())?;
        let z_output = ZOutput {
            to_addr,
            amount: Amount::from_u64(satoshi)
                .map_to_mm(|_| NumConversError(format!("Failed to get ZCash amount from {}", amount)))?,
            // TODO add optional viewing_key and memo fields to the WithdrawRequest
            viewing_key: Some(self.z_fields.evk.fvk.ovk),
            memo: None,
        };

        let (tx, data, _sync_guard) = self.gen_tx(vec![], vec![z_output]).await?;
        let mut tx_bytes = Vec::with_capacity(1024);
        tx.write(&mut tx_bytes)
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;
        let mut tx_hash = tx.txid().0.to_vec();
        tx_hash.reverse();

        let received_by_me = big_decimal_from_sat_unsigned(data.received_by_me, self.decimals());
        let spent_by_me = big_decimal_from_sat_unsigned(data.spent_by_me, self.decimals());

        Ok(TransactionDetails {
            tx_hex: tx_bytes.into(),
            tx_hash: hex::encode(&tx_hash),
            from: vec![self.z_fields.my_z_addr_encoded.clone()],
            to: vec![req.to],
            my_balance_change: &received_by_me - &spent_by_me,
            total_amount: spent_by_me.clone(),
            spent_by_me,
            received_by_me,
            block_height: 0,
            timestamp: 0,
            fee_details: Some(TxFeeDetails::Utxo(UtxoFeeDetails {
                coin: Some(self.ticker().to_owned()),
                amount: big_decimal_from_sat_unsigned(data.fee_amount, self.decimals()),
            })),
            coin: self.ticker().to_owned(),
            internal_id: tx_hash.into(),
            kmd_rewards: None,
            transaction_type: Default::default(),
        })
    }
}

fn extended_spending_key_from_protocol_info_and_policy(
    protocol_info: &ZcoinProtocolInfo,
    priv_key_policy: &PrivKeyBuildPolicy,
) -> MmResult<ExtendedSpendingKey, ZCoinBuildError> {
    match priv_key_policy {
        PrivKeyBuildPolicy::IguanaPrivKey(iguana) => Ok(ExtendedSpendingKey::master(iguana.as_slice())),
        PrivKeyBuildPolicy::GlobalHDAccount(global_hd) => {
            extended_spending_key_from_global_hd_account(protocol_info, global_hd)
        },
        PrivKeyBuildPolicy::Trezor => {
            let priv_key_err = PrivKeyPolicyNotAllowed::HardwareWalletNotSupported;
            MmError::err(ZCoinBuildError::UtxoBuilderError(
                UtxoCoinBuildError::PrivKeyPolicyNotAllowed(priv_key_err),
            ))
        },
    }
}

fn extended_spending_key_from_global_hd_account(
    protocol_info: &ZcoinProtocolInfo,
    global_hd: &GlobalHDAccountArc,
) -> MmResult<ExtendedSpendingKey, ZCoinBuildError> {
    let path_to_coin = protocol_info
        .z_derivation_path
        .clone()
        .or_mm_err(|| ZCoinBuildError::ZDerivationPathNotSet)?;

    let path_to_account = path_to_coin
        .to_derivation_path()
        .into_iter()
        // Map `bip32::ChildNumber` to `zip32::Zip32Child`.
        .map(|child| Zip32Child::from_index(child.0))
        // Push the hardened `account` index, so the derivation path looks like:
        // `m/purpose'/coin'/account'`.
        .chain(iter::once(Zip32Child::Hardened(global_hd.account_id())));

    let mut spending_key = ExtendedSpendingKey::master(global_hd.root_seed_bytes());
    for zip32_child in path_to_account {
        spending_key = spending_key.derive_child(zip32_child);
    }

    Ok(spending_key)
}

#[test]
fn derive_z_key_from_mm_seed() {
    use crypto::privkey::key_pair_from_seed;
    use zcash_client_backend::encoding::encode_extended_spending_key;

    let seed = "spice describe gravity federal blast come thank unfair canal monkey style afraid";
    let secp_keypair = key_pair_from_seed(seed).unwrap();
    let z_spending_key = ExtendedSpendingKey::master(&*secp_keypair.private().secret);
    let encoded = encode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, &z_spending_key);
    assert_eq!(encoded, "secret-extended-key-main1qqqqqqqqqqqqqqytwz2zjt587n63kyz6jawmflttqu5rxavvqx3lzfs0tdr0w7g5tgntxzf5erd3jtvva5s52qx0ms598r89vrmv30r69zehxy2r3vesghtqd6dfwdtnauzuj8u8eeqfx7qpglzu6z54uzque6nzzgnejkgq569ax4lmk0v95rfhxzxlq3zrrj2z2kqylx2jp8g68lqu6alczdxd59lzp4hlfuj3jp54fp06xsaaay0uyass992g507tdd7psua5w6q76dyq3");

    let (_, address) = z_spending_key.default_address().unwrap();
    let encoded_addr = encode_payment_address(z_mainnet_constants::HRP_SAPLING_PAYMENT_ADDRESS, &address);
    assert_eq!(
        encoded_addr,
        "zs182ht30wnnnr8jjhj2j9v5dkx3qsknnr5r00jfwk2nczdtqy7w0v836kyy840kv2r8xle5gcl549"
    );

    let seed = "also shoot benefit prefer juice shell elder veteran woman mimic image kidney";
    let secp_keypair = key_pair_from_seed(seed).unwrap();
    let z_spending_key = ExtendedSpendingKey::master(&*secp_keypair.private().secret);
    let encoded = encode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, &z_spending_key);
    assert_eq!(encoded, "secret-extended-key-main1qqqqqqqqqqqqqq8jnhc9stsqwts6pu5ayzgy4szplvy03u227e50n3u8e6dwn5l0q5s3s8xfc03r5wmyh5s5dq536ufwn2k89ngdhnxy64sd989elwas6kr7ygztsdkw6k6xqyvhtu6e0dhm4mav8rus0fy8g0hgy9vt97cfjmus0m2m87p4qz5a00um7gwjwk494gul0uvt3gqyjujcclsqry72z57kr265jsajactgfn9m3vclqvx8fsdnwp4jwj57ffw560vvwks9g9hpu");

    let (_, address) = z_spending_key.default_address().unwrap();
    let encoded_addr = encode_payment_address(z_mainnet_constants::HRP_SAPLING_PAYMENT_ADDRESS, &address);
    assert_eq!(
        encoded_addr,
        "zs1funuwrjr2stlr6fnhkdh7fyz3p7n0p8rxase9jnezdhc286v5mhs6q3myw0phzvad5mvqgfxpam"
    );
}

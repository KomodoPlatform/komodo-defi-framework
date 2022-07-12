use crate::rpc_command::init_withdraw::{InitWithdrawCoin, WithdrawInProgressStatus, WithdrawTaskHandle};
use crate::utxo::rpc_clients::{ElectrumRpcRequest, UnspentInfo, UtxoRpcClientEnum, UtxoRpcError, UtxoRpcFut,
                               UtxoRpcResult};
use crate::utxo::utxo_builder::{UtxoCoinBuilderCommonOps, UtxoCoinWithIguanaPrivKeyBuilder,
                                UtxoFieldsWithIguanaPrivKeyBuilder};
use crate::utxo::utxo_common::{big_decimal_from_sat_unsigned, payment_script};
use crate::utxo::{sat_from_big_decimal, utxo_common, ActualTxFee, AdditionalTxData, Address, BroadcastTxErr,
                  FeePolicy, GetUtxoListOps, HistoryUtxoTx, HistoryUtxoTxMap, MatureUnspentList,
                  RecentlySpentOutPointsGuard, UtxoActivationParams, UtxoAddressFormat, UtxoArc, UtxoCoinFields,
                  UtxoCommonOps, UtxoFeeDetails, UtxoRpcMode, UtxoTxBroadcastOps, UtxoTxGenerationOps,
                  VerboseTransactionFrom};
use crate::{BalanceError, BalanceFut, CoinBalance, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps,
            MmCoin, NegotiateSwapContractAddrErr, NumConversError, PrivKeyActivationPolicy, RawTransactionFut,
            RawTransactionRequest, SearchForSwapTxSpendInput, SignatureError, SignatureResult, SwapOps, TradeFee,
            TradePreimageFut, TradePreimageResult, TradePreimageValue, TransactionDetails, TransactionEnum,
            TransactionFut, TxFeeDetails, UnexpectedDerivationMethod, ValidateAddressResult, ValidatePaymentInput,
            VerificationError, VerificationResult, WithdrawFut, WithdrawRequest};
use crate::{Transaction, WithdrawError};
use async_trait::async_trait;
use bitcrypto::{dhash160, dhash256};
use chain::constants::SEQUENCE_FINAL;
use chain::{Transaction as UtxoTx, TransactionOutput};
use common::{async_blocking, log};
use crypto::privkey::{key_pair_from_secret, secp_privkey_from_hash};
use futures::compat::Future01CompatExt;
use futures::lock::Mutex as AsyncMutex;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use http::Uri;
use keys::hash::H256;
use keys::{KeyPair, Message, Public};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumber};
#[cfg(test)] use mocktopus::macros::*;
use primitives::bytes::Bytes;
use rpc::v1::types::{Bytes as BytesJson, Transaction as RpcTransaction, H256 as H256Json};
use script::{Builder as ScriptBuilder, Opcode, Script, TransactionInputSigner};
use serde_json::Value as Json;
use serialization::CoinVariant;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
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
use zcash_primitives::{consensus, constants::mainnet as z_mainnet_constants, sapling::PaymentAddress,
                       zip32::ExtendedFullViewingKey, zip32::ExtendedSpendingKey};
use zcash_proofs::prover::LocalTxProver;

mod z_htlc;
use z_htlc::{z_p2sh_spend, z_send_dex_fee, z_send_htlc};

mod z_rpc;
pub use z_rpc::SyncStatus;
use z_rpc::{init_light_client, SaplingSyncConnector, SaplingSyncGuard, WalletDbShared};

mod z_coin_errors;
pub use z_coin_errors::*;

#[cfg(all(test, feature = "zhtlc-native-tests"))]
mod z_coin_tests;

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
    z_tx_prover: Arc<LocalTxProver>,
    light_wallet_db: WalletDbShared,
    consensus_params: ZcoinConsensusParams,
    sync_state_connector: AsyncMutex<SaplingSyncConnector>,
}

impl std::fmt::Debug for ZCoinFields {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "ZCoinFields {{ my_z_addr: {:?}, my_z_addr_encoded: {} }}",
            self.my_z_addr, self.my_z_addr_encoded
        )
    }
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

#[derive(Clone, Debug)]
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
                None,
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

#[derive(Deserialize)]
pub struct ZcoinActivationParams {
    pub mode: ZcoinRpcMode,
    pub required_confirmations: Option<u64>,
    pub requires_notarization: Option<bool>,
}

pub async fn z_coin_from_conf_and_params(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    params: &ZcoinActivationParams,
    consensus_params: ZcoinConsensusParams,
    secp_priv_key: &[u8],
) -> Result<ZCoin, MmError<ZCoinBuildError>> {
    let z_key = ExtendedSpendingKey::master(secp_priv_key);
    let db_dir = ctx.dbdir();
    z_coin_from_conf_and_params_with_z_key(
        ctx,
        ticker,
        conf,
        params,
        secp_priv_key,
        db_dir,
        z_key,
        consensus_params,
    )
    .await
}

pub struct ZCoinBuilder<'a> {
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    z_coin_params: &'a ZcoinActivationParams,
    utxo_params: UtxoActivationParams,
    secp_priv_key: &'a [u8],
    db_dir_path: PathBuf,
    z_spending_key: ExtendedSpendingKey,
    consensus_params: ZcoinConsensusParams,
}

impl<'a> UtxoCoinBuilderCommonOps for ZCoinBuilder<'a> {
    fn ctx(&self) -> &MmArc { self.ctx }

    fn conf(&self) -> &Json { self.conf }

    fn activation_params(&self) -> &UtxoActivationParams { &self.utxo_params }

    fn ticker(&self) -> &str { self.ticker }
}

#[async_trait]
impl<'a> UtxoFieldsWithIguanaPrivKeyBuilder for ZCoinBuilder<'a> {}

#[async_trait]
impl<'a> UtxoCoinWithIguanaPrivKeyBuilder for ZCoinBuilder<'a> {
    type ResultCoin = ZCoin;
    type Error = ZCoinBuildError;

    fn priv_key(&self) -> &[u8] { self.secp_priv_key }

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error> {
        let utxo = self.build_utxo_fields_with_iguana_priv_key(self.priv_key()).await?;
        let utxo_arc = UtxoArc::new(utxo);

        let (_, my_z_addr) = self
            .z_spending_key
            .default_address()
            .map_err(|_| MmError::new(ZCoinBuildError::GetAddressError))?;

        let dex_fee_addr = decode_payment_address(self.consensus_params.hrp_sapling_payment_address(), DEX_FEE_Z_ADDR)
            .expect("DEX_FEE_Z_ADDR is a valid z-address")
            .expect("DEX_FEE_Z_ADDR is a valid z-address");

        let z_tx_prover = async_blocking(LocalTxProver::with_default_location)
            .await
            .or_mm_err(|| ZCoinBuildError::ZCashParamsNotFound)?;

        let my_z_addr_encoded = encode_payment_address(self.consensus_params.hrp_sapling_payment_address(), &my_z_addr);

        let evk = ExtendedFullViewingKey::from(&self.z_spending_key);
        let (sync_state_connector, light_wallet_db) = match &self.z_coin_params.mode {
            ZcoinRpcMode::Native => {
                return MmError::err(ZCoinBuildError::NativeModeIsNotSupportedYet);
            },
            ZcoinRpcMode::Light {
                light_wallet_d_servers, ..
            } => {
                let cache_db_path = self.db_dir_path.join(format!("{}_light_cache.db", self.ticker));
                let wallet_db_path = self.db_dir_path.join(format!("{}_light_wallet.db", self.ticker));
                // TODO multi lightwalletd servers support will be added on the next iteration
                let uri = Uri::from_str(
                    light_wallet_d_servers
                        .first()
                        .or_mm_err(|| ZCoinBuildError::EmptyLightwalletdUris)?,
                )?;

                init_light_client(uri, cache_db_path, wallet_db_path, self.consensus_params.clone(), evk).await?
            },
        };

        let z_fields = ZCoinFields {
            dex_fee_addr,
            my_z_addr,
            my_z_addr_encoded,
            z_spending_key: self.z_spending_key,
            z_tx_prover: Arc::new(z_tx_prover),
            light_wallet_db,
            consensus_params: self.consensus_params,
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
        secp_priv_key: &'a [u8],
        db_dir_path: PathBuf,
        z_spending_key: ExtendedSpendingKey,
        consensus_params: ZcoinConsensusParams,
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
            scan_policy: Default::default(),
            priv_key_policy: PrivKeyActivationPolicy::IguanaPrivKey,
            check_utxo_maturity: None,
        };
        ZCoinBuilder {
            ctx,
            ticker,
            conf,
            z_coin_params,
            utxo_params,
            secp_priv_key,
            db_dir_path,
            z_spending_key,
            consensus_params,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn z_coin_from_conf_and_params_with_z_key(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    params: &ZcoinActivationParams,
    secp_priv_key: &[u8],
    db_dir_path: PathBuf,
    z_spending_key: ExtendedSpendingKey,
    consensus_params: ZcoinConsensusParams,
) -> Result<ZCoin, MmError<ZCoinBuildError>> {
    let builder = ZCoinBuilder::new(
        ctx,
        ticker,
        conf,
        params,
        secp_priv_key,
        db_dir_path,
        z_spending_key,
        consensus_params,
    );
    builder.build().await
}

impl MarketCoinOps for ZCoin {
    fn ticker(&self) -> &str { &self.utxo_arc.conf.ticker }

    fn my_address(&self) -> Result<String, String> { Ok(self.z_fields.my_z_addr_encoded.clone()) }

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

    fn wait_for_tx_spend(
        &self,
        transaction: &[u8],
        wait_until: u64,
        from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        utxo_common::wait_for_output_spend(
            self.as_ref(),
            transaction,
            utxo_common::DEFAULT_SWAP_VOUT,
            from_block,
            wait_until,
        )
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> {
        ZTransaction::read(bytes).map(|tx| tx.into()).map_err(|e| e.to_string())
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

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        let selfi = self.clone();
        let maker_key_pair = self.derive_htlc_key_pair(swap_unique_data);
        let taker_pub = try_tx_fus!(Public::from_slice(taker_pub));
        let secret_hash = secret_hash.to_vec();
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

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        let selfi = self.clone();
        let taker_keypair = self.derive_htlc_key_pair(swap_unique_data);
        let maker_pub = try_tx_fus!(Public::from_slice(maker_pub));
        let secret_hash = secret_hash.to_vec();
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
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx = try_tx_fus!(ZTransaction::read(taker_payment_tx));
        let key_pair = self.derive_htlc_key_pair(swap_unique_data);
        let redeem_script = payment_script(
            time_lock,
            &*dhash160(secret),
            &try_tx_fus!(Public::from_slice(taker_pub)),
            key_pair.public(),
        );
        let script_data = ScriptBuilder::default()
            .push_data(secret)
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
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx = try_tx_fus!(ZTransaction::read(maker_payment_tx));
        let key_pair = self.derive_htlc_key_pair(swap_unique_data);
        let redeem_script = payment_script(
            time_lock,
            &*dhash160(secret),
            &try_tx_fus!(Public::from_slice(maker_pub)),
            key_pair.public(),
        );
        let script_data = ScriptBuilder::default()
            .push_data(secret)
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
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx = try_tx_fus!(ZTransaction::read(taker_payment_tx));
        let key_pair = self.derive_htlc_key_pair(swap_unique_data);
        let redeem_script = payment_script(
            time_lock,
            secret_hash,
            key_pair.public(),
            &try_tx_fus!(Public::from_slice(maker_pub)),
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
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx = try_tx_fus!(ZTransaction::read(maker_payment_tx));
        let key_pair = self.derive_htlc_key_pair(swap_unique_data);
        let redeem_script = payment_script(
            time_lock,
            secret_hash,
            key_pair.public(),
            &try_tx_fus!(Public::from_slice(taker_pub)),
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
        fee_tx: &TransactionEnum,
        _expected_sender: &[u8],
        _fee_addr: &[u8],
        amount: &BigDecimal,
        min_block_number: u64,
        uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let z_tx = match fee_tx {
            TransactionEnum::ZTransaction(t) => t.clone(),
            _ => panic!("Unexpected tx {:?}", fee_tx),
        };
        let amount_sat = try_fus!(sat_from_big_decimal(amount, self.utxo_arc.decimals));
        let expected_memo = MemoBytes::from_bytes(uuid).expect("Uuid length < 512");

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

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::validate_maker_payment(self, input)
    }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::validate_taker_payment(self, input)
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        _search_from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        utxo_common::check_if_my_payment_sent(self.clone(), time_lock, other_pub, secret_hash, swap_unique_data)
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_my(self, input, utxo_common::DEFAULT_SWAP_VOUT).await
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_other(self, input, utxo_common::DEFAULT_SWAP_VOUT).await
    }

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        utxo_common::extract_secret(secret_hash, spend_tx)
    }

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
}

#[async_trait]
impl MmCoin for ZCoin {
    fn is_asset_chain(&self) -> bool { self.utxo_arc.conf.asset_chain }

    fn withdraw(&self, _req: WithdrawRequest) -> WithdrawFut {
        Box::new(futures01::future::err(MmError::new(WithdrawError::InternalError(
            "Zcoin doesn't support legacy withdraw".into(),
        ))))
    }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut {
        Box::new(utxo_common::get_raw_transaction(&self.utxo_arc, req).boxed().compat())
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

    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
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

    fn mature_confirmations(&self) -> Option<u32> { Some(self.utxo_arc.conf.mature_confirmations) }

    fn coin_protocol_info(&self) -> Vec<u8> { utxo_common::coin_protocol_info(self) }

    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool {
        utxo_common::is_coin_protocol_supported(self, info)
    }
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
    async fn get_htlc_spend_fee(&self, tx_size: u64) -> UtxoRpcResult<u64> {
        utxo_common::get_htlc_spend_fee(self, tx_size).await
    }

    fn addresses_from_script(&self, script: &Script) -> Result<Vec<Address>, String> {
        utxo_common::addresses_from_script(self, script)
    }

    fn denominate_satoshis(&self, satoshi: i64) -> f64 { utxo_common::denominate_satoshis(&self.utxo_arc, satoshi) }

    fn my_public_key(&self) -> Result<&Public, MmError<UnexpectedDerivationMethod>> {
        utxo_common::my_public_key(self.as_ref())
    }

    fn address_from_str(&self, address: &str) -> Result<Address, String> {
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
            viewing_key: None,
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

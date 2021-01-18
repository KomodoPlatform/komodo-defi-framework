use super::*;
use bigdecimal::{BigDecimal, Zero};
pub use bitcrypto::{dhash160, sha256, ChecksumType};
use chain::constants::SEQUENCE_FINAL;
use chain::{OutPoint, TransactionInput, TransactionOutput};
use common::executor::Timer;
use common::jsonrpc_client::{JsonRpcError, JsonRpcErrorType};
use common::log::{error, info};
use common::mm_ctx::MmArc;
use common::mm_metrics::MetricsArc;
use futures::compat::Future01CompatExt;
use futures::future::{FutureExt, TryFutureExt};
use futures01::future::Either;
use gstuff::now_ms;
use keys::bytes::Bytes;
use keys::{Address, KeyPair, Public, Type};
use primitives::hash::H512;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use script::{Builder, Opcode, Script, ScriptAddress, SignatureVersion, TransactionInputSigner,
             UnsignedTransactionInput};
use serde_json::{self as json};
use serialization::{deserialize, serialize};
use std::cmp::Ordering;
use std::collections::hash_map::{Entry, HashMap};
use std::str::FromStr;
use std::sync::atomic::Ordering as AtomicOrderding;
use std::thread;
use std::time::Duration;

pub use chain::Transaction as UtxoTx;

use self::rpc_clients::{electrum_script_hash, UnspentInfo, UtxoRpcClientEnum};
use crate::utxo::rpc_clients::UtxoRpcClientOps;
use crate::ValidateAddressResult;
use common::block_on;

macro_rules! true_or {
    ($cond: expr, $etype: expr) => {
        if !$cond {
            return Err($etype);
        }
    };
}

lazy_static! {
    pub static ref HISTORY_TOO_LARGE_ERROR: Json = json!({
        "code": 1,
        "message": "history too large"
    });
}

pub const HISTORY_TOO_LARGE_ERR_CODE: i64 = -1;

pub struct UtxoArcBuilder<'a> {
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    req: &'a Json,
    priv_key: &'a [u8],
}

impl<'a> UtxoArcBuilder<'a> {
    pub fn new(
        ctx: &'a MmArc,
        ticker: &'a str,
        conf: &'a Json,
        req: &'a Json,
        priv_key: &'a [u8],
    ) -> UtxoArcBuilder<'a> {
        UtxoArcBuilder {
            ctx,
            ticker,
            conf,
            req,
            priv_key,
        }
    }
}

#[async_trait]
impl UtxoCoinBuilder for UtxoArcBuilder<'_> {
    type ResultCoin = UtxoArc;

    async fn build(self) -> Result<Self::ResultCoin, String> {
        let utxo = try_s!(self.build_utxo_fields().await);
        Ok(UtxoArc(Arc::new(utxo)))
    }

    fn ctx(&self) -> &MmArc { self.ctx }

    fn conf(&self) -> &Json { self.conf }

    fn req(&self) -> &Json { self.req }

    fn ticker(&self) -> &str { self.ticker }

    fn priv_key(&self) -> &[u8] { self.priv_key }
}

pub async fn utxo_arc_from_conf_and_request<T>(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    req: &Json,
    priv_key: &[u8],
) -> Result<T, String>
where
    T: From<UtxoArc> + AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let builder = UtxoArcBuilder::new(ctx, ticker, conf, req, priv_key);
    let utxo_arc = try_s!(builder.build().await);

    let merge_params: Option<UtxoMergeParams> = try_s!(json::from_value(req["utxo_merge_params"].clone()));
    if let Some(merge_params) = merge_params {
        let weak = utxo_arc.downgrade();
        let merge_loop = merge_utxo_loop::<T>(
            weak,
            merge_params.merge_at,
            merge_params.check_every,
            merge_params.max_merge_at_once,
        );
        info!("Starting UTXO merge loop for coin {}", ticker);
        spawn(merge_loop);
    }
    Ok(T::from(utxo_arc))
}

fn ten_f64() -> f64 { 10. }

fn one_hundred() -> usize { 100 }

#[derive(Debug, Deserialize)]
struct UtxoMergeParams {
    merge_at: usize,
    #[serde(default = "ten_f64")]
    check_every: f64,
    #[serde(default = "one_hundred")]
    max_merge_at_once: usize,
}

pub async fn get_tx_fee(coin: &UtxoCoinFields) -> Result<ActualTxFee, JsonRpcError> {
    match &coin.tx_fee {
        TxFee::Fixed(fee) => Ok(ActualTxFee::Fixed(*fee)),
        TxFee::Dynamic(method) => {
            let fee = coin
                .rpc_client
                .estimate_fee_sat(coin.decimals, method, &coin.estimate_fee_mode)
                .compat()
                .await?;
            Ok(ActualTxFee::Dynamic(fee))
        },
    }
}

/// returns the fee required to be paid for HTLC spend transaction
pub async fn get_htlc_spend_fee<T>(coin: &T) -> Result<u64, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    let coin_fee = try_s!(coin.get_tx_fee().await);
    let mut fee = match coin_fee {
        ActualTxFee::Fixed(fee) => fee,
        // atomic swap payment spend transaction is slightly more than 300 bytes in average as of now
        ActualTxFee::Dynamic(fee_per_kb) => (fee_per_kb * SWAP_TX_SPEND_SIZE) / KILO_BYTE,
    };
    if coin.as_ref().force_min_relay_fee {
        let relay_fee = try_s!(coin.as_ref().rpc_client.get_relay_fee().compat().await);
        let relay_fee_sat = try_s!(sat_from_big_decimal(&relay_fee, coin.as_ref().decimals));
        if fee < relay_fee_sat {
            fee = relay_fee_sat;
        }
    }
    Ok(fee)
}

pub fn addresses_from_script(coin: &UtxoCoinFields, script: &Script) -> Result<Vec<Address>, String> {
    let destinations: Vec<ScriptAddress> = try_s!(script.extract_destinations());

    let addresses = destinations
        .into_iter()
        .map(|dst| {
            let (prefix, t_addr_prefix) = match dst.kind {
                Type::P2PKH => (coin.pub_addr_prefix, coin.pub_t_addr_prefix),
                Type::P2SH => (coin.p2sh_addr_prefix, coin.p2sh_t_addr_prefix),
            };

            Address {
                hash: dst.hash,
                checksum_type: coin.checksum_type,
                prefix,
                t_addr_prefix,
            }
        })
        .collect();

    Ok(addresses)
}

pub fn denominate_satoshis(coin: &UtxoCoinFields, satoshi: i64) -> f64 {
    satoshi as f64 / 10f64.powf(coin.decimals as f64)
}

pub fn base_coin_balance<T>(coin: &T) -> Box<dyn Future<Item = BigDecimal, Error = String> + Send>
where
    T: MarketCoinOps,
{
    coin.my_balance()
}

pub fn display_address(coin: &UtxoCoinFields, address: &Address) -> Result<String, String> {
    match &coin.address_format {
        UtxoAddressFormat::Standard => Ok(address.to_string()),
        UtxoAddressFormat::CashAddress { network } => address
            .to_cashaddress(&network, coin.pub_addr_prefix, coin.p2sh_addr_prefix)
            .and_then(|cashaddress| cashaddress.encode()),
    }
}

pub fn address_from_str(coin: &UtxoCoinFields, address: &str) -> Result<Address, String> {
    match &coin.address_format {
        UtxoAddressFormat::Standard => Address::from_str(address)
            .or_else(|e| match Address::from_cashaddress(
                &address,
                coin.checksum_type,
                coin.pub_addr_prefix,
                coin.p2sh_addr_prefix) {
                Ok(_) => ERR!("Legacy address format activated for {}, but cashaddress format used instead. Try to call 'convertaddress'", coin.ticker),
                Err(_) => ERR!("{}", e),
            }),
        UtxoAddressFormat::CashAddress { .. } => Address::from_cashaddress(
            &address,
            coin.checksum_type,
            coin.pub_addr_prefix,
            coin.p2sh_addr_prefix)
            .or_else(|e| match Address::from_str(&address) {
                Ok(_) => ERR!("Cashaddress address format activated for {}, but legacy format used instead. Try to call 'convertaddress'", coin.ticker),
                Err(_) => ERR!("{}", e),
            })
    }
}

pub async fn get_current_mtp(coin: &UtxoCoinFields) -> Result<u32, String> {
    let current_block = try_s!(coin.rpc_client.get_block_count().compat().await);
    coin.rpc_client
        .get_median_time_past(current_block, coin.mtp_block_count)
        .compat()
        .await
}

pub fn send_outputs_from_my_address<T>(coin: T, outputs: Vec<TransactionOutput>) -> TransactionFut
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let fut = send_outputs_from_my_address_impl(coin, outputs);
    Box::new(fut.boxed().compat().map(|tx| tx.into()))
}

/// Generates unsigned transaction (TransactionInputSigner) from specified utxos and outputs.
/// This function expects that utxos are sorted by amounts in ascending order
/// Consider sorting before calling this function
/// Sends the change (inputs amount - outputs amount) to "my_address"
/// Also returns additional transaction data
///
/// Note `gas_fee` should be enough to execute all of the contract calls within UTXO outputs.
/// QRC20 specific: `gas_fee` should be calculated by: gas_limit * gas_price * (count of contract calls),
/// or should be sum of gas fee of all contract calls.
pub async fn generate_transaction<T>(
    coin: &T,
    utxos: Vec<UnspentInfo>,
    outputs: Vec<TransactionOutput>,
    fee_policy: FeePolicy,
    fee: Option<ActualTxFee>,
    gas_fee: Option<u64>,
) -> Result<(TransactionInputSigner, AdditionalTxData), GenerateTransactionError>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    macro_rules! try_other {
        ($exp: expr) => {
            match $exp {
                Ok(x) => x,
                Err(e) => {
                    let err = format!("{}", e);
                    return Err(GenerateTransactionError::Other(err));
                },
            }
        };
    }

    let dust: u64 = coin.as_ref().dust_amount;
    let lock_time = (now_ms() / 1000) as u32;
    let change_script_pubkey = Builder::build_p2pkh(&coin.as_ref().my_address.hash).to_bytes();
    let coin_tx_fee = match fee {
        Some(f) => f,
        None => try_other!(coin.get_tx_fee().await),
    };
    true_or!(!utxos.is_empty(), GenerateTransactionError::EmptyUtxoSet);
    true_or!(!outputs.is_empty(), GenerateTransactionError::EmptyOutputs);

    let mut sum_outputs_value = 0;
    let mut received_by_me = 0;
    for output in outputs.iter() {
        let script: Script = output.script_pubkey.clone().into();
        if script.opcodes().next() != Some(Ok(Opcode::OP_RETURN)) {
            true_or!(
                output.value >= dust,
                GenerateTransactionError::OutputValueLessThanDust {
                    value: output.value,
                    dust
                }
            );
        }
        sum_outputs_value += output.value;
        if output.script_pubkey == change_script_pubkey {
            received_by_me += output.value;
        }
    }

    if let Some(gas_fee) = gas_fee {
        sum_outputs_value = sum_outputs_value
            .checked_add(gas_fee)
            .ok_or(GenerateTransactionError::TooLargeGasFee)?;
    }

    let str_d_zeel = if coin.as_ref().ticker == "NAV" {
        Some("".into())
    } else {
        None
    };
    let mut tx = TransactionInputSigner {
        inputs: vec![],
        outputs,
        lock_time,
        version: coin.as_ref().tx_version,
        n_time: if coin.as_ref().is_pos {
            Some((now_ms() / 1000) as u32)
        } else {
            None
        },
        overwintered: coin.as_ref().overwintered,
        expiry_height: 0,
        join_splits: vec![],
        shielded_spends: vec![],
        shielded_outputs: vec![],
        value_balance: 0,
        version_group_id: coin.as_ref().version_group_id,
        consensus_branch_id: coin.as_ref().consensus_branch_id,
        zcash: coin.as_ref().zcash,
        str_d_zeel,
    };
    let mut sum_inputs = 0;
    let mut tx_fee = 0;
    let min_relay_fee = if coin.as_ref().force_min_relay_fee {
        let fee_dec = try_other!(coin.as_ref().rpc_client.get_relay_fee().compat().await);
        Some(try_other!(sat_from_big_decimal(&fee_dec, coin.as_ref().decimals)))
    } else {
        None
    };
    for utxo in utxos.iter() {
        sum_inputs += utxo.value;
        tx.inputs.push(UnsignedTransactionInput {
            previous_output: utxo.outpoint.clone(),
            sequence: SEQUENCE_FINAL,
            amount: utxo.value,
        });
        tx_fee = match &coin_tx_fee {
            ActualTxFee::Fixed(f) => *f,
            ActualTxFee::Dynamic(f) => {
                let transaction = UtxoTx::from(tx.clone());
                let transaction_bytes = serialize(&transaction);
                // 2 bytes are used to indicate the length of signature and pubkey
                // total is 107
                let additional_len = 2 + MAX_DER_SIGNATURE_LEN + COMPRESSED_PUBKEY_LEN;
                let tx_size = transaction_bytes.len() + transaction.inputs().len() * additional_len;
                (f * tx_size as u64) / KILO_BYTE
            },
        };

        match fee_policy {
            FeePolicy::SendExact => {
                let mut outputs_plus_fee = sum_outputs_value + tx_fee;
                if sum_inputs >= outputs_plus_fee {
                    if sum_inputs - outputs_plus_fee > dust {
                        // there will be change output if sum_inputs - outputs_plus_fee > dust
                        if let ActualTxFee::Dynamic(ref f) = coin_tx_fee {
                            tx_fee += (f * P2PKH_OUTPUT_LEN) / KILO_BYTE;
                            outputs_plus_fee += (f * P2PKH_OUTPUT_LEN) / KILO_BYTE;
                        }
                    }
                    if let Some(min_relay) = min_relay_fee {
                        if tx_fee < min_relay {
                            outputs_plus_fee -= tx_fee;
                            outputs_plus_fee += min_relay;
                            tx_fee = min_relay;
                        }
                    }
                    if sum_inputs >= outputs_plus_fee {
                        break;
                    }
                }
            },
            FeePolicy::DeductFromOutput(_) => {
                if sum_inputs >= sum_outputs_value {
                    if sum_inputs - sum_outputs_value > dust {
                        if let ActualTxFee::Dynamic(ref f) = coin_tx_fee {
                            tx_fee += (f * P2PKH_OUTPUT_LEN) / KILO_BYTE;
                        }
                        if let Some(min_relay) = min_relay_fee {
                            if tx_fee < min_relay {
                                tx_fee = min_relay;
                            }
                        }
                    }
                    break;
                }
            },
        };
    }
    match fee_policy {
        FeePolicy::SendExact => sum_outputs_value += tx_fee,
        FeePolicy::DeductFromOutput(i) => {
            let min_output = tx_fee + dust;
            let val = tx.outputs[i].value;
            true_or!(val >= min_output, GenerateTransactionError::DeductFeeFromOutputFailed {
                description: format!(
                    "Output {} value {} is too small, required no less than {}",
                    i, val, min_output
                ),
            });
            tx.outputs[i].value -= tx_fee;
            if tx.outputs[i].script_pubkey == change_script_pubkey {
                received_by_me -= tx_fee;
            }
        },
    };
    true_or!(
        sum_inputs >= sum_outputs_value,
        GenerateTransactionError::NotSufficientBalance {
            description: format!(
                "Couldn't collect enough value from utxos {:?} to create tx with outputs {:?}",
                utxos, tx.outputs
            )
        }
    );

    let change = sum_inputs - sum_outputs_value;
    if change >= dust {
        tx.outputs.push({
            TransactionOutput {
                value: change,
                script_pubkey: change_script_pubkey.clone(),
            }
        });
        received_by_me += change;
    } else {
        tx_fee += change;
    }

    let data = AdditionalTxData {
        fee_amount: tx_fee,
        received_by_me,
        spent_by_me: sum_inputs,
    };

    Ok(try_other!(
        coin.calc_interest_if_required(tx, data, change_script_pubkey).await
    ))
}

/// Calculates interest if the coin is KMD
/// Adds the value to existing output to my_script_pub or creates additional interest output
/// returns transaction and data as is if the coin is not KMD
pub async fn calc_interest_if_required<T>(
    coin: &T,
    mut unsigned: TransactionInputSigner,
    mut data: AdditionalTxData,
    my_script_pub: Bytes,
) -> Result<(TransactionInputSigner, AdditionalTxData), String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    if coin.as_ref().ticker != "KMD" {
        return Ok((unsigned, data));
    }
    unsigned.lock_time = try_s!(coin.get_current_mtp().await);
    let mut interest = 0;
    for input in unsigned.inputs.iter() {
        let prev_hash = input.previous_output.hash.reversed().into();
        let tx = try_s!(
            coin.as_ref()
                .rpc_client
                .get_verbose_transaction(prev_hash)
                .compat()
                .await
        );
        if let Ok(output_interest) =
            kmd_interest(tx.height, input.amount, tx.locktime as u64, unsigned.lock_time as u64)
        {
            interest += output_interest;
        };
    }
    if interest > 0 {
        data.received_by_me += interest;
        let mut output_to_me = unsigned
            .outputs
            .iter_mut()
            .find(|out| out.script_pubkey == my_script_pub);
        // add calculated interest to existing output to my address
        // or create the new one if it's not found
        match output_to_me {
            Some(ref mut output) => output.value += interest,
            None => {
                let interest_output = TransactionOutput {
                    script_pubkey: my_script_pub,
                    value: interest,
                };
                unsigned.outputs.push(interest_output);
            },
        };
    } else {
        // if interest is zero attempt to set the lowest possible lock_time to claim it later
        unsigned.lock_time = (now_ms() / 1000) as u32 - 3600 + 777 * 2;
    }
    Ok((unsigned, data))
}

pub fn p2sh_spending_tx(
    coin: &UtxoCoinFields,
    prev_transaction: UtxoTx,
    redeem_script: Bytes,
    outputs: Vec<TransactionOutput>,
    script_data: Script,
    sequence: u32,
) -> Result<UtxoTx, String> {
    // https://github.com/bitcoin/bitcoin/blob/master/doc/release-notes/release-notes-0.11.2.md#bip113-mempool-only-locktime-enforcement-using-getmediantimepast
    // Implication for users: GetMedianTimePast() always trails behind the current time,
    // so a transaction locktime set to the present time will be rejected by nodes running this
    // release until the median time moves forward.
    // To compensate, subtract one hour (3,600 seconds) from your locktimes to allow those
    // transactions to be included in mempools at approximately the expected time.
    let lock_time = if coin.ticker == "KMD" {
        (now_ms() / 1000) as u32 - 3600 + 2 * 777
    } else {
        (now_ms() / 1000) as u32 - 3600
    };
    let n_time = if coin.is_pos {
        Some((now_ms() / 1000) as u32)
    } else {
        None
    };
    let str_d_zeel = if coin.ticker == "NAV" { Some("".into()) } else { None };
    let unsigned = TransactionInputSigner {
        lock_time,
        version: coin.tx_version,
        n_time,
        overwintered: coin.overwintered,
        inputs: vec![UnsignedTransactionInput {
            sequence,
            previous_output: OutPoint {
                hash: prev_transaction.hash(),
                index: 0,
            },
            amount: prev_transaction.outputs[0].value,
        }],
        outputs: outputs.clone(),
        expiry_height: 0,
        join_splits: vec![],
        shielded_spends: vec![],
        shielded_outputs: vec![],
        value_balance: 0,
        version_group_id: coin.version_group_id,
        consensus_branch_id: coin.consensus_branch_id,
        zcash: coin.zcash,
        str_d_zeel,
    };
    let signed_input = try_s!(p2sh_spend(
        &unsigned,
        0,
        &coin.key_pair,
        script_data,
        redeem_script.into(),
        coin.signature_version,
        coin.fork_id
    ));
    Ok(UtxoTx {
        version: unsigned.version,
        n_time: unsigned.n_time,
        overwintered: unsigned.overwintered,
        lock_time: unsigned.lock_time,
        inputs: vec![signed_input],
        outputs,
        expiry_height: unsigned.expiry_height,
        join_splits: vec![],
        shielded_spends: vec![],
        shielded_outputs: vec![],
        value_balance: 0,
        version_group_id: coin.version_group_id,
        binding_sig: H512::default(),
        join_split_sig: H512::default(),
        join_split_pubkey: H256::default(),
        zcash: coin.zcash,
        str_d_zeel: unsigned.str_d_zeel,
    })
}

pub fn send_taker_fee<T>(coin: T, fee_pub_key: &[u8], amount: BigDecimal) -> TransactionFut
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let address = try_fus!(address_from_raw_pubkey(
        fee_pub_key,
        coin.as_ref().pub_addr_prefix,
        coin.as_ref().pub_t_addr_prefix,
        coin.as_ref().checksum_type
    ));
    let amount = try_fus!(sat_from_big_decimal(&amount, coin.as_ref().decimals));
    let output = TransactionOutput {
        value: amount,
        script_pubkey: Builder::build_p2pkh(&address.hash).to_bytes(),
    };
    send_outputs_from_my_address(coin, vec![output])
}

pub fn send_maker_payment<T>(
    coin: T,
    time_lock: u32,
    taker_pub: &[u8],
    secret_hash: &[u8],
    amount: BigDecimal,
) -> TransactionFut
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Clone + Send + Sync + 'static,
{
    let redeem_script = payment_script(
        time_lock,
        secret_hash,
        coin.as_ref().key_pair.public(),
        &try_fus!(Public::from_slice(taker_pub)),
    );
    let amount = try_fus!(sat_from_big_decimal(&amount, coin.as_ref().decimals));
    let htlc_out = TransactionOutput {
        value: amount,
        script_pubkey: Builder::build_p2sh(&dhash160(&redeem_script)).into(),
    };
    // record secret hash to blockchain too making it impossible to lose
    // lock time may be easily brute forced so it is not mandatory to record it
    let secret_hash_op_return_script = Builder::default()
        .push_opcode(Opcode::OP_RETURN)
        .push_bytes(secret_hash)
        .into_bytes();
    let secret_hash_op_return_out = TransactionOutput {
        value: 0,
        script_pubkey: secret_hash_op_return_script,
    };
    let send_fut = match &coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Electrum(_) => Either::A(send_outputs_from_my_address(coin, vec![
            htlc_out,
            secret_hash_op_return_out,
        ])),
        UtxoRpcClientEnum::Native(client) => {
            let payment_addr = Address {
                checksum_type: coin.as_ref().checksum_type,
                hash: dhash160(&redeem_script),
                prefix: coin.as_ref().p2sh_addr_prefix,
                t_addr_prefix: coin.as_ref().p2sh_t_addr_prefix,
            };
            let addr_string = try_fus!(coin.display_address(&payment_addr));
            Either::B(
                client
                    .import_address(&addr_string, &addr_string, false)
                    .map_err(|e| ERRL!("{}", e))
                    .and_then(move |_| send_outputs_from_my_address(coin, vec![htlc_out, secret_hash_op_return_out])),
            )
        },
    };
    Box::new(send_fut)
}

pub fn send_taker_payment<T>(
    coin: T,
    time_lock: u32,
    maker_pub: &[u8],
    secret_hash: &[u8],
    amount: BigDecimal,
) -> TransactionFut
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Clone + Send + Sync + 'static,
{
    let redeem_script = payment_script(
        time_lock,
        secret_hash,
        coin.as_ref().key_pair.public(),
        &try_fus!(Public::from_slice(maker_pub)),
    );

    let amount = try_fus!(sat_from_big_decimal(&amount, coin.as_ref().decimals));

    let htlc_out = TransactionOutput {
        value: amount,
        script_pubkey: Builder::build_p2sh(&dhash160(&redeem_script)).into(),
    };
    // record secret hash to blockchain too making it impossible to lose
    // lock time may be easily brute forced so it is not mandatory to record it
    let secret_hash_op_return_script = Builder::default()
        .push_opcode(Opcode::OP_RETURN)
        .push_bytes(secret_hash)
        .into_bytes();
    let secret_hash_op_return_out = TransactionOutput {
        value: 0,
        script_pubkey: secret_hash_op_return_script,
    };
    let send_fut = match &coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Electrum(_) => Either::A(send_outputs_from_my_address(coin, vec![
            htlc_out,
            secret_hash_op_return_out,
        ])),
        UtxoRpcClientEnum::Native(client) => {
            let payment_addr = Address {
                checksum_type: coin.as_ref().checksum_type,
                hash: dhash160(&redeem_script),
                prefix: coin.as_ref().p2sh_addr_prefix,
                t_addr_prefix: coin.as_ref().p2sh_t_addr_prefix,
            };
            let addr_string = try_fus!(coin.display_address(&payment_addr));
            Either::B(
                client
                    .import_address(&addr_string, &addr_string, false)
                    .map_err(|e| ERRL!("{}", e))
                    .and_then(move |_| send_outputs_from_my_address(coin, vec![htlc_out, secret_hash_op_return_out])),
            )
        },
    };
    Box::new(send_fut)
}

pub fn send_maker_spends_taker_payment<T>(
    coin: T,
    taker_payment_tx: &[u8],
    time_lock: u32,
    taker_pub: &[u8],
    secret: &[u8],
) -> TransactionFut
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let prev_tx: UtxoTx = try_fus!(deserialize(taker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
    let script_data = Builder::default()
        .push_data(secret)
        .push_opcode(Opcode::OP_0)
        .into_script();
    let redeem_script = payment_script(
        time_lock,
        &*dhash160(secret),
        &try_fus!(Public::from_slice(taker_pub)),
        coin.as_ref().key_pair.public(),
    );
    let fut = async move {
        let fee = try_s!(coin.get_htlc_spend_fee().await);
        let output = TransactionOutput {
            value: prev_tx.outputs[0].value - fee,
            script_pubkey: Builder::build_p2pkh(&coin.as_ref().key_pair.public().address_hash()).to_bytes(),
        };
        let transaction =
            try_s!(coin.p2sh_spending_tx(prev_tx, redeem_script.into(), vec![output], script_data, SEQUENCE_FINAL,));
        let tx_fut = coin.as_ref().rpc_client.send_transaction(&transaction).compat();
        try_s!(tx_fut.await);
        Ok(transaction.into())
    };
    Box::new(fut.boxed().compat())
}

pub fn send_taker_spends_maker_payment<T>(
    coin: T,
    maker_payment_tx: &[u8],
    time_lock: u32,
    maker_pub: &[u8],
    secret: &[u8],
) -> TransactionFut
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let prev_tx: UtxoTx = try_fus!(deserialize(maker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
    let script_data = Builder::default()
        .push_data(secret)
        .push_opcode(Opcode::OP_0)
        .into_script();
    let redeem_script = payment_script(
        time_lock,
        &*dhash160(secret),
        &try_fus!(Public::from_slice(maker_pub)),
        coin.as_ref().key_pair.public(),
    );
    let fut = async move {
        let fee = try_s!(coin.get_htlc_spend_fee().await);
        let output = TransactionOutput {
            value: prev_tx.outputs[0].value - fee,
            script_pubkey: Builder::build_p2pkh(&coin.as_ref().key_pair.public().address_hash()).to_bytes(),
        };
        let transaction =
            try_s!(coin.p2sh_spending_tx(prev_tx, redeem_script.into(), vec![output], script_data, SEQUENCE_FINAL,));
        let tx_fut = coin.as_ref().rpc_client.send_transaction(&transaction).compat();
        try_s!(tx_fut.await);
        Ok(transaction.into())
    };
    Box::new(fut.boxed().compat())
}

pub fn send_taker_refunds_payment<T>(
    coin: T,
    taker_payment_tx: &[u8],
    time_lock: u32,
    maker_pub: &[u8],
    secret_hash: &[u8],
) -> TransactionFut
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let prev_tx: UtxoTx = try_fus!(deserialize(taker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
    let script_data = Builder::default().push_opcode(Opcode::OP_1).into_script();
    let redeem_script = payment_script(
        time_lock,
        secret_hash,
        coin.as_ref().key_pair.public(),
        &try_fus!(Public::from_slice(maker_pub)),
    );
    let fut = async move {
        let fee = try_s!(coin.get_htlc_spend_fee().await);
        let output = TransactionOutput {
            value: prev_tx.outputs[0].value - fee,
            script_pubkey: Builder::build_p2pkh(&coin.as_ref().key_pair.public().address_hash()).to_bytes(),
        };
        let transaction = try_s!(coin.p2sh_spending_tx(
            prev_tx,
            redeem_script.into(),
            vec![output],
            script_data,
            SEQUENCE_FINAL - 1,
        ));
        let tx_fut = coin.as_ref().rpc_client.send_transaction(&transaction).compat();
        try_s!(tx_fut.await);
        Ok(transaction.into())
    };
    Box::new(fut.boxed().compat())
}

pub fn send_maker_refunds_payment<T>(
    coin: T,
    maker_payment_tx: &[u8],
    time_lock: u32,
    taker_pub: &[u8],
    secret_hash: &[u8],
) -> TransactionFut
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let prev_tx: UtxoTx = try_fus!(deserialize(maker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
    let script_data = Builder::default().push_opcode(Opcode::OP_1).into_script();
    let redeem_script = payment_script(
        time_lock,
        secret_hash,
        coin.as_ref().key_pair.public(),
        &try_fus!(Public::from_slice(taker_pub)),
    );
    let fut = async move {
        let fee = try_s!(coin.get_htlc_spend_fee().await);
        let output = TransactionOutput {
            value: prev_tx.outputs[0].value - fee,
            script_pubkey: Builder::build_p2pkh(&coin.as_ref().key_pair.public().address_hash()).to_bytes(),
        };
        let transaction = try_s!(coin.p2sh_spending_tx(
            prev_tx,
            redeem_script.into(),
            vec![output],
            script_data,
            SEQUENCE_FINAL - 1,
        ));
        let tx_fut = coin.as_ref().rpc_client.send_transaction(&transaction).compat();
        try_s!(tx_fut.await);
        Ok(transaction.into())
    };
    Box::new(fut.boxed().compat())
}

pub fn validate_fee<T>(
    coin: T,
    fee_tx: &TransactionEnum,
    fee_addr: &[u8],
    amount: &BigDecimal,
) -> Box<dyn Future<Item = (), Error = String> + Send>
where
    T: AsRef<UtxoCoinFields> + Send + Sync + 'static,
{
    let tx = match fee_tx {
        TransactionEnum::UtxoTx(tx) => tx.clone(),
        _ => panic!(),
    };
    let amount = amount.clone();
    let address = try_fus!(address_from_raw_pubkey(
        fee_addr,
        coin.as_ref().pub_addr_prefix,
        coin.as_ref().pub_t_addr_prefix,
        coin.as_ref().checksum_type
    ));

    let fut = async move {
        let amount = try_s!(sat_from_big_decimal(&amount, coin.as_ref().decimals));
        let tx_from_rpc = try_s!(
            coin.as_ref()
                .rpc_client
                .get_transaction_bytes(tx.hash().reversed().into())
                .compat()
                .await
        );

        if tx_from_rpc.0 != serialize(&tx).take() {
            return ERR!(
                "Provided dex fee tx {:?} doesn't match tx data from rpc {:?}",
                tx,
                tx_from_rpc
            );
        }

        match tx.outputs.first() {
            Some(out) => {
                let expected_script_pubkey = Builder::build_p2pkh(&address.hash).to_bytes();
                if out.script_pubkey != expected_script_pubkey {
                    return ERR!(
                        "Provided dex fee tx output script_pubkey doesn't match expected {:?} {:?}",
                        out.script_pubkey,
                        expected_script_pubkey
                    );
                }
                if out.value < amount {
                    return ERR!(
                        "Provided dex fee tx output value is less than expected {:?} {:?}",
                        out.value,
                        amount
                    );
                }
            },
            None => {
                return ERR!("Provided dex fee tx {:?} has no outputs", tx);
            },
        }
        Ok(())
    };
    Box::new(fut.boxed().compat())
}

pub fn validate_maker_payment<T>(
    coin: &T,
    payment_tx: &[u8],
    time_lock: u32,
    maker_pub: &[u8],
    priv_bn_hash: &[u8],
    amount: BigDecimal,
) -> Box<dyn Future<Item = (), Error = String> + Send>
where
    T: AsRef<UtxoCoinFields> + Clone + Send + Sync + 'static,
{
    let my_public = coin.as_ref().key_pair.public();
    validate_payment(
        coin.clone(),
        payment_tx,
        time_lock,
        &try_fus!(Public::from_slice(maker_pub)),
        my_public,
        priv_bn_hash,
        amount,
    )
}

pub fn validate_taker_payment<T>(
    coin: &T,
    payment_tx: &[u8],
    time_lock: u32,
    taker_pub: &[u8],
    priv_bn_hash: &[u8],
    amount: BigDecimal,
) -> Box<dyn Future<Item = (), Error = String> + Send>
where
    T: AsRef<UtxoCoinFields> + Clone + Send + Sync + 'static,
{
    let my_public = coin.as_ref().key_pair.public();
    validate_payment(
        coin.clone(),
        payment_tx,
        time_lock,
        &try_fus!(Public::from_slice(taker_pub)),
        my_public,
        priv_bn_hash,
        amount,
    )
}

pub fn check_if_my_payment_sent<T>(
    coin: T,
    time_lock: u32,
    other_pub: &[u8],
    secret_hash: &[u8],
) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let script = payment_script(
        time_lock,
        secret_hash,
        coin.as_ref().key_pair.public(),
        &try_fus!(Public::from_slice(other_pub)),
    );
    let hash = dhash160(&script);
    let p2sh = Builder::build_p2sh(&hash);
    let script_hash = electrum_script_hash(&p2sh);
    let fut = async move {
        match &coin.as_ref().rpc_client {
            UtxoRpcClientEnum::Electrum(client) => {
                let history = try_s!(client.scripthash_get_history(&hex::encode(script_hash)).compat().await);
                match history.first() {
                    Some(item) => {
                        let tx_bytes = try_s!(client.get_transaction_bytes(item.tx_hash.clone()).compat().await);
                        let tx: UtxoTx = try_s!(deserialize(tx_bytes.0.as_slice()).map_err(|e| ERRL!("{:?}", e)));
                        Ok(Some(tx.into()))
                    },
                    None => Ok(None),
                }
            },
            UtxoRpcClientEnum::Native(client) => {
                let target_addr = Address {
                    t_addr_prefix: coin.as_ref().p2sh_t_addr_prefix,
                    prefix: coin.as_ref().p2sh_addr_prefix,
                    hash,
                    checksum_type: coin.as_ref().checksum_type,
                };
                let target_addr = target_addr.to_string();
                let is_imported = try_s!(client.is_address_imported(&target_addr).await);
                if !is_imported {
                    return Ok(None);
                }
                let received_by_addr = try_s!(client.list_received_by_address(0, true, true).compat().await);
                for item in received_by_addr {
                    if item.address == target_addr && !item.txids.is_empty() {
                        let tx_bytes = try_s!(client.get_transaction_bytes(item.txids[0].clone()).compat().await);
                        let tx: UtxoTx = try_s!(deserialize(tx_bytes.0.as_slice()).map_err(|e| ERRL!("{:?}", e)));
                        return Ok(Some(tx.into()));
                    }
                }
                Ok(None)
            },
        }
    };
    Box::new(fut.boxed().compat())
}

pub fn search_for_swap_tx_spend_my(
    coin: &UtxoCoinFields,
    time_lock: u32,
    other_pub: &[u8],
    secret_hash: &[u8],
    tx: &[u8],
    search_from_block: u64,
) -> Result<Option<FoundSwapTxSpend>, String> {
    block_on(search_for_swap_tx_spend(
        coin,
        time_lock,
        coin.key_pair.public(),
        &try_s!(Public::from_slice(other_pub)),
        secret_hash,
        tx,
        search_from_block,
    ))
}

pub fn search_for_swap_tx_spend_other(
    coin: &UtxoCoinFields,
    time_lock: u32,
    other_pub: &[u8],
    secret_hash: &[u8],
    tx: &[u8],
    search_from_block: u64,
) -> Result<Option<FoundSwapTxSpend>, String> {
    block_on(search_for_swap_tx_spend(
        coin,
        time_lock,
        &try_s!(Public::from_slice(other_pub)),
        coin.key_pair.public(),
        secret_hash,
        tx,
        search_from_block,
    ))
}

/// Extract a secret from the `spend_tx`.
/// Note spender could generate the spend with several inputs where the only one input is the p2sh script.
pub fn extract_secret(secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
    let spend_tx: UtxoTx = try_s!(deserialize(spend_tx).map_err(|e| ERRL!("{:?}", e)));
    for (input_idx, input) in spend_tx.inputs.into_iter().enumerate() {
        let script: Script = input.script_sig.clone().into();
        let instruction = match script.get_instruction(1) {
            Some(Ok(instr)) => instr,
            Some(Err(e)) => {
                log!("Warning: "[e]);
                continue;
            },
            None => {
                log!("Warning: couldn't find secret in "[input_idx]" input");
                continue;
            },
        };

        if instruction.opcode != Opcode::OP_PUSHBYTES_32 {
            log!("Warning: expected "[Opcode::OP_PUSHBYTES_32]" opcode, found "[instruction.opcode] " in "[input_idx]" input");
            continue;
        }

        let secret = match instruction.data {
            Some(data) => data.to_vec(),
            None => {
                log!("Warning: secret is empty in "[input_idx] " input");
                continue;
            },
        };

        let actual_secret_hash = &*dhash160(&secret);
        if actual_secret_hash != secret_hash {
            log!("Warning: invalid 'dhash160(secret)' "[actual_secret_hash]", expected "[secret_hash]);
            continue;
        }
        return Ok(secret);
    }
    ERR!("Couldn't extract secret")
}

pub fn my_address<T>(coin: &T) -> Result<String, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    coin.display_address(&coin.as_ref().my_address)
}

pub fn my_balance(coin: &UtxoCoinFields) -> Box<dyn Future<Item = BigDecimal, Error = String> + Send> {
    Box::new(
        coin.rpc_client
            .display_balance(coin.my_address.clone(), coin.decimals)
            .map_err(|e| ERRL!("{}", e)),
    )
}

pub fn send_raw_tx(coin: &UtxoCoinFields, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
    let bytes = try_fus!(hex::decode(tx));
    Box::new(
        coin.rpc_client
            .send_raw_transaction(bytes.into())
            .map_err(|e| ERRL!("{}", e))
            .map(|hash| format!("{:?}", hash)),
    )
}

pub fn wait_for_confirmations(
    coin: &UtxoCoinFields,
    tx: &[u8],
    confirmations: u64,
    requires_nota: bool,
    wait_until: u64,
    check_every: u64,
) -> Box<dyn Future<Item = (), Error = String> + Send> {
    let tx: UtxoTx = try_fus!(deserialize(tx).map_err(|e| ERRL!("{:?}", e)));
    coin.rpc_client
        .wait_for_confirmations(&tx, confirmations as u32, requires_nota, wait_until, check_every)
}

pub fn wait_for_tx_spend(coin: &UtxoCoinFields, tx_bytes: &[u8], wait_until: u64, from_block: u64) -> TransactionFut {
    let tx: UtxoTx = try_fus!(deserialize(tx_bytes).map_err(|e| ERRL!("{:?}", e)));
    let vout = 0;
    let client = coin.rpc_client.clone();
    let fut = async move {
        loop {
            match client.find_output_spend(&tx, vout, from_block).compat().await {
                Ok(Some(tx)) => return Ok(tx.into()),
                Ok(None) => (),
                Err(e) => {
                    log!("Error " (e) " on find_output_spend of tx " [e]);
                },
            };

            if now_ms() / 1000 > wait_until {
                return ERR!(
                    "Waited too long until {} for transaction {:?} {} to be spent ",
                    wait_until,
                    tx,
                    vout
                );
            }
            Timer::sleep(10.).await;
        }
    };
    Box::new(fut.boxed().compat())
}

pub fn tx_enum_from_bytes(bytes: &[u8]) -> Result<TransactionEnum, String> {
    let transaction: UtxoTx = try_s!(deserialize(bytes).map_err(|err| format!("{:?}", err)));
    Ok(transaction.into())
}

pub fn current_block(coin: &UtxoCoinFields) -> Box<dyn Future<Item = u64, Error = String> + Send> {
    Box::new(coin.rpc_client.get_block_count().map_err(|e| ERRL!("{}", e)))
}

pub fn address_from_pubkey_str<T>(coin: &T, pubkey: &str) -> Result<String, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    let pubkey_bytes = try_s!(hex::decode(pubkey));
    let addr = try_s!(address_from_raw_pubkey(
        &pubkey_bytes,
        coin.as_ref().pub_addr_prefix,
        coin.as_ref().pub_t_addr_prefix,
        coin.as_ref().checksum_type
    ));
    coin.display_address(&addr)
}

pub fn display_priv_key(coin: &UtxoCoinFields) -> String { format!("{}", coin.key_pair.private()) }

pub fn is_asset_chain(coin: &UtxoCoinFields) -> bool { coin.asset_chain }

pub fn can_i_spend_other_payment() -> Box<dyn Future<Item = (), Error = String> + Send> {
    Box::new(futures01::future::ok(()))
}

pub async fn withdraw<T>(coin: T, req: WithdrawRequest) -> Result<TransactionDetails, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + MarketCoinOps,
{
    let to = try_s!(coin.address_from_str(&req.to));

    let is_p2pkh = to.prefix == coin.as_ref().pub_addr_prefix && to.t_addr_prefix == coin.as_ref().pub_t_addr_prefix;
    let is_p2sh = to.prefix == coin.as_ref().p2sh_addr_prefix
        && to.t_addr_prefix == coin.as_ref().p2sh_t_addr_prefix
        && coin.as_ref().segwit;

    let script_pubkey = if is_p2pkh {
        Builder::build_p2pkh(&to.hash)
    } else if is_p2sh {
        Builder::build_p2sh(&to.hash)
    } else {
        return ERR!("Address {} has invalid format", to);
    };

    if to.checksum_type != coin.as_ref().checksum_type {
        return ERR!(
            "Address {} has invalid checksum type, it must be {:?}",
            to,
            coin.as_ref().checksum_type
        );
    }

    let script_pubkey = script_pubkey.to_bytes();

    let _utxo_lock = UTXO_LOCK.lock().await;
    let unspents = try_s!(coin
        .ordered_mature_unspents(&coin.as_ref().my_address)
        .compat()
        .await
        .map_err(|e| ERRL!("{}", e)));
    let (value, fee_policy) = if req.max {
        (
            unspents.iter().fold(0, |sum, unspent| sum + unspent.value),
            FeePolicy::DeductFromOutput(0),
        )
    } else {
        (
            try_s!(sat_from_big_decimal(&req.amount, coin.as_ref().decimals)),
            FeePolicy::SendExact,
        )
    };
    let outputs = vec![TransactionOutput { value, script_pubkey }];
    let fee = match req.fee {
        Some(WithdrawFee::UtxoFixed { amount }) => Some(ActualTxFee::Fixed(try_s!(sat_from_big_decimal(
            &amount,
            coin.as_ref().decimals
        )))),
        Some(WithdrawFee::UtxoPerKbyte { amount }) => Some(ActualTxFee::Dynamic(try_s!(sat_from_big_decimal(
            &amount,
            coin.as_ref().decimals
        )))),
        Some(_) => return ERR!("Unsupported input fee type"),
        None => None,
    };
    let gas_fee = None;
    let (unsigned, data) = try_s!(
        coin.generate_transaction(unspents, outputs, fee_policy, fee, gas_fee)
            .await
    );
    let prev_script = Builder::build_p2pkh(&coin.as_ref().my_address.hash);
    let signed = try_s!(sign_tx(
        unsigned,
        &coin.as_ref().key_pair,
        prev_script,
        coin.as_ref().signature_version,
        coin.as_ref().fork_id
    ));
    let fee_details = UtxoFeeDetails {
        amount: big_decimal_from_sat(data.fee_amount as i64, coin.as_ref().decimals),
    };
    let my_address = try_s!(coin.my_address());
    let to_address = try_s!(coin.display_address(&to));
    Ok(TransactionDetails {
        from: vec![my_address],
        to: vec![to_address],
        total_amount: big_decimal_from_sat(data.spent_by_me as i64, coin.as_ref().decimals),
        spent_by_me: big_decimal_from_sat(data.spent_by_me as i64, coin.as_ref().decimals),
        received_by_me: big_decimal_from_sat(data.received_by_me as i64, coin.as_ref().decimals),
        my_balance_change: big_decimal_from_sat(
            data.received_by_me as i64 - data.spent_by_me as i64,
            coin.as_ref().decimals,
        ),
        tx_hash: signed.hash().reversed().to_vec().into(),
        tx_hex: serialize(&signed).into(),
        fee_details: Some(fee_details.into()),
        block_height: 0,
        coin: coin.as_ref().ticker.clone(),
        internal_id: vec![].into(),
        timestamp: now_ms() / 1000,
    })
}

pub fn decimals(coin: &UtxoCoinFields) -> u8 { coin.decimals }

pub fn convert_to_address<T>(coin: &T, from: &str, to_address_format: Json) -> Result<String, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    let to_address_format: UtxoAddressFormat =
        json::from_value(to_address_format).map_err(|e| ERRL!("Error on parse UTXO address format {:?}", e))?;
    let from_address = try_s!(address_from_any_format(&coin.as_ref(), from));
    match to_address_format {
        UtxoAddressFormat::Standard => Ok(from_address.to_string()),
        UtxoAddressFormat::CashAddress { network } => Ok(try_s!(from_address
            .to_cashaddress(&network, coin.as_ref().pub_addr_prefix, coin.as_ref().p2sh_addr_prefix)
            .and_then(|cashaddress| cashaddress.encode()))),
    }
}

pub fn validate_address<T>(coin: &T, address: &str) -> ValidateAddressResult
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    let result = coin.address_from_str(address);
    let address = match result {
        Ok(addr) => addr,
        Err(e) => {
            return ValidateAddressResult {
                is_valid: false,
                reason: Some(e),
            }
        },
    };

    let is_p2pkh =
        address.prefix == coin.as_ref().pub_addr_prefix && address.t_addr_prefix == coin.as_ref().pub_t_addr_prefix;
    let is_p2sh = address.prefix == coin.as_ref().p2sh_addr_prefix
        && address.t_addr_prefix == coin.as_ref().p2sh_t_addr_prefix
        && coin.as_ref().segwit;

    if is_p2pkh || is_p2sh {
        ValidateAddressResult {
            is_valid: true,
            reason: None,
        }
    } else {
        ValidateAddressResult {
            is_valid: false,
            reason: Some(ERRL!("Address {} has invalid prefixes", address)),
        }
    }
}

#[allow(clippy::cognitive_complexity)]
pub fn process_history_loop<T>(coin: &T, ctx: MmArc)
where
    T: AsRef<UtxoCoinFields> + UtxoStandardOps + UtxoCommonOps + MmCoin + MarketCoinOps,
{
    let mut my_balance: Option<BigDecimal> = None;
    let history = coin.load_history_from_file(&ctx);
    let mut history_map: HashMap<H256Json, TransactionDetails> = history
        .into_iter()
        .map(|tx| (H256Json::from(tx.tx_hash.as_slice()), tx))
        .collect();

    let mut success_iteration = 0i32;
    loop {
        if ctx.is_stopping() {
            break;
        };
        {
            let coins_ctx = unwrap!(CoinsContext::from_ctx(&ctx));
            let coins = block_on(coins_ctx.coins.lock());
            if !coins.contains_key(&coin.as_ref().ticker) {
                ctx.log.log("", &[&"tx_history", &coin.as_ref().ticker], "Loop stopped");
                break;
            };
        }

        let actual_balance = match coin.my_balance().wait() {
            Ok(actual_balance) => Some(actual_balance),
            Err(err) => {
                ctx.log.log(
                    "",
                    &[&"tx_history", &coin.as_ref().ticker],
                    &ERRL!("Error {:?} on getting balance", err),
                );
                None
            },
        };

        let need_update = history_map
            .iter()
            .any(|(_, tx)| tx.should_update_timestamp() || tx.should_update_block_height());
        match (&my_balance, &actual_balance) {
            (Some(prev_balance), Some(actual_balance)) if prev_balance == actual_balance && !need_update => {
                // my balance hasn't been changed, there is no need to reload tx_history
                thread::sleep(Duration::from_secs(30));
                continue;
            },
            _ => (),
        }

        let tx_ids = match block_on(coin.request_tx_history(ctx.metrics.clone())) {
            RequestTxHistoryResult::Ok(tx_ids) => tx_ids,
            RequestTxHistoryResult::Retry { error } => {
                ctx.log.log(
                    "",
                    &[&"tx_history", &coin.as_ref().ticker],
                    &ERRL!("{}, retrying", error),
                );
                thread::sleep(Duration::from_secs(10));
                continue;
            },
            RequestTxHistoryResult::HistoryTooLarge => {
                ctx.log.log(
                    "",
                    &[&"tx_history", &coin.as_ref().ticker],
                    &ERRL!("Got `history too large`, stopping further attempts to retrieve it"),
                );
                *unwrap!(coin.as_ref().history_sync_state.lock()) = HistorySyncState::Error(json!({
                    "code": HISTORY_TOO_LARGE_ERR_CODE,
                    "message": "Got `history too large` error from Electrum server. History is not available",
                }));
                break;
            },
            RequestTxHistoryResult::UnknownError(e) => {
                ctx.log.log(
                    "",
                    &[&"tx_history", &coin.as_ref().ticker],
                    &ERRL!("{}, stopping futher attempts to retreive it", e),
                );
                break;
            },
        };
        let mut transactions_left = if tx_ids.len() > history_map.len() {
            *unwrap!(coin.as_ref().history_sync_state.lock()) = HistorySyncState::InProgress(json!({
                "transactions_left": tx_ids.len() - history_map.len()
            }));
            tx_ids.len() - history_map.len()
        } else {
            *unwrap!(coin.as_ref().history_sync_state.lock()) = HistorySyncState::InProgress(json!({
                "transactions_left": 0
            }));
            0
        };

        for (txid, height) in tx_ids {
            let mut updated = false;
            match history_map.entry(txid.clone()) {
                Entry::Vacant(e) => {
                    mm_counter!(ctx.metrics, "tx.history.request.count", 1, "coin" => coin.as_ref().ticker.clone(), "method" => "tx_detail_by_hash");

                    match block_on(coin.tx_details_by_hash(&txid.0)) {
                        Ok(mut tx_details) => {
                            mm_counter!(ctx.metrics, "tx.history.response.count", 1, "coin" => coin.as_ref().ticker.clone(), "method" => "tx_detail_by_hash");

                            if tx_details.block_height == 0 && height > 0 {
                                tx_details.block_height = height;
                            }

                            e.insert(tx_details);
                            if transactions_left > 0 {
                                transactions_left -= 1;
                                *unwrap!(coin.as_ref().history_sync_state.lock()) =
                                    HistorySyncState::InProgress(json!({ "transactions_left": transactions_left }));
                            }
                            updated = true;
                        },
                        Err(e) => ctx.log.log(
                            "",
                            &[&"tx_history", &coin.as_ref().ticker],
                            &ERRL!("Error {:?} on getting the details of {:?}, skipping the tx", e, txid),
                        ),
                    }
                },
                Entry::Occupied(mut e) => {
                    // update block height for previously unconfirmed transaction
                    if e.get().should_update_block_height() && height > 0 {
                        e.get_mut().block_height = height;
                        updated = true;
                    }
                    if e.get().should_update_timestamp() {
                        mm_counter!(ctx.metrics, "tx.history.request.count", 1, "coin" => coin.as_ref().ticker.clone(), "method" => "tx_detail_by_hash");

                        if let Ok(tx_details) = block_on(coin.tx_details_by_hash(&txid.0)) {
                            mm_counter!(ctx.metrics, "tx.history.response.count", 1, "coin" => coin.as_ref().ticker.clone(), "method" => "tx_detail_by_hash");

                            e.get_mut().timestamp = tx_details.timestamp;
                            updated = true;
                        }
                    }
                },
            }
            if updated {
                let mut to_write: Vec<&TransactionDetails> = history_map.iter().map(|(_, value)| value).collect();
                // the transactions with block_height == 0 are the most recent so we need to separately handle them while sorting
                to_write.sort_unstable_by(|a, b| {
                    if a.block_height == 0 {
                        Ordering::Less
                    } else if b.block_height == 0 {
                        Ordering::Greater
                    } else {
                        b.block_height.cmp(&a.block_height)
                    }
                });
                coin.save_history_to_file(&unwrap!(json::to_vec(&to_write)), &ctx);
            }
        }
        *unwrap!(coin.as_ref().history_sync_state.lock()) = HistorySyncState::Finished;

        if success_iteration == 0 {
            ctx.log.log(
                "😅",
                &[&"tx_history", &("coin", coin.as_ref().ticker.clone().as_str())],
                "history has been loaded successfully",
            );
        }

        my_balance = actual_balance;
        success_iteration += 1;
        thread::sleep(Duration::from_secs(30));
    }
}

pub async fn request_tx_history<T>(coin: &T, metrics: MetricsArc) -> RequestTxHistoryResult
where
    T: AsRef<UtxoCoinFields> + MmCoin + MarketCoinOps,
{
    let my_address = match coin.my_address() {
        Ok(addr) => addr,
        Err(e) => {
            return RequestTxHistoryResult::UnknownError(ERRL!("Error on getting self address: {}. Stop tx history", e))
        },
    };

    let tx_ids = match &coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(client) => {
            let mut from = 0;
            let mut all_transactions = vec![];
            loop {
                mm_counter!(metrics, "tx.history.request.count", 1,
                    "coin" => coin.as_ref().ticker.clone(), "client" => "native", "method" => "listtransactions");

                let transactions = match client.list_transactions(100, from).compat().await {
                    Ok(value) => value,
                    Err(e) => {
                        return RequestTxHistoryResult::Retry {
                            error: ERRL!("Error {} on list transactions", e),
                        };
                    },
                };

                mm_counter!(metrics, "tx.history.response.count", 1,
                    "coin" => coin.as_ref().ticker.clone(), "client" => "native", "method" => "listtransactions");

                if transactions.is_empty() {
                    break;
                }
                from += 100;
                all_transactions.extend(transactions);
            }

            mm_counter!(metrics, "tx.history.response.total_length", all_transactions.len() as u64,
                "coin" => coin.as_ref().ticker.clone(), "client" => "native", "method" => "listtransactions");

            all_transactions
                .into_iter()
                .filter_map(|item| {
                    if item.address == my_address {
                        Some((item.txid, item.blockindex))
                    } else {
                        None
                    }
                })
                .collect()
        },
        UtxoRpcClientEnum::Electrum(client) => {
            let script = Builder::build_p2pkh(&coin.as_ref().my_address.hash);
            let script_hash = electrum_script_hash(&script);

            mm_counter!(metrics, "tx.history.request.count", 1,
                "coin" => coin.as_ref().ticker.clone(), "client" => "electrum", "method" => "blockchain.scripthash.get_history");

            let electrum_history = match client.scripthash_get_history(&hex::encode(script_hash)).compat().await {
                Ok(value) => value,
                Err(e) => match &e.error {
                    JsonRpcErrorType::Transport(e) | JsonRpcErrorType::Parse(_, e) => {
                        return RequestTxHistoryResult::Retry {
                            error: ERRL!("Error {} on scripthash_get_history", e),
                        };
                    },
                    JsonRpcErrorType::Response(_addr, err) => {
                        if HISTORY_TOO_LARGE_ERROR.eq(err) {
                            return RequestTxHistoryResult::HistoryTooLarge;
                        } else {
                            return RequestTxHistoryResult::Retry {
                                error: ERRL!("Error {:?} on scripthash_get_history", e),
                            };
                        }
                    },
                },
            };
            mm_counter!(metrics, "tx.history.response.count", 1,
                "coin" => coin.as_ref().ticker.clone(), "client" => "electrum", "method" => "blockchain.scripthash.get_history");

            mm_counter!(metrics, "tx.history.response.total_length", electrum_history.len() as u64,
                "coin" => coin.as_ref().ticker.clone(), "client" => "electrum", "method" => "blockchain.scripthash.get_history");

            // electrum returns the most recent transactions in the end but we need to
            // process them first so rev is required
            electrum_history
                .into_iter()
                .rev()
                .map(|item| {
                    let height = if item.height < 0 { 0 } else { item.height as u64 };
                    (item.tx_hash, height)
                })
                .collect()
        },
    };
    RequestTxHistoryResult::Ok(tx_ids)
}

pub async fn tx_details_by_hash<T>(coin: &T, hash: &[u8]) -> Result<TransactionDetails, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let hash = H256Json::from(hash);
    let verbose_tx = try_s!(coin.as_ref().rpc_client.get_verbose_transaction(hash).compat().await);
    let tx: UtxoTx = try_s!(deserialize(verbose_tx.hex.as_slice()).map_err(|e| ERRL!("{:?}", e)));
    let mut input_transactions: HashMap<&H256, UtxoTx> = HashMap::new();
    let mut input_amount = 0;
    let mut output_amount = 0;
    let mut from_addresses = vec![];
    let mut to_addresses = vec![];
    let mut spent_by_me = 0;
    let mut received_by_me = 0;
    for input in tx.inputs.iter() {
        // input transaction is zero if the tx is the coinbase transaction
        if input.previous_output.hash.is_zero() {
            continue;
        }

        let input_tx = match input_transactions.entry(&input.previous_output.hash) {
            Entry::Vacant(e) => {
                let prev_hash = input.previous_output.hash.reversed();
                let prev: BytesJson = try_s!(
                    coin.as_ref()
                        .rpc_client
                        .get_transaction_bytes(prev_hash.clone().into())
                        .compat()
                        .await
                );
                let prev_tx: UtxoTx =
                    try_s!(deserialize(prev.as_slice()).map_err(|e| ERRL!("{:?}, tx: {:?}", e, prev_hash)));
                e.insert(prev_tx)
            },
            Entry::Occupied(e) => e.into_mut(),
        };
        input_amount += input_tx.outputs[input.previous_output.index as usize].value;
        let from: Vec<Address> = try_s!(coin.addresses_from_script(
            &input_tx.outputs[input.previous_output.index as usize]
                .script_pubkey
                .clone()
                .into()
        ));
        if from.contains(&coin.as_ref().my_address) {
            spent_by_me += input_tx.outputs[input.previous_output.index as usize].value;
        }
        from_addresses.push(from);
    }

    for output in tx.outputs.iter() {
        output_amount += output.value;
        let to = try_s!(coin.addresses_from_script(&output.script_pubkey.clone().into()));
        if to.contains(&coin.as_ref().my_address) {
            received_by_me += output.value;
        }
        to_addresses.push(to);
    }
    // remove address duplicates in case several inputs were spent from same address
    // or several outputs are sent to same address
    let mut from_addresses: Vec<String> = try_s!(from_addresses
        .into_iter()
        .flatten()
        .map(|addr| coin.display_address(&addr))
        .collect());
    from_addresses.sort();
    from_addresses.dedup();
    let mut to_addresses: Vec<String> = try_s!(to_addresses
        .into_iter()
        .flatten()
        .map(|addr| coin.display_address(&addr))
        .collect());
    to_addresses.sort();
    to_addresses.dedup();

    let fee = big_decimal_from_sat(input_amount as i64 - output_amount as i64, coin.as_ref().decimals);
    Ok(TransactionDetails {
        from: from_addresses,
        to: to_addresses,
        received_by_me: big_decimal_from_sat(received_by_me as i64, coin.as_ref().decimals),
        spent_by_me: big_decimal_from_sat(spent_by_me as i64, coin.as_ref().decimals),
        my_balance_change: big_decimal_from_sat(received_by_me as i64 - spent_by_me as i64, coin.as_ref().decimals),
        total_amount: big_decimal_from_sat(input_amount as i64, coin.as_ref().decimals),
        tx_hash: tx.hash().reversed().to_vec().into(),
        tx_hex: verbose_tx.hex,
        fee_details: Some(UtxoFeeDetails { amount: fee }.into()),
        block_height: verbose_tx.height.unwrap_or(0),
        coin: coin.as_ref().ticker.clone(),
        internal_id: tx.hash().reversed().to_vec().into(),
        timestamp: verbose_tx.time.into(),
    })
}

pub fn history_sync_status(coin: &UtxoCoinFields) -> HistorySyncState {
    unwrap!(coin.history_sync_state.lock()).clone()
}

pub fn get_trade_fee<T>(coin: T) -> Box<dyn Future<Item = TradeFee, Error = String> + Send>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    let ticker = coin.as_ref().ticker.clone();
    let decimals = coin.as_ref().decimals;
    let fut = async move {
        let fee = try_s!(coin.get_tx_fee().await);
        let amount = match fee {
            ActualTxFee::Fixed(f) => f,
            ActualTxFee::Dynamic(f) => f,
        };
        Ok(TradeFee {
            coin: ticker,
            amount: big_decimal_from_sat(amount as i64, decimals).into(),
        })
    };
    Box::new(fut.boxed().compat())
}

pub fn required_confirmations(coin: &UtxoCoinFields) -> u64 {
    coin.required_confirmations.load(AtomicOrderding::Relaxed)
}

pub fn requires_notarization(coin: &UtxoCoinFields) -> bool {
    coin.requires_notarization.load(AtomicOrderding::Relaxed)
}

pub fn set_required_confirmations(coin: &UtxoCoinFields, confirmations: u64) {
    coin.required_confirmations
        .store(confirmations, AtomicOrderding::Relaxed);
}

pub fn set_requires_notarization(coin: &UtxoCoinFields, requires_nota: bool) {
    coin.requires_notarization
        .store(requires_nota, AtomicOrderding::Relaxed);
}

pub async fn ordered_mature_unspents<T>(coin: T, address: Address) -> Result<Vec<UnspentInfo>, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    fn calc_actual_cached_tx_confirmations(tx: &RpcTransaction, block_count: u64) -> Result<u32, String> {
        let tx_height = tx
            .height
            .ok_or(ERRL!(r#"Warning, height of cached "{:?}" tx is unknown"#, tx.txid))?;
        // utxo_common::cache_transaction_if_possible() shouldn't cache transaction with height == 0
        if tx_height == 0 {
            return ERR!(
                r#"Warning, height of cached "{:?}" tx is expected to be non-zero"#,
                tx.txid
            );
        }
        if block_count < tx_height {
            return ERR!(
                r#"Warning, actual block_count {} less than cached tx_height {} of {:?}"#,
                block_count,
                tx_height,
                tx.txid
            );
        }

        let confirmations = block_count - tx_height + 1;
        Ok(confirmations as u32)
    }

    let (unspents, _) = try_s!(coin.list_unspent_ordered(&address).await);
    let block_count = try_s!(coin.as_ref().rpc_client.get_block_count().compat().await);

    let mut result = Vec::with_capacity(unspents.len());
    for unspent in unspents {
        let tx_hash: H256Json = unspent.outpoint.hash.reversed().into();
        let tx_info = match coin
            .get_verbose_transaction_from_cache_or_rpc(tx_hash.clone())
            .compat()
            .await
        {
            Ok(x) => x,
            Err(err) => {
                log!("Error " [err] " getting the transaction " [tx_hash] ", skip the unspent output");
                continue;
            },
        };

        let tx_info = match tx_info {
            VerboseTransactionFrom::Cache(mut tx) => {
                if unspent.height.is_some() {
                    tx.height = unspent.height;
                }
                match calc_actual_cached_tx_confirmations(&tx, block_count) {
                    Ok(conf) => tx.confirmations = conf,
                    // do not skip the transaction with unknown confirmations,
                    // because the transaction can be matured
                    Err(e) => log!((e)),
                }
                tx
            },
            VerboseTransactionFrom::Rpc(mut tx) => {
                if tx.height.is_none() {
                    tx.height = unspent.height;
                }
                if let Err(e) = coin.cache_transaction_if_possible(&tx).await {
                    log!((e));
                }
                tx
            },
        };

        if coin.is_unspent_mature(&tx_info) {
            result.push(unspent);
        }
    }

    Ok(result)
}

pub fn is_unspent_mature(mature_confirmations: u32, output: &RpcTransaction) -> bool {
    // don't skip outputs with confirmations == 0, because we can spend them
    !output.is_coinbase() || output.confirmations >= mature_confirmations
}

#[cfg(feature = "native")]
pub async fn get_verbose_transaction_from_cache_or_rpc(
    coin: &UtxoCoinFields,
    txid: H256Json,
) -> Result<VerboseTransactionFrom, String> {
    let tx_cache_path = match &coin.tx_cache_directory {
        Some(p) => p.clone(),
        _ => {
            // the coin doesn't support TX local cache, don't try to load from cache and don't cache it
            let tx = try_s!(coin.rpc_client.get_verbose_transaction(txid.clone()).compat().await);
            return Ok(VerboseTransactionFrom::Rpc(tx));
        },
    };

    match tx_cache::load_transaction_from_cache(&tx_cache_path, &txid).await {
        Ok(Some(tx)) => return Ok(VerboseTransactionFrom::Cache(tx)),
        Err(err) => log!("Error " [err] " loading the " [txid] " transaction. Try request tx using Rpc client"),
        // txid just not found
        _ => (),
    }

    let tx = try_s!(coin.rpc_client.get_verbose_transaction(txid).compat().await);
    Ok(VerboseTransactionFrom::Rpc(tx))
}

#[cfg(not(feature = "native"))]
pub async fn get_verbose_transaction_from_cache_or_rpc(
    coin: &UtxoCoinFields,
    txid: H256Json,
) -> Result<VerboseTransactionFrom, String> {
    let tx = try_s!(coin.rpc_client.get_verbose_transaction(txid.clone()).compat().await);
    Ok(VerboseTransactionFrom::Rpc(tx))
}

#[cfg(feature = "native")]
pub async fn cache_transaction_if_possible(coin: &UtxoCoinFields, tx: &RpcTransaction) -> Result<(), String> {
    let tx_cache_path = match &coin.tx_cache_directory {
        Some(p) => p.clone(),
        _ => {
            return Ok(());
        },
    };
    // check if the transaction height is set and not zero
    match tx.height {
        Some(0) => return Ok(()),
        Some(_) => (),
        None => return Ok(()),
    }

    tx_cache::cache_transaction(&tx_cache_path, &tx)
        .await
        .map_err(|e| ERRL!("Error {:?} on caching transaction {:?}", e, tx.txid))
}

#[cfg(not(feature = "native"))]
pub async fn cache_transaction_if_possible(_coin: &UtxoCoinFields, _tx: &RpcTransaction) -> Result<(), String> {
    Ok(())
}

pub async fn my_unspendable_balance<T>(coin: T) -> Result<BigDecimal, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + MarketCoinOps,
{
    let mut attempts = 0i32;
    loop {
        let balance = try_s!(coin.my_balance().compat().await);
        let mature_unspents = try_s!(coin.ordered_mature_unspents(&coin.as_ref().my_address).compat().await);
        let spendable_balance = mature_unspents.iter().fold(BigDecimal::zero(), |acc, x| {
            acc + big_decimal_from_sat(x.value as i64, coin.as_ref().decimals)
        });
        if balance >= spendable_balance {
            return Ok(balance - spendable_balance);
        }

        if attempts == 2 {
            return ERR!(
                "spendable balance {} more than total balance {}",
                spendable_balance,
                balance
            );
        }

        // the balance could be changed by other instance between my_balance() and ordered_mature_unspents() calls
        // try again
        attempts += 1;
        Timer::sleep(0.3).await;
    }
}

/// Swap contract address is not used by standard UTXO coins.
pub fn swap_contract_address() -> Option<BytesJson> { None }

/// Convert satoshis to BigDecimal amount of coin units
pub fn big_decimal_from_sat(satoshis: i64, decimals: u8) -> BigDecimal {
    BigDecimal::from(satoshis) / BigDecimal::from(10u64.pow(decimals as u32))
}

pub fn address_from_raw_pubkey(
    pub_key: &[u8],
    prefix: u8,
    t_addr_prefix: u8,
    checksum_type: ChecksumType,
) -> Result<Address, String> {
    Ok(Address {
        t_addr_prefix,
        prefix,
        hash: try_s!(Public::from_slice(pub_key)).address_hash(),
        checksum_type,
    })
}

/// Try to parse address from either cashaddress or standard UTXO address format.
fn address_from_any_format(coin: &UtxoCoinFields, from: &str) -> Result<Address, String> {
    let standard_err = match Address::from_str(from) {
        Ok(a) => return Ok(a),
        Err(e) => e,
    };

    let cashaddress_err =
        match Address::from_cashaddress(from, coin.checksum_type, coin.pub_addr_prefix, coin.p2sh_addr_prefix) {
            Ok(a) => return Ok(a),
            Err(e) => e,
        };

    ERR!(
        "error on parse standard address: {:?}, error on parse cashaddress: {:?}",
        standard_err,
        cashaddress_err,
    )
}

fn validate_payment<T>(
    coin: T,
    payment_tx: &[u8],
    time_lock: u32,
    first_pub0: &Public,
    second_pub0: &Public,
    priv_bn_hash: &[u8],
    amount: BigDecimal,
) -> Box<dyn Future<Item = (), Error = String> + Send>
where
    T: AsRef<UtxoCoinFields> + Send + Sync + 'static,
{
    let tx: UtxoTx = try_fus!(deserialize(payment_tx).map_err(|e| ERRL!("{:?}", e)));
    let amount = try_fus!(sat_from_big_decimal(&amount, coin.as_ref().decimals));

    let expected_redeem = payment_script(
        time_lock,
        priv_bn_hash,
        &try_fus!(Public::from_slice(first_pub0)),
        &try_fus!(Public::from_slice(second_pub0)),
    );
    let fut = async move {
        let mut attempts = 0;
        loop {
            let tx_from_rpc = match coin
                .as_ref()
                .rpc_client
                .get_transaction_bytes(tx.hash().reversed().into())
                .compat()
                .await
            {
                Ok(t) => t,
                Err(e) => {
                    if attempts > 2 {
                        return ERR!(
                            "Got error {:?} after 3 attempts of getting tx {:?} from RPC",
                            e,
                            tx.tx_hash()
                        );
                    };
                    attempts += 1;
                    log!("Error " [e] " getting the tx " [tx.tx_hash()] " from rpc");
                    Timer::sleep(10.).await;
                    continue;
                },
            };
            if serialize(&tx).take() != tx_from_rpc.0 {
                return ERR!(
                    "Provided payment tx {:?} doesn't match tx data from rpc {:?}",
                    tx,
                    tx_from_rpc
                );
            }

            let expected_output = TransactionOutput {
                value: amount,
                script_pubkey: Builder::build_p2sh(&dhash160(&expected_redeem)).into(),
            };

            if tx.outputs[0] != expected_output {
                return ERR!(
                    "Provided payment tx output doesn't match expected {:?} {:?}",
                    tx.outputs[0],
                    expected_output
                );
            }
            return Ok(());
        }
    };
    Box::new(fut.boxed().compat())
}

async fn search_for_swap_tx_spend(
    coin: &UtxoCoinFields,
    time_lock: u32,
    first_pub: &Public,
    second_pub: &Public,
    secret_hash: &[u8],
    tx: &[u8],
    search_from_block: u64,
) -> Result<Option<FoundSwapTxSpend>, String> {
    let tx: UtxoTx = try_s!(deserialize(tx).map_err(|e| ERRL!("{:?}", e)));
    let script = payment_script(time_lock, secret_hash, first_pub, second_pub);
    let expected_script_pubkey = Builder::build_p2sh(&dhash160(&script)).to_bytes();
    if tx.outputs[0].script_pubkey != expected_script_pubkey {
        return ERR!(
            "Transaction {:?} output 0 script_pubkey doesn't match expected {:?}",
            tx,
            expected_script_pubkey
        );
    }

    let spend = try_s!(
        coin.rpc_client
            .find_output_spend(&tx, 0, search_from_block)
            .compat()
            .await
    );
    match spend {
        Some(tx) => {
            let script: Script = tx.inputs[0].script_sig.clone().into();
            if let Some(Ok(ref i)) = script.iter().nth(2) {
                if i.opcode == Opcode::OP_0 {
                    return Ok(Some(FoundSwapTxSpend::Spent(tx.into())));
                }
            }

            if let Some(Ok(ref i)) = script.iter().nth(1) {
                if i.opcode == Opcode::OP_1 {
                    return Ok(Some(FoundSwapTxSpend::Refunded(tx.into())));
                }
            }

            ERR!(
                "Couldn't find required instruction in script_sig of input 0 of tx {:?}",
                tx
            )
        },
        None => Ok(None),
    }
}

fn payment_script(time_lock: u32, secret_hash: &[u8], pub_0: &Public, pub_1: &Public) -> Script {
    let builder = Builder::default();
    builder
        .push_opcode(Opcode::OP_IF)
        .push_bytes(&time_lock.to_le_bytes())
        .push_opcode(Opcode::OP_CHECKLOCKTIMEVERIFY)
        .push_opcode(Opcode::OP_DROP)
        .push_bytes(pub_0)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ELSE)
        .push_opcode(Opcode::OP_SIZE)
        .push_bytes(&[32])
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_opcode(Opcode::OP_HASH160)
        .push_bytes(secret_hash)
        .push_opcode(Opcode::OP_EQUALVERIFY)
        .push_bytes(pub_1)
        .push_opcode(Opcode::OP_CHECKSIG)
        .push_opcode(Opcode::OP_ENDIF)
        .into_script()
}

/// Creates signed input spending hash time locked p2sh output
fn p2sh_spend(
    signer: &TransactionInputSigner,
    input_index: usize,
    key_pair: &KeyPair,
    script_data: Script,
    redeem_script: Script,
    signature_version: SignatureVersion,
    fork_id: u32,
) -> Result<TransactionInput, String> {
    let sighash = signer.signature_hash(
        input_index,
        signer.inputs[input_index].amount,
        &redeem_script,
        signature_version,
        1 | fork_id,
    );

    let sig = try_s!(script_sig(&sighash, &key_pair, fork_id));

    let mut resulting_script = Builder::default().push_data(&sig).into_bytes();
    if !script_data.is_empty() {
        resulting_script.extend_from_slice(&script_data);
    }

    let redeem_part = Builder::default().push_data(&redeem_script).into_bytes();
    resulting_script.extend_from_slice(&redeem_part);

    Ok(TransactionInput {
        script_sig: resulting_script,
        sequence: signer.inputs[input_index].sequence,
        script_witness: vec![],
        previous_output: signer.inputs[input_index].previous_output.clone(),
    })
}

#[allow(clippy::needless_lifetimes)]
pub async fn list_unspent_ordered<'a, T>(
    coin: &'a T,
    address: &Address,
) -> Result<(Vec<UnspentInfo>, AsyncMutexGuard<'a, RecentlySpentOutPoints>), String>
where
    T: AsRef<UtxoCoinFields>,
{
    let decimals = coin.as_ref().decimals;
    let mut unspents = try_s!(
        coin.as_ref()
            .rpc_client
            .list_unspent(address, decimals)
            .map_err(|e| ERRL!("{}", e))
            .compat()
            .await
    );
    let recently_spent = coin.as_ref().recently_spent_outpoints.lock().await;
    unspents = recently_spent
        .replace_spent_outputs_with_cache(unspents.into_iter().collect())
        .into_iter()
        .collect();
    unspents.sort_unstable_by(|a, b| {
        if a.value < b.value {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    });
    // dedup just in case we add duplicates of same unspent out
    // all duplicates will be removed because vector in sorted before dedup
    unspents.dedup_by(|one, another| one.outpoint == another.outpoint);
    Ok((unspents, recently_spent))
}

async fn merge_utxo_loop<T>(weak: UtxoWeak, merge_at: usize, check_every: f64, max_merge_at_once: usize)
where
    T: From<UtxoArc> + AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    loop {
        Timer::sleep(check_every).await;

        let coin = match weak.upgrade() {
            Some(arc) => T::from(arc),
            None => break,
        };

        let ticker = &coin.as_ref().ticker;
        let (unspents, recently_spent) = match coin.list_unspent_ordered(&coin.as_ref().my_address).await {
            Ok((unspents, recently_spent)) => (unspents, recently_spent),
            Err(e) => {
                error!("Error {} on list_unspent_ordered of coin {}", e, ticker);
                continue;
            },
        };
        if unspents.len() >= merge_at {
            let unspents: Vec<_> = unspents.into_iter().take(max_merge_at_once).collect();
            info!("Trying to merge {} UTXOs of coin {}", unspents.len(), ticker);
            let value = unspents.iter().fold(0, |sum, unspent| sum + unspent.value);
            let script_pubkey = Builder::build_p2pkh(&coin.as_ref().my_address.hash).to_bytes();
            let output = TransactionOutput { value, script_pubkey };
            let merge_tx_fut = generate_and_send_tx(
                &coin,
                unspents,
                vec![output],
                FeePolicy::DeductFromOutput(0),
                recently_spent,
            );
            match merge_tx_fut.await {
                Ok(tx) => info!(
                    "UTXO merge successful for coin {}, tx_hash {:?}",
                    ticker,
                    tx.hash().reversed()
                ),
                Err(e) => error!("Error {} on UTXO merge attempt for coin {}", e, ticker),
            }
        }
    }
}

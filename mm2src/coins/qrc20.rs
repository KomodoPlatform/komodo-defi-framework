use crate::eth::{self, u256_to_big_decimal, wei_from_big_decimal, TryToAddress};
use crate::qrc20::rpc_clients::{LogEntry, Qrc20ElectrumOps, Qrc20NativeOps, Qrc20RpcOps, TopicFilter, TxReceipt,
                                ViewContractCallType};
use crate::utxo::qtum::QtumBasedCoin;
use crate::utxo::rpc_clients::{ElectrumClient, NativeClient, UnspentInfo, UtxoRpcClientEnum, UtxoRpcClientOps};
use crate::utxo::utxo_common::{self, big_decimal_from_sat};
use crate::utxo::{coin_daemon_data_dir, qtum, sign_tx, ActualTxFee, AdditionalTxData, FeePolicy,
                  GenerateTransactionError, RecentlySpentOutPoints, UtxoAddressFormat, UtxoCoinBuilder,
                  UtxoCoinFields, UtxoCommonOps, UtxoTx, VerboseTransactionFrom, UTXO_LOCK};
use crate::{FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin, SwapOps, TradeFee, TransactionDetails,
            TransactionEnum, TransactionFut, ValidateAddressResult, WithdrawFee, WithdrawRequest};
use async_trait::async_trait;
use bigdecimal::BigDecimal;
use bitcrypto::{dhash160, sha256};
use chain::TransactionOutput;
use common::block_on;
use common::executor::Timer;
use common::jsonrpc_client::{JsonRpcClient, JsonRpcError, JsonRpcRequest, RpcRes};
use common::log::{error, warn};
use common::mm_ctx::MmArc;
use ethabi::{Function, Token};
use ethereum_types::{H160, U256};
use futures::compat::Future01CompatExt;
use futures::lock::MutexGuard as AsyncMutexGuard;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use gstuff::now_ms;
use keys::bytes::Bytes as ScriptBytes;
use keys::{Address as UtxoAddress, Address, Public};
#[cfg(test)] use mocktopus::macros::*;
use rpc::v1::types::{Bytes as BytesJson, Transaction as RpcTransaction, H160 as H160Json, H256 as H256Json};
use script::{Builder as ScriptBuilder, Opcode, Script, TransactionInputSigner};
use script_pubkey::generate_contract_call_script_pubkey;
use serde_json::{self as json, Value as Json};
use serialization::deserialize;
use serialization::serialize;
use std::ops::{Deref, Neg};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

mod history;
#[cfg(test)] mod qrc20_tests;
pub mod rpc_clients;
mod script_pubkey;
mod swap;

/// Qtum amount is always 0 for the QRC20 UTXO outputs,
/// because we should pay only a fee in Qtum to send the QRC20 transaction.
const OUTPUT_QTUM_AMOUNT: u64 = 0;
const QRC20_GAS_LIMIT_DEFAULT: u64 = 100_000;
const QRC20_GAS_PRICE_DEFAULT: u64 = 40;
const QRC20_SWAP_GAS_REQUIRED: u64 = QRC20_GAS_LIMIT_DEFAULT * 3;
const QRC20_DUST: u64 = 0;
// Keccak-256 hash of `Transfer` event
const QRC20_TRANSFER_TOPIC: &str = "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const QRC20_PAYMENT_SENT_TOPIC: &str = "ccc9c05183599bd3135da606eaaf535daffe256e9de33c048014cffcccd4ad57";
const QRC20_RECEIVER_SPENT_TOPIC: &str = "36c177bcb01c6d568244f05261e2946c8c977fa50822f3fa098c470770ee1f3e";
const QRC20_SENDER_REFUNDED_TOPIC: &str = "1797d500133f8e427eb9da9523aa4a25cb40f50ebc7dbda3c7c81778973f35ba";

struct Qrc20CoinBuilder<'a> {
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    req: &'a Json,
    priv_key: &'a [u8],
    platform: String,
    contract_address: H160,
}

impl<'a> Qrc20CoinBuilder<'a> {
    pub fn new(
        ctx: &'a MmArc,
        ticker: &'a str,
        conf: &'a Json,
        req: &'a Json,
        priv_key: &'a [u8],
        platform: String,
        contract_address: H160,
    ) -> Qrc20CoinBuilder<'a> {
        Qrc20CoinBuilder {
            ctx,
            ticker,
            conf,
            req,
            priv_key,
            platform,
            contract_address,
        }
    }
}

impl Qrc20CoinBuilder<'_> {
    fn swap_contract_address(&self) -> Result<H160, String> {
        match self.req()["swap_contract_address"].as_str() {
            Some(address) => qtum::contract_addr_from_str(address).map_err(|e| ERRL!("{}", e)),
            None => return ERR!("\"swap_contract_address\" field is expected"),
        }
    }
}

#[async_trait]
impl UtxoCoinBuilder for Qrc20CoinBuilder<'_> {
    type ResultCoin = Qrc20Coin;

    async fn build(self) -> Result<Self::ResultCoin, String> {
        let swap_contract_address = try_s!(self.swap_contract_address());
        let utxo = try_s!(self.build_utxo_fields().await);
        let inner = Qrc20CoinFields {
            utxo,
            platform: self.platform,
            contract_address: self.contract_address,
            swap_contract_address,
        };
        Ok(Qrc20Coin(Arc::new(inner)))
    }

    fn ctx(&self) -> &MmArc { self.ctx }

    fn conf(&self) -> &Json { self.conf }

    fn req(&self) -> &Json { self.req }

    fn ticker(&self) -> &str { self.ticker }

    fn priv_key(&self) -> &[u8] { self.priv_key }

    fn address_format(&self) -> Result<UtxoAddressFormat, String> { Ok(UtxoAddressFormat::Standard) }

    async fn decimals(&self, rpc_client: &UtxoRpcClientEnum) -> Result<u8, String> {
        if let Some(d) = self.conf()["decimals"].as_u64() {
            return Ok(d as u8);
        }

        rpc_client
            .token_decimals(&self.contract_address)
            .compat()
            .await
            .map_err(|e| ERRL!("{}", e))
    }

    fn dust_amount(&self) -> u64 { QRC20_DUST }

    #[cfg(feature = "native")]
    fn confpath(&self) -> Result<PathBuf, String> {
        // Documented at https://github.com/jl777/coins#bitcoin-protocol-specific-json
        // "USERHOME/" prefix should be replaced with the user's home folder.
        let declared_confpath = match self.conf()["confpath"].as_str() {
            Some(path) if !path.is_empty() => path.trim(),
            _ => {
                let is_asset_chain = false;
                let platform = self.platform.to_lowercase();
                let data_dir = coin_daemon_data_dir(&platform, is_asset_chain);

                let confname = format!("{}.conf", platform);
                return Ok(data_dir.join(&confname[..]));
            },
        };

        let (confpath, rel_to_home) = match declared_confpath.strip_prefix("~/") {
            Some(stripped) => (stripped, true),
            None => match declared_confpath.strip_prefix("USERHOME/") {
                Some(stripped) => (stripped, true),
                None => (declared_confpath, false),
            },
        };

        if rel_to_home {
            let home = try_s!(dirs::home_dir().ok_or("Can not detect the user home directory"));
            Ok(home.join(confpath))
        } else {
            Ok(confpath.into())
        }
    }
}

pub async fn qrc20_coin_from_conf_and_request(
    ctx: &MmArc,
    ticker: &str,
    platform: &str,
    conf: &Json,
    req: &Json,
    priv_key: &[u8],
    contract_address: H160,
) -> Result<Qrc20Coin, String> {
    let builder = Qrc20CoinBuilder::new(ctx, ticker, conf, req, priv_key, platform.to_owned(), contract_address);
    builder.build().await
}

#[derive(Debug)]
pub struct Qrc20CoinFields {
    pub utxo: UtxoCoinFields,
    pub platform: String,
    pub contract_address: H160,
    pub swap_contract_address: H160,
}

#[derive(Clone, Debug)]
pub struct Qrc20Coin(Arc<Qrc20CoinFields>);

impl Deref for Qrc20Coin {
    type Target = Qrc20CoinFields;
    fn deref(&self) -> &Qrc20CoinFields { &*self.0 }
}

impl AsRef<UtxoCoinFields> for Qrc20Coin {
    fn as_ref(&self) -> &UtxoCoinFields { &self.utxo }
}

impl qtum::QtumBasedCoin for Qrc20Coin {}

#[derive(Clone, Debug, PartialEq)]
pub struct ContractCallOutput {
    pub value: u64,
    pub script_pubkey: ScriptBytes,
    pub gas_limit: u64,
    pub gas_price: u64,
}

impl From<ContractCallOutput> for TransactionOutput {
    fn from(out: ContractCallOutput) -> Self {
        TransactionOutput {
            value: out.value,
            script_pubkey: out.script_pubkey,
        }
    }
}

/// Functions of ERC20/EtomicSwap smart contracts that may change the blockchain state.
#[derive(Debug, Eq, PartialEq)]
pub enum MutContractCallType {
    Transfer,
    Erc20Payment,
    ReceiverSpend,
    SenderRefund,
}

impl MutContractCallType {
    fn as_function_name(&self) -> &'static str {
        match self {
            MutContractCallType::Transfer => "transfer",
            MutContractCallType::Erc20Payment => "erc20Payment",
            MutContractCallType::ReceiverSpend => "receiverSpend",
            MutContractCallType::SenderRefund => "senderRefund",
        }
    }

    fn as_function(&self) -> &'static Function {
        match self {
            MutContractCallType::Transfer => unwrap!(eth::ERC20_CONTRACT.function(self.as_function_name())),
            MutContractCallType::Erc20Payment
            | MutContractCallType::ReceiverSpend
            | MutContractCallType::SenderRefund => unwrap!(eth::SWAP_CONTRACT.function(self.as_function_name())),
        }
    }

    pub fn from_script_pubkey(script: &[u8]) -> Result<Option<MutContractCallType>, String> {
        lazy_static! {
            static ref TRANSFER_SHORT_SIGN: [u8; 4] =
                eth::ERC20_CONTRACT.function("transfer").unwrap().short_signature();
            static ref ERC20_PAYMENT_SHORT_SIGN: [u8; 4] =
                eth::SWAP_CONTRACT.function("erc20Payment").unwrap().short_signature();
            static ref RECEIVER_SPEND_SHORT_SIGN: [u8; 4] =
                eth::SWAP_CONTRACT.function("receiverSpend").unwrap().short_signature();
            static ref SENDER_REFUND_SHORT_SIGN: [u8; 4] =
                eth::SWAP_CONTRACT.function("senderRefund").unwrap().short_signature();
        }

        if script.len() < 4 {
            return ERR!("Length of the script pubkey less than 4: {:?}", script);
        }

        if script.starts_with(TRANSFER_SHORT_SIGN.as_ref()) {
            return Ok(Some(MutContractCallType::Transfer));
        }
        if script.starts_with(ERC20_PAYMENT_SHORT_SIGN.as_ref()) {
            return Ok(Some(MutContractCallType::Erc20Payment));
        }
        if script.starts_with(RECEIVER_SPEND_SHORT_SIGN.as_ref()) {
            return Ok(Some(MutContractCallType::ReceiverSpend));
        }
        if script.starts_with(SENDER_REFUND_SHORT_SIGN.as_ref()) {
            return Ok(Some(MutContractCallType::SenderRefund));
        }
        Ok(None)
    }

    #[allow(dead_code)]
    fn short_signature(&self) -> [u8; 4] { self.as_function().short_signature() }
}

struct GenerateQrc20TxResult {
    signed: UtxoTx,
    miner_fee: u64,
    gas_fee: u64,
}

impl Qrc20Coin {
    /// `gas_fee` should be calculated by: gas_limit * gas_price * (count of contract calls),
    /// or should be sum of gas fee of all contract calls.
    pub async fn get_qrc20_tx_fee(&self, gas_fee: u64) -> Result<u64, String> {
        match try_s!(self.get_tx_fee().await) {
            ActualTxFee::Fixed(amount) | ActualTxFee::Dynamic(amount) => Ok(amount + gas_fee),
        }
    }

    /// Generate and send a transaction with the specified UTXO outputs.
    /// Note this function locks the `UTXO_LOCK`.
    pub async fn send_contract_calls(&self, outputs: Vec<ContractCallOutput>) -> Result<TransactionEnum, String> {
        // TODO: we need to somehow refactor it using RecentlySpentOutpoints cache
        // Move over all QRC20 tokens should share the same cache with each other and base QTUM coin
        let _utxo_lock = UTXO_LOCK.lock().await;

        let GenerateQrc20TxResult { signed, .. } = try_s!(self.generate_qrc20_transaction(outputs).await);
        let _tx = try_s!(self.utxo.rpc_client.send_transaction(&signed).compat().await);
        Ok(signed.into())
    }

    /// Generate Qtum UTXO transaction with contract calls.
    /// Note: lock the UTXO_LOCK mutex before this function will be called.
    async fn generate_qrc20_transaction(
        &self,
        outputs: Vec<ContractCallOutput>,
    ) -> Result<GenerateQrc20TxResult, String> {
        let unspents = try_s!(self
            .ordered_mature_unspents(&self.utxo.my_address)
            .compat()
            .await
            .map_err(|e| ERRL!("{}", e)));

        // None seems that the generate_transaction() should request estimated fee for Kbyte
        let actual_tx_fee = None;
        let gas_fee = outputs
            .iter()
            .fold(0, |gas_fee, output| gas_fee + output.gas_limit * output.gas_price);
        let fee_policy = FeePolicy::SendExact;

        let outputs = outputs.into_iter().map(|output| output.into()).collect();
        let (unsigned, data) = self
            .generate_transaction(unspents, outputs, fee_policy, actual_tx_fee, Some(gas_fee))
            .await
            .map_err(|e| match &e {
                GenerateTransactionError::EmptyUtxoSet => ERRL!("Not enough {} to Pay Fee: {}", self.platform, e),
                GenerateTransactionError::NotSufficientBalance { description } => {
                    ERRL!("Not enough {} to Pay Fee: {}", self.platform, description)
                },
                e => ERRL!("{}", e),
            })?;
        let prev_script = ScriptBuilder::build_p2pkh(&self.utxo.my_address.hash);
        let signed = try_s!(sign_tx(
            unsigned,
            &self.utxo.key_pair,
            prev_script,
            self.utxo.signature_version,
            self.utxo.fork_id
        ));
        Ok(GenerateQrc20TxResult {
            signed,
            miner_fee: data.fee_amount,
            gas_fee,
        })
    }

    fn transfer_output(
        &self,
        to_addr: H160,
        amount: U256,
        gas_limit: u64,
        gas_price: u64,
    ) -> Result<ContractCallOutput, String> {
        let function = try_s!(eth::ERC20_CONTRACT.function("transfer"));
        let params = try_s!(function.encode_input(&[Token::Address(to_addr), Token::Uint(amount)]));

        let script_pubkey = try_s!(generate_contract_call_script_pubkey(
            &params,
            gas_limit,
            gas_price,
            &self.contract_address,
        ))
        .to_bytes();

        Ok(ContractCallOutput {
            value: OUTPUT_QTUM_AMOUNT,
            script_pubkey,
            gas_limit,
            gas_price,
        })
    }
}

#[cfg_attr(test, mockable)]
#[async_trait]
impl UtxoCommonOps for Qrc20Coin {
    /// Get only QTUM transaction fee.
    async fn get_tx_fee(&self) -> Result<ActualTxFee, JsonRpcError> { utxo_common::get_tx_fee(&self.utxo).await }

    async fn get_htlc_spend_fee(&self) -> Result<u64, String> { utxo_common::get_htlc_spend_fee(self).await }

    fn addresses_from_script(&self, script: &Script) -> Result<Vec<UtxoAddress>, String> {
        utxo_common::addresses_from_script(&self.utxo, script)
    }

    fn denominate_satoshis(&self, satoshi: i64) -> f64 { utxo_common::denominate_satoshis(&self.utxo, satoshi) }

    fn my_public_key(&self) -> &Public { self.utxo.key_pair.public() }

    fn display_address(&self, address: &UtxoAddress) -> Result<String, String> {
        utxo_common::display_address(&self.utxo, address)
    }

    fn address_from_str(&self, address: &str) -> Result<UtxoAddress, String> {
        utxo_common::address_from_str(&self.utxo, address)
    }

    async fn get_current_mtp(&self) -> Result<u32, String> { utxo_common::get_current_mtp(&self.utxo).await }

    fn is_unspent_mature(&self, output: &RpcTransaction) -> bool { self.is_qtum_unspent_mature(output) }

    /// Generate UTXO transaction with specified unspent inputs and specified outputs.
    async fn generate_transaction(
        &self,
        utxos: Vec<UnspentInfo>,
        outputs: Vec<TransactionOutput>,
        fee_policy: FeePolicy,
        fee: Option<ActualTxFee>,
        gas_fee: Option<u64>,
    ) -> Result<(TransactionInputSigner, AdditionalTxData), GenerateTransactionError> {
        utxo_common::generate_transaction(self, utxos, outputs, fee_policy, fee, gas_fee).await
    }

    async fn calc_interest_if_required(
        &self,
        unsigned: TransactionInputSigner,
        data: AdditionalTxData,
        my_script_pub: ScriptBytes,
    ) -> Result<(TransactionInputSigner, AdditionalTxData), String> {
        utxo_common::calc_interest_if_required(self, unsigned, data, my_script_pub).await
    }

    fn p2sh_spending_tx(
        &self,
        prev_transaction: UtxoTx,
        redeem_script: ScriptBytes,
        outputs: Vec<TransactionOutput>,
        script_data: Script,
        sequence: u32,
    ) -> Result<UtxoTx, String> {
        utxo_common::p2sh_spending_tx(
            &self.utxo,
            prev_transaction,
            redeem_script,
            outputs,
            script_data,
            sequence,
        )
    }

    fn ordered_mature_unspents(
        &self,
        address: &UtxoAddress,
    ) -> Box<dyn Future<Item = Vec<UnspentInfo>, Error = String> + Send> {
        Box::new(
            utxo_common::ordered_mature_unspents(self.clone(), address.clone())
                .boxed()
                .compat(),
        )
    }

    fn get_verbose_transaction_from_cache_or_rpc(
        &self,
        txid: H256Json,
    ) -> Box<dyn Future<Item = VerboseTransactionFrom, Error = String> + Send> {
        let selfi = self.clone();
        let fut = async move { utxo_common::get_verbose_transaction_from_cache_or_rpc(&selfi.utxo, txid).await };
        Box::new(fut.boxed().compat())
    }

    async fn cache_transaction_if_possible(&self, tx: &RpcTransaction) -> Result<(), String> {
        utxo_common::cache_transaction_if_possible(&self.utxo, tx).await
    }

    async fn list_unspent_ordered<'a>(
        &'a self,
        address: &Address,
    ) -> Result<(Vec<UnspentInfo>, AsyncMutexGuard<'a, RecentlySpentOutPoints>), String> {
        utxo_common::list_unspent_ordered(self, address).await
    }
}

impl SwapOps for Qrc20Coin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal) -> TransactionFut {
        let to_address = try_fus!(self.contract_address_from_raw_pubkey(fee_addr));
        let amount = try_fus!(wei_from_big_decimal(&amount, self.utxo.decimals));
        let transfer_output =
            try_fus!(self.transfer_output(to_address, amount, QRC20_GAS_LIMIT_DEFAULT, QRC20_GAS_PRICE_DEFAULT));
        let outputs = vec![transfer_output];

        let selfi = self.clone();
        let fut = async move { selfi.send_contract_calls(outputs).await };

        Box::new(fut.boxed().compat())
    }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        let taker_addr = try_fus!(self.contract_address_from_raw_pubkey(taker_pub));
        let id = qrc20_swap_id(time_lock, secret_hash);
        let value = try_fus!(wei_from_big_decimal(&amount, self.utxo.decimals));
        let secret_hash = Vec::from(secret_hash);
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());

        let selfi = self.clone();
        let fut = async move {
            selfi
                .send_hash_time_locked_payment(id, value, time_lock, secret_hash, taker_addr, swap_contract_address)
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        let maker_addr = try_fus!(self.contract_address_from_raw_pubkey(maker_pub));
        let id = qrc20_swap_id(time_lock, secret_hash);
        let value = try_fus!(wei_from_big_decimal(&amount, self.utxo.decimals));
        let secret_hash = Vec::from(secret_hash);
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());

        let selfi = self.clone();
        let fut = async move {
            selfi
                .send_hash_time_locked_payment(id, value, time_lock, secret_hash, maker_addr, swap_contract_address)
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        let payment_tx: UtxoTx = try_fus!(deserialize(taker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());
        let secret = secret.to_vec();

        let selfi = self.clone();
        let fut = async move {
            selfi
                .spend_hash_time_locked_payment(payment_tx, swap_contract_address, secret)
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        let payment_tx: UtxoTx = try_fus!(deserialize(maker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let secret = secret.to_vec();
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());

        let selfi = self.clone();
        let fut = async move {
            selfi
                .spend_hash_time_locked_payment(payment_tx, swap_contract_address, secret)
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        let payment_tx: UtxoTx = try_fus!(deserialize(taker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());

        let selfi = self.clone();
        let fut = async move {
            selfi
                .refund_hash_time_locked_payment(swap_contract_address, payment_tx)
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        _secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        let payment_tx: UtxoTx = try_fus!(deserialize(maker_payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());

        let selfi = self.clone();
        let fut = async move {
            selfi
                .refund_hash_time_locked_payment(swap_contract_address, payment_tx)
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        fee_addr: &[u8],
        amount: &BigDecimal,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let fee_tx_hash: H256Json = match fee_tx {
            TransactionEnum::UtxoTx(tx) => tx.hash().reversed().into(),
            _ => panic!("Unexpected TransactionEnum"),
        };
        let fee_addr = try_fus!(self.contract_address_from_raw_pubkey(fee_addr));
        let expected_value = try_fus!(wei_from_big_decimal(amount, self.utxo.decimals));

        let selfi = self.clone();
        let fut = async move { selfi.validate_fee_impl(fee_tx_hash, fee_addr, expected_value).await };
        Box::new(fut.boxed().compat())
    }

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let payment_tx: UtxoTx = try_fus!(deserialize(payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let sender = try_fus!(self.contract_address_from_raw_pubkey(maker_pub));
        let secret_hash = secret_hash.to_vec();
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());

        let selfi = self.clone();
        let fut = async move {
            selfi
                .validate_payment(
                    payment_tx,
                    time_lock,
                    sender,
                    secret_hash,
                    amount,
                    swap_contract_address,
                )
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn validate_taker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());
        let payment_tx: UtxoTx = try_fus!(deserialize(payment_tx).map_err(|e| ERRL!("{:?}", e)));
        let sender = try_fus!(self.contract_address_from_raw_pubkey(taker_pub));
        let secret_hash = secret_hash.to_vec();

        let selfi = self.clone();
        let fut = async move {
            selfi
                .validate_payment(
                    payment_tx,
                    time_lock,
                    sender,
                    secret_hash,
                    amount,
                    swap_contract_address,
                )
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        _other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        let swap_id = qrc20_swap_id(time_lock, secret_hash);
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());

        let selfi = self.clone();
        let fut = async move {
            selfi
                .check_if_my_payment_sent_impl(swap_contract_address, swap_id, search_from_block)
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn search_for_swap_tx_spend_my(
        &self,
        time_lock: u32,
        _other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let tx: UtxoTx = try_s!(deserialize(tx).map_err(|e| ERRL!("{:?}", e)));

        let selfi = self.clone();
        let fut = selfi.search_for_swap_tx_spend(time_lock, secret_hash.to_vec(), tx, search_from_block);
        block_on(fut)
    }

    fn search_for_swap_tx_spend_other(
        &self,
        time_lock: u32,
        _other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let tx: UtxoTx = try_s!(deserialize(tx).map_err(|e| ERRL!("{:?}", e)));

        let selfi = self.clone();
        let fut = selfi.search_for_swap_tx_spend(time_lock, secret_hash.to_vec(), tx, search_from_block);
        block_on(fut)
    }

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        self.extract_secret_impl(secret_hash, spend_tx)
    }
}

impl MarketCoinOps for Qrc20Coin {
    fn ticker(&self) -> &str { &self.utxo.ticker }

    fn my_address(&self) -> Result<String, String> { utxo_common::my_address(self) }

    fn my_balance(&self) -> Box<dyn Future<Item = BigDecimal, Error = String> + Send> {
        let my_address = self.my_addr_as_contract_addr();
        let params = &[Token::Address(my_address)];
        let contract_address = self.contract_address;
        let decimals = self.utxo.decimals;

        let fut = self
            .utxo
            .rpc_client
            .rpc_contract_call(ViewContractCallType::BalanceOf, &contract_address, params)
            .map_err(|e| ERRL!("{}", e))
            .and_then(move |tokens| match tokens.first() {
                Some(Token::Uint(bal)) => u256_to_big_decimal(*bal, decimals),
                Some(_) => ERR!(r#"Expected Uint as "balanceOf" result but got {:?}"#, tokens),
                None => ERR!(r#"Expected Uint as "balanceOf" result but got nothing"#),
            });
        Box::new(fut)
    }

    fn base_coin_balance(&self) -> Box<dyn Future<Item = BigDecimal, Error = String> + Send> {
        // use standard UTXO my_balance implementation that returns Qtum balance instead of QRC20
        utxo_common::my_balance(&self.utxo)
    }

    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        utxo_common::send_raw_tx(&self.utxo, tx)
    }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let tx: UtxoTx = try_fus!(deserialize(tx).map_err(|e| ERRL!("{:?}", e)));
        let selfi = self.clone();
        let fut = async move {
            selfi
                .wait_for_confirmations_and_check_result(tx, confirmations, requires_nota, wait_until, check_every)
                .await
        };
        Box::new(fut.boxed().compat())
    }

    fn wait_for_tx_spend(
        &self,
        transaction: &[u8],
        wait_until: u64,
        from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        let tx: UtxoTx = try_fus!(deserialize(transaction).map_err(|e| ERRL!("{:?}", e)));

        let selfi = self.clone();
        let fut = async move { selfi.wait_for_tx_spend_impl(tx, wait_until, from_block).await };
        Box::new(fut.boxed().compat())
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> {
        utxo_common::tx_enum_from_bytes(bytes)
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        utxo_common::current_block(&self.utxo)
    }

    fn address_from_pubkey_str(&self, pubkey: &str) -> Result<String, String> {
        utxo_common::address_from_pubkey_str(self, pubkey)
    }

    fn display_priv_key(&self) -> String { utxo_common::display_priv_key(&self.utxo) }
}

impl MmCoin for Qrc20Coin {
    fn is_asset_chain(&self) -> bool { utxo_common::is_asset_chain(&self.utxo) }

    fn can_i_spend_other_payment(&self) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let selfi = self.clone();
        let fut = async move {
            let qtum_balance = try_s!(selfi.base_coin_balance().compat().await);
            let qtum_balance_sat = try_s!(wei_from_big_decimal(&qtum_balance, selfi.utxo.decimals));

            // other payment can be spend by `receiverSpend` that require only one output
            let gas_fee = QRC20_GAS_LIMIT_DEFAULT * QRC20_GAS_PRICE_DEFAULT;
            let min_amount: U256 = try_s!(selfi.get_qrc20_tx_fee(gas_fee).await).into();

            if qtum_balance_sat < min_amount {
                // u256_to_big_decimal() is expected to return no error
                let min_amount = try_s!(u256_to_big_decimal(min_amount, selfi.utxo.decimals));
                return ERR!(
                    "Base coin balance {} is too low to cover gas fee, required {}",
                    qtum_balance,
                    min_amount,
                );
            }
            Ok(())
        };
        Box::new(fut.boxed().compat())
    }

    fn wallet_only(&self) -> bool { false }

    fn withdraw(&self, req: WithdrawRequest) -> Box<dyn Future<Item = TransactionDetails, Error = String> + Send> {
        Box::new(qrc20_withdraw(self.clone(), req).boxed().compat())
    }

    fn decimals(&self) -> u8 { utxo_common::decimals(&self.utxo) }

    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> {
        qtum::QtumBasedCoin::convert_to_address(self, from, to_address_format)
    }

    fn validate_address(&self, address: &str) -> ValidateAddressResult { utxo_common::validate_address(self, address) }

    fn process_history_loop(&self, ctx: MmArc) { self.history_loop(ctx) }

    fn history_sync_status(&self) -> HistorySyncState { utxo_common::history_sync_status(&self.utxo) }

    /// This method is called to check our QTUM balance.
    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> {
        // `erc20Payment` may require two `approve` contract calls in worst case,
        // therefore use `QRC20_SWAP_GAS_REQUIRED` instead of `QRC20_GAS_LIMIT_DEFAULT`.
        let gas_fee = QRC20_SWAP_GAS_REQUIRED * QRC20_GAS_PRICE_DEFAULT;

        let selfi = self.clone();
        let fut = async move {
            let fee = try_s!(selfi.get_qrc20_tx_fee(gas_fee).await);
            Ok(TradeFee {
                coin: selfi.platform.clone(),
                amount: big_decimal_from_sat(fee as i64, selfi.utxo.decimals).into(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn required_confirmations(&self) -> u64 { utxo_common::required_confirmations(&self.utxo) }

    fn requires_notarization(&self) -> bool { utxo_common::requires_notarization(&self.utxo) }

    fn set_required_confirmations(&self, confirmations: u64) {
        utxo_common::set_required_confirmations(&self.utxo, confirmations)
    }

    fn set_requires_notarization(&self, requires_nota: bool) {
        utxo_common::set_requires_notarization(&self.utxo, requires_nota)
    }

    fn my_unspendable_balance(&self) -> Box<dyn Future<Item = BigDecimal, Error = String> + Send> {
        // QRC20 cannot have unspendable balance
        Box::new(futures01::future::ok(0.into()))
    }

    fn swap_contract_address(&self) -> Option<BytesJson> {
        Some(BytesJson::from(self.swap_contract_address.0.as_ref()))
    }
}

pub fn qrc20_swap_id(time_lock: u32, secret_hash: &[u8]) -> Vec<u8> {
    let mut input = vec![];
    input.extend_from_slice(&time_lock.to_le_bytes());
    input.extend_from_slice(secret_hash);
    sha256(&input).to_vec()
}

fn contract_addr_into_rpc_format(address: &H160) -> H160Json { H160Json::from(address.0) }

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Qrc20FeeDetails {
    /// Coin name
    coin: String,
    /// Standard UTXO miner fee based on transaction size
    miner_fee: BigDecimal,
    /// Gas limit in satoshi.
    gas_limit: u64,
    /// Gas price in satoshi.
    gas_price: u64,
    /// Total used gas.
    total_gas_fee: BigDecimal,
}

async fn qrc20_withdraw(coin: Qrc20Coin, req: WithdrawRequest) -> Result<TransactionDetails, String> {
    let to_addr = try_s!(UtxoAddress::from_str(&req.to));
    let is_p2pkh = to_addr.prefix == coin.utxo.pub_addr_prefix && to_addr.t_addr_prefix == coin.utxo.pub_t_addr_prefix;
    let is_p2sh = to_addr.prefix == coin.utxo.p2sh_addr_prefix
        && to_addr.t_addr_prefix == coin.utxo.p2sh_t_addr_prefix
        && coin.utxo.segwit;
    if !is_p2pkh && !is_p2sh {
        return ERR!("Address {} has invalid format", to_addr);
    }

    let _utxo_lock = UTXO_LOCK.lock().await;

    let qrc20_balance = try_s!(coin.my_balance().compat().await);

    // the qrc20_amount_sat is used only within smart contract calls
    let (qrc20_amount_sat, qrc20_amount) = if req.max {
        let amount = try_s!(wei_from_big_decimal(&qrc20_balance, coin.utxo.decimals));
        if amount.is_zero() {
            return ERR!("Balance is 0");
        }
        (amount, qrc20_balance.clone())
    } else {
        let amount_sat = try_s!(wei_from_big_decimal(&req.amount, coin.utxo.decimals));
        if amount_sat.is_zero() {
            return ERR!("The amount {} is too small", req.amount);
        }

        if req.amount > qrc20_balance {
            return ERR!(
                "The amount {} to withdraw is larger than balance {}",
                req.amount,
                qrc20_balance
            );
        }
        (amount_sat, req.amount)
    };

    let (gas_limit, gas_price) = match req.fee {
        Some(WithdrawFee::Qrc20Gas { gas_limit, gas_price }) => (gas_limit, gas_price),
        Some(_) => return ERR!("Unsupported input fee type"),
        None => (QRC20_GAS_LIMIT_DEFAULT, QRC20_GAS_PRICE_DEFAULT),
    };

    let transfer_output = try_s!(coin.transfer_output(
        qtum::contract_addr_from_utxo_addr(to_addr.clone()),
        qrc20_amount_sat,
        gas_limit,
        gas_price
    ));
    let outputs = vec![transfer_output];

    let GenerateQrc20TxResult {
        signed,
        miner_fee,
        gas_fee,
    } = try_s!(coin.generate_qrc20_transaction(outputs).await);

    let received_by_me = if to_addr == coin.utxo.my_address {
        qrc20_amount.clone()
    } else {
        0.into()
    };
    let my_balance_change = &received_by_me - &qrc20_amount;
    let my_address = try_s!(coin.my_address());
    let to_address = try_s!(coin.display_address(&to_addr));
    let fee_details = Qrc20FeeDetails {
        // QRC20 fees are paid in base platform currency (in particular Qtum)
        coin: coin.platform.clone(),
        miner_fee: utxo_common::big_decimal_from_sat(miner_fee as i64, coin.utxo.decimals),
        gas_limit,
        gas_price,
        total_gas_fee: utxo_common::big_decimal_from_sat(gas_fee as i64, coin.utxo.decimals),
    };
    Ok(TransactionDetails {
        from: vec![my_address],
        to: vec![to_address],
        total_amount: qrc20_amount.clone(),
        spent_by_me: qrc20_amount,
        received_by_me,
        my_balance_change,
        tx_hash: signed.hash().reversed().to_vec().into(),
        tx_hex: serialize(&signed).into(),
        fee_details: Some(fee_details.into()),
        block_height: 0,
        coin: coin.utxo.ticker.clone(),
        internal_id: vec![].into(),
        timestamp: now_ms() / 1000,
    })
}

/// Parse the given topic to `H160` address.
fn address_from_log_topic(topic: &str) -> Result<H160, String> {
    if topic.len() != 64 {
        return ERR!(
            "Topic {:?} is expected to be H256 encoded topic (with length of 64)",
            topic
        );
    }

    // skip the first 24 characters to parse the last 40 characters to H160.
    // https://github.com/qtumproject/qtum-electrum/blob/v4.0.2/electrum/wallet.py#L2112
    let hash = try_s!(H160Json::from_str(&topic[24..]));
    Ok(hash.0.into())
}

fn address_to_log_topic(address: &H160) -> String {
    let zeros = std::str::from_utf8(&[b'0'; 24]).expect("Expected a valid str from slice of '0' chars");
    let mut topic = format!("{:02x}", address);
    topic.insert_str(0, zeros);
    topic
}

pub struct TransferEventDetails {
    contract_address: H160,
    amount: U256,
    sender: H160,
    receiver: H160,
}

fn transfer_event_from_log(log: &LogEntry) -> Result<TransferEventDetails, String> {
    let contract_address = if log.address.starts_with("0x") {
        try_s!(qtum::contract_addr_from_str(&log.address))
    } else {
        let address = format!("0x{}", log.address);
        try_s!(qtum::contract_addr_from_str(&address))
    };

    if log.topics.len() != 3 {
        return ERR!("'Transfer' event must have 3 topics, found, {}", log.topics.len());
    }

    // https://github.com/qtumproject/qtum-electrum/blob/v4.0.2/electrum/wallet.py#L2111
    let amount = try_s!(U256::from_str(&log.data));

    // https://github.com/qtumproject/qtum-electrum/blob/v4.0.2/electrum/wallet.py#L2112
    let sender = try_s!(address_from_log_topic(&log.topics[1]));
    // https://github.com/qtumproject/qtum-electrum/blob/v4.0.2/electrum/wallet.py#L2113
    let receiver = try_s!(address_from_log_topic(&log.topics[2]));
    Ok(TransferEventDetails {
        contract_address,
        amount,
        sender,
        receiver,
    })
}

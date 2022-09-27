use super::htlc::{IrisHtlc, MsgCreateHtlc};
#[cfg(not(target_arch = "wasm32"))]
use super::tendermint_native_rpc::*;
#[cfg(target_arch = "wasm32")] use super::tendermint_wasm_rpc::*;
use crate::coin_errors::{MyAddressError, ValidatePaymentError};
use crate::tendermint::htlc::MsgClaimHtlc;
use crate::utxo::sat_from_big_decimal;
use crate::{big_decimal_from_sat_unsigned, BalanceError, BalanceFut, BigDecimal, CoinBalance, FeeApproxStage,
            FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin, NegotiateSwapContractAddrErr,
            RawTransactionFut, RawTransactionRequest, SearchForSwapTxSpendInput, SignatureResult, SwapOps, TradeFee,
            TradePreimageFut, TradePreimageResult, TradePreimageValue, TransactionDetails, TransactionEnum,
            TransactionFut, TransactionType, TxFeeDetails, TxMarshalingErr, UnexpectedDerivationMethod,
            ValidateAddressResult, ValidatePaymentFut, ValidatePaymentInput, VerificationResult,
            WatcherValidatePaymentInput, WithdrawError, WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use bitcrypto::sha256;
use common::{get_utc_timestamp, Future01CompatExt};
use cosmrs::bank::MsgSend;
use cosmrs::crypto::secp256k1::SigningKey;
use cosmrs::proto::cosmos::auth::v1beta1::{BaseAccount, QueryAccountRequest, QueryAccountResponse};
use cosmrs::proto::cosmos::bank::v1beta1::{QueryBalanceRequest, QueryBalanceResponse};
use cosmrs::tendermint::abci::Path as AbciPath;
use cosmrs::tendermint::chain::Id as ChainId;
use cosmrs::tx::{self, Fee, Msg, Raw, SignDoc, SignerInfo};
use cosmrs::{AccountId, Any, Coin, Denom};
use derive_more::Display;
use futures::lock::Mutex as AsyncMutex;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::MmNumber;
use prost::{DecodeError, Message};
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use std::convert::TryFrom;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;

const TIMEOUT_HEIGHT_DELTA: u64 = 100;
pub const GAS_LIMIT_DEFAULT: u64 = 100_000;
pub const TX_DEFAULT_MEMO: &str = "";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TendermintFeeDetails {
    coin: String,
    amount: BigDecimal,
    gas_limit: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TendermintProtocolInfo {
    decimals: u8,
    denom: String,
    account_prefix: String,
    chain_id: String,
}

#[derive(Clone, Deserialize)]
pub struct TendermintActivationParams {
    rpc_urls: Vec<String>,
}

pub struct TendermintCoinImpl {
    ticker: String,
    rpc_client: HttpClient,
    /// My address
    pub account_id: AccountId,
    account_prefix: String,
    priv_key: Vec<u8>,
    decimals: u8,
    denom: Denom,
    chain_id: ChainId,
    sequence_lock: AsyncMutex<()>,
}

#[derive(Clone)]
pub struct TendermintCoin(Arc<TendermintCoinImpl>);

impl Deref for TendermintCoin {
    type Target = TendermintCoinImpl;

    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug)]
pub struct TendermintInitError {
    pub ticker: String,
    pub kind: TendermintInitErrorKind,
}

#[derive(Display, Debug)]
pub enum TendermintInitErrorKind {
    InvalidPrivKey(String),
    CouldNotGenerateAccountId(String),
    EmptyRpcUrls,
    RpcClientInitError(String),
    InvalidChainId(String),
    InvalidDenom(String),
    RpcError(String),
}

#[derive(Display, Debug)]
enum TendermintCoinRpcError {
    Prost(prost::DecodeError),
    InvalidResponse(String),
    PerformError(String),
}

impl From<prost::DecodeError> for TendermintCoinRpcError {
    fn from(err: DecodeError) -> Self { TendermintCoinRpcError::Prost(err) }
}

impl From<TendermintCoinRpcError> for WithdrawError {
    fn from(err: TendermintCoinRpcError) -> Self { WithdrawError::Transport(err.to_string()) }
}

impl From<TendermintCoinRpcError> for BalanceError {
    fn from(err: TendermintCoinRpcError) -> Self {
        match err {
            TendermintCoinRpcError::InvalidResponse(e) => BalanceError::InvalidResponse(e),
            TendermintCoinRpcError::Prost(e) => BalanceError::InvalidResponse(e.to_string()),
            TendermintCoinRpcError::PerformError(e) => BalanceError::Transport(e),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<cosmrs::rpc::Error> for TendermintCoinRpcError {
    fn from(err: cosmrs::rpc::Error) -> Self { TendermintCoinRpcError::PerformError(err.to_string()) }
}

#[cfg(target_arch = "wasm32")]
impl From<PerformError> for TendermintCoinRpcError {
    fn from(err: PerformError) -> Self { TendermintCoinRpcError::PerformError(err.to_string()) }
}

fn account_id_from_privkey(priv_key: &[u8], prefix: &str) -> MmResult<AccountId, TendermintInitErrorKind> {
    let signing_key =
        SigningKey::from_bytes(priv_key).map_to_mm(|e| TendermintInitErrorKind::InvalidPrivKey(e.to_string()))?;

    signing_key
        .public_key()
        .account_id(prefix)
        .map_to_mm(|e| TendermintInitErrorKind::CouldNotGenerateAccountId(e.to_string()))
}

fn upper_hex(bytes: &[u8]) -> String {
    let mut str = hex::encode(bytes);
    str.make_ascii_uppercase();
    str
}

impl TendermintCoin {
    pub async fn init(
        ticker: String,
        protocol_info: TendermintProtocolInfo,
        activation_params: TendermintActivationParams,
        priv_key: &[u8],
    ) -> MmResult<Self, TendermintInitError> {
        if activation_params.rpc_urls.is_empty() {
            return MmError::err(TendermintInitError {
                ticker,
                kind: TendermintInitErrorKind::EmptyRpcUrls,
            });
        }

        let account_id =
            account_id_from_privkey(priv_key, &protocol_info.account_prefix).mm_err(|kind| TendermintInitError {
                ticker: ticker.clone(),
                kind,
            })?;

        // TODO multiple rpc_urls support will be added on the next iteration
        let rpc_client =
            HttpClient::new(activation_params.rpc_urls[0].as_str()).map_to_mm(|e| TendermintInitError {
                ticker: ticker.clone(),
                kind: TendermintInitErrorKind::RpcClientInitError(e.to_string()),
            })?;

        let chain_id = ChainId::try_from(protocol_info.chain_id).map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::InvalidChainId(e.to_string()),
        })?;

        let denom = Denom::from_str(&protocol_info.denom).map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::InvalidDenom(e.to_string()),
        })?;

        Ok(TendermintCoin(Arc::new(TendermintCoinImpl {
            ticker,
            rpc_client,
            account_id,
            account_prefix: protocol_info.account_prefix,
            priv_key: priv_key.to_vec(),
            decimals: protocol_info.decimals,
            denom,
            chain_id,
            sequence_lock: AsyncMutex::new(()),
        })))
    }

    async fn my_account_info(&self) -> MmResult<BaseAccount, TendermintCoinRpcError> {
        let path = AbciPath::from_str("/cosmos.auth.v1beta1.Query/Account").expect("valid path");
        let request = QueryAccountRequest {
            address: self.account_id.to_string(),
        };
        let request = AbciRequest::new(Some(path), request.encode_to_vec(), None, false);

        let response = self.rpc_client.perform(request).await?;
        let account_response = QueryAccountResponse::decode(response.response.value.as_slice())?;
        let account = account_response
            .account
            .or_mm_err(|| TendermintCoinRpcError::InvalidResponse("Account is None".into()))?;
        Ok(BaseAccount::decode(account.value.as_slice())?)
    }

    async fn my_balance_denom(&self) -> MmResult<u64, TendermintCoinRpcError> {
        let path = AbciPath::from_str("/cosmos.bank.v1beta1.Query/Balance").expect("valid path");
        let request = QueryBalanceRequest {
            address: self.account_id.to_string(),
            denom: self.denom.to_string(),
        };
        let request = AbciRequest::new(Some(path), request.encode_to_vec(), None, false);

        let response = self.rpc_client.perform(request).await?;
        let response = QueryBalanceResponse::decode(response.response.value.as_slice())?;
        response
            .balance
            .or_mm_err(|| TendermintCoinRpcError::InvalidResponse("balance is None".into()))?
            .amount
            .parse()
            .map_to_mm(|e| TendermintCoinRpcError::InvalidResponse(format!("balance is not u64, err {}", e)))
    }

    #[allow(dead_code)]
    fn gen_create_htlc_tx(
        &self,
        base_denom: Denom,
        to: &AccountId,
        amount: cosmrs::Decimal,
        secret_hash: &[u8],
        time_lock: u64,
    ) -> MmResult<IrisHtlc, TxMarshalingErr> {
        let timestamp = get_utc_timestamp() as u64;
        let mut hash_lock_hash = vec![];
        hash_lock_hash.extend_from_slice(secret_hash);
        hash_lock_hash.extend_from_slice(&timestamp.to_be_bytes());
        drop_mutability!(hash_lock_hash);

        let amount = vec![Coin {
            denom: self.denom.clone(),
            amount,
        }];

        // Needs to be sorted if cointains multiple coins
        // amount.sort();

        // << BEGIN HTLC id calculation
        // This is converted from irismod and cosmos-sdk source codes written in golang.
        // Refs:
        //  - Main algorithm: https://github.com/irisnet/irismod/blob/main/modules/htlc/types/htlc.go#L157
        //  - Coins string building https://github.com/cosmos/cosmos-sdk/blob/main/types/coin.go#L210-L225
        let coins_string = amount
            .iter()
            .map(|t| format!("{}{}", t.amount, t.denom))
            .collect::<Vec<String>>()
            .join(",");

        let mut htlc_id = vec![];
        htlc_id.extend_from_slice(sha256(&hash_lock_hash).as_slice());
        htlc_id.extend_from_slice(&self.account_id.to_bytes());
        htlc_id.extend_from_slice(&to.to_bytes());
        htlc_id.extend_from_slice(coins_string.as_bytes());
        let htlc_id = sha256(&htlc_id).to_string().to_uppercase();
        // >> END HTLC id calculation

        let msg_payload = MsgCreateHtlc {
            sender: self.account_id.clone(),
            to: to.clone(),
            receiver_on_other_chain: "".to_string(),
            sender_on_other_chain: "".to_string(),
            amount,
            hash_lock: sha256(&hash_lock_hash).to_string(),
            timestamp,
            time_lock,
            transfer: false,
        };

        let fee_amount = Coin {
            denom: base_denom,
            // TODO
            // Calculate current fee
            amount: 200_u64.into(),
        };

        let fee = Fee::from_amount_and_gas(fee_amount, GAS_LIMIT_DEFAULT);

        Ok(IrisHtlc {
            id: htlc_id,
            fee,
            msg_payload: msg_payload
                .to_any()
                .map_err(|e| MmError::new(TxMarshalingErr::InvalidInput(e.to_string())))?,
        })
    }

    #[allow(dead_code)]
    fn gen_claim_htlc_tx(
        &self,
        base_denom: Denom,
        htlc_id: String,
        secret_hash: &[u8],
    ) -> MmResult<IrisHtlc, TxMarshalingErr> {
        let msg_payload = MsgClaimHtlc {
            id: htlc_id.clone(),
            sender: self.account_id.clone(),
            secret: hex::encode(secret_hash),
        };

        let fee_amount = Coin {
            denom: base_denom,
            // TODO
            // Calculate current fee
            amount: 200_u64.into(),
        };

        let fee = Fee::from_amount_and_gas(fee_amount, GAS_LIMIT_DEFAULT);

        Ok(IrisHtlc {
            id: htlc_id,
            fee,
            msg_payload: msg_payload
                .to_any()
                .map_err(|e| MmError::new(TxMarshalingErr::InvalidInput(e.to_string())))?,
        })
    }

    fn any_to_signed_raw_tx(
        &self,
        account_info: BaseAccount,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
    ) -> cosmrs::Result<Raw> {
        let signkey = SigningKey::from_bytes(&self.priv_key)?;
        let tx_body = tx::Body::new(vec![tx_payload], TX_DEFAULT_MEMO, timeout_height as u32);
        let auth_info = SignerInfo::single_direct(Some(signkey.public_key()), account_info.sequence).auth_info(fee);
        let sign_doc = SignDoc::new(&tx_body, &auth_info, &self.chain_id, account_info.account_number)?;
        sign_doc.sign(&signkey)
    }
}

#[async_trait]
#[allow(unused_variables)]
impl MmCoin for TendermintCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        let coin = self.clone();
        let fut = async move {
            let to_address =
                AccountId::from_str(&req.to).map_to_mm(|e| WithdrawError::InvalidAddress(e.to_string()))?;
            if to_address.prefix() != coin.account_prefix {
                return MmError::err(WithdrawError::InvalidAddress(format!(
                    "expected {} address prefix",
                    coin.account_prefix
                )));
            }
            let balance_denom = coin.my_balance_denom().await?;
            let balance_dec = big_decimal_from_sat_unsigned(balance_denom, coin.decimals);

            // TODO calculate current fee instead of using hard-coded value
            let fee_denom = 1000;
            let fee_amount_dec = big_decimal_from_sat_unsigned(fee_denom, coin.decimals);

            let (amount_denom, amount_dec, total_amount) = if req.max {
                if balance_denom < fee_denom {
                    return MmError::err(WithdrawError::NotSufficientBalance {
                        coin: coin.ticker.clone(),
                        available: balance_dec,
                        required: fee_amount_dec,
                    });
                }
                let amount_denom = balance_denom - fee_denom;
                (
                    amount_denom,
                    big_decimal_from_sat_unsigned(amount_denom, coin.decimals),
                    balance_dec,
                )
            } else {
                let total = &req.amount + &fee_amount_dec;
                if balance_dec < total {
                    return MmError::err(WithdrawError::NotSufficientBalance {
                        coin: coin.ticker.clone(),
                        available: balance_dec,
                        required: total,
                    });
                }

                (sat_from_big_decimal(&req.amount, coin.decimals)?, req.amount, total)
            };
            let received_by_me = if to_address == coin.account_id {
                amount_dec
            } else {
                BigDecimal::default()
            };

            let msg_send = MsgSend {
                from_address: coin.account_id.clone(),
                to_address,
                amount: vec![Coin {
                    denom: coin.denom.clone(),
                    amount: amount_denom.into(),
                }],
            }
            .to_any()
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let current_block = coin
                .current_block()
                .compat()
                .await
                .map_to_mm(WithdrawError::Transport)?;

            let _sequence_lock = coin.sequence_lock.lock().await;
            let account_info = coin.my_account_info().await?;

            let fee_amount = Coin {
                denom: coin.denom.clone(),
                amount: fee_denom.into(),
            };
            let fee = Fee::from_amount_and_gas(fee_amount, GAS_LIMIT_DEFAULT);
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let tx_raw = coin
                .any_to_signed_raw_tx(account_info, msg_send, fee, timeout_height)
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let tx_bytes = tx_raw
                .to_bytes()
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let hash = sha256(&tx_bytes);
            Ok(TransactionDetails {
                tx_hash: upper_hex(hash.as_slice()),
                tx_hex: tx_bytes.into(),
                from: vec![coin.account_id.to_string()],
                to: vec![req.to],
                my_balance_change: &received_by_me - &total_amount,
                spent_by_me: total_amount.clone(),
                total_amount,
                received_by_me,
                block_height: 0,
                timestamp: 0,
                fee_details: Some(TxFeeDetails::Tendermint(TendermintFeeDetails {
                    coin: coin.ticker.clone(),
                    amount: fee_amount_dec,
                    gas_limit: GAS_LIMIT_DEFAULT,
                })),
                coin: coin.ticker.to_string(),
                internal_id: hash.to_vec().into(),
                kmd_rewards: None,
                transaction_type: TransactionType::default(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut { todo!() }

    fn decimals(&self) -> u8 { self.decimals }

    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> { todo!() }

    fn validate_address(&self, address: &str) -> ValidateAddressResult { todo!() }

    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { todo!() }

    fn history_sync_status(&self) -> HistorySyncState { todo!() }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { todo!() }

    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        todo!()
    }

    fn get_receiver_trade_fee(&self, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { todo!() }

    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: BigDecimal,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        todo!()
    }

    fn required_confirmations(&self) -> u64 { todo!() }

    fn requires_notarization(&self) -> bool { todo!() }

    fn set_required_confirmations(&self, confirmations: u64) { todo!() }

    fn set_requires_notarization(&self, requires_nota: bool) { todo!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { todo!() }

    fn mature_confirmations(&self) -> Option<u32> { None }

    fn coin_protocol_info(&self) -> Vec<u8> { Vec::new() }

    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool { true }
}

#[allow(unused_variables)]
impl MarketCoinOps for TendermintCoin {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { Ok(self.account_id.to_string()) }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> { todo!() }

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> { todo!() }

    fn sign_message(&self, _message: &str) -> SignatureResult<String> { todo!() }

    fn verify_message(&self, _signature: &str, _message: &str, _address: &str) -> VerificationResult<bool> { todo!() }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            let balance_denom = coin.my_balance_denom().await?;
            Ok(CoinBalance {
                spendable: big_decimal_from_sat_unsigned(balance_denom, coin.decimals),
                unspendable: BigDecimal::default(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> {
        Box::new(self.my_balance().map(|coin_balance| coin_balance.spendable))
    }

    fn platform_ticker(&self) -> &str { &self.ticker }

    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        let tx_bytes = try_fus!(hex::decode(tx));
        self.send_raw_tx_bytes(&tx_bytes)
    }

    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        // as sanity check
        try_fus!(Raw::from_bytes(tx));

        let coin = self.clone();
        let tx_bytes = tx.to_owned();
        let fut = async move {
            let broadcast_res = try_s!(coin.rpc_client.broadcast_tx_commit(tx_bytes.into()).await);
            if !broadcast_res.check_tx.code.is_ok() {
                return ERR!("Tx check failed {:?}", broadcast_res.check_tx);
            }

            if !broadcast_res.deliver_tx.code.is_ok() {
                return ERR!("Tx deliver failed {:?}", broadcast_res.deliver_tx);
            }
            Ok(broadcast_res.hash.to_string())
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
        todo!()
    }

    fn wait_for_tx_spend(
        &self,
        transaction: &[u8],
        wait_until: u64,
        from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        todo!()
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        MmError::err(TxMarshalingErr::NotSupported(
            "tx_enum_from_bytes is not supported for Tendermint yet.".to_string(),
        ))
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        let coin = self.clone();
        let fut = async move {
            let info = try_s!(coin.rpc_client.abci_info().await);
            Ok(info.last_block_height.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn display_priv_key(&self) -> Result<String, String> { Ok(hex::encode(&self.priv_key)) }

    fn min_tx_amount(&self) -> BigDecimal { todo!() }

    fn min_trading_vol(&self) -> MmNumber { todo!() }
}

#[async_trait]
#[allow(unused_variables)]
impl SwapOps for TendermintCoin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut { todo!() }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn create_taker_spends_maker_payment_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_taker_spends_maker_payment_preimage(&self, preimage: &[u8], secret: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        expected_sender: &[u8],
        fee_addr: &[u8],
        amount: &BigDecimal,
        min_block_number: u64,
        uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        todo!()
    }

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> { todo!() }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> { todo!() }

    fn watcher_validate_taker_payment(
        &self,
        _input: WatcherValidatePaymentInput,
    ) -> Box<dyn Future<Item = (), Error = MmError<ValidatePaymentError>> + Send> {
        unimplemented!();
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        todo!()
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        todo!()
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        todo!()
    }

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> { todo!() }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        todo!()
    }

    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair { todo!() }
}

#[cfg(test)]
mod tendermint_coin_tests {
    use super::*;

    const IRIS_TESTNET_HTLC_PAIR1_SEED: &str = "iris test seed";
    // const IRIS_TESTNET_HTLC_PAIR1_ADDRESS: &str = "iaa1e0rx87mdj79zejewuc4jg7ql9ud2286g2us8f2";

    // const IRIS_TESTNET_HTLC_PAIR2_SEED: &str = "iris test2 seed";
    const IRIS_TESTNET_HTLC_PAIR2_ADDRESS: &str = "iaa1erfnkjsmalkwtvj44qnfr2drfzdt4n9ldh0kjv";

    const IRIS_TESTNET_RPC_URL: &str = "http://34.80.202.172:26657";

    fn get_iris_usdc_ibc_protocol() -> TendermintProtocolInfo {
        TendermintProtocolInfo {
            decimals: 6,
            denom: String::from("ibc/5C465997B4F582F602CD64E12031C6A6E18CAF1E6EDC9B5D808822DC0B5F850C"),
            account_prefix: String::from("iaa"),
            chain_id: String::from("nyancat-9"),
        }
    }

    #[test]
    fn test_tx_hash_str_from_bytes() {
        let tx_hex = "0a97010a8f010a1c2f636f736d6f732e62616e6b2e763162657461312e4d736753656e64126f0a2d636f736d6f7331737661773061716334353834783832356a753775613033673578747877643061686c3836687a122d636f736d6f7331737661773061716334353834783832356a753775613033673578747877643061686c3836687a1a0f0a057561746f6d120631303030303018d998bf0512670a500a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a2102000eef4ab169e7b26a4a16c47420c4176ab702119ba57a8820fb3e53c8e7506212040a020801180312130a0d0a057561746f6d12043130303010a08d061a4093e5aec96f7d311d129f5ec8714b21ad06a75e483ba32afab86354400b2ac8350bfc98731bbb05934bf138282750d71aadbe08ceb6bb195f2b55e1bbfdddaaad";
        let expected_hash = "1C25ED7D17FCC5959409498D5423594666C4E84F15AF7B4AF17DF29B2AF9E7F5";

        let tx_bytes = hex::decode(tx_hex).unwrap();
        let hash = sha256(&tx_bytes);
        assert_eq!(upper_hex(hash.as_slice()), expected_hash);
    }

    #[test]
    fn test_htlc_create_and_claim() {
        let activation_request = TendermintActivationParams {
            rpc_urls: vec![IRIS_TESTNET_RPC_URL.to_string()],
        };

        let protocol_conf = get_iris_usdc_ibc_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default()
            .with_secp256k1_key_pair(crypto::privkey::key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap())
            .into_mm_arc();

        let priv_key = &*ctx.secp256k1_key_pair().private().secret;

        let coin = common::block_on(TendermintCoin::init(
            "USDC-IBC".to_string(),
            protocol_conf,
            activation_request,
            priv_key,
        ))
        .unwrap();

        // << BEGIN HTLC CREATION
        let base_denom: Denom = "unyan".parse().unwrap();
        let to: AccountId = IRIS_TESTNET_HTLC_PAIR2_ADDRESS.parse().unwrap();
        let amount: cosmrs::Decimal = 1_u64.into();
        let sec = &[1; 32];
        let time_lock = 1000;

        let create_htlc_tx = coin
            .gen_create_htlc_tx(base_denom.clone(), &to, amount, sec, time_lock)
            .unwrap();

        let current_block_fut = coin.current_block().compat();
        let current_block = common::block_on(async { current_block_fut.await.unwrap() });
        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let account_info_fut = coin.my_account_info();
        let account_info = common::block_on(async { account_info_fut.await.unwrap() });

        let raw_tx = common::block_on(async {
            coin.any_to_signed_raw_tx(
                account_info.clone(),
                create_htlc_tx.msg_payload.clone(),
                create_htlc_tx.fee.clone(),
                timeout_height,
            )
            .unwrap()
        });
        let send_tx_fut = coin.send_raw_tx_bytes(&raw_tx.to_bytes().unwrap()).compat();
        common::block_on(async {
            send_tx_fut.await.unwrap();
        });
        // >> END HTLC CREATION

        // << BEGIN HTLC CLAIMING
        let claim_htlc_tx = coin
            .gen_claim_htlc_tx(base_denom.clone(), create_htlc_tx.id, sec)
            .unwrap();

        let current_block_fut = coin.current_block().compat();
        let current_block = common::block_on(async { current_block_fut.await.unwrap() });
        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let account_info_fut = coin.my_account_info();
        let account_info = common::block_on(async { account_info_fut.await.unwrap() });

        let raw_tx = common::block_on(async {
            coin.any_to_signed_raw_tx(
                account_info,
                claim_htlc_tx.msg_payload,
                claim_htlc_tx.fee,
                timeout_height,
            )
            .unwrap()
        });

        let send_tx_fut = coin.send_raw_tx_bytes(&raw_tx.to_bytes().unwrap()).compat();
        common::block_on(async {
            send_tx_fut.await.unwrap();
        });
        // >> END HTLC CLAIMING
    }
}

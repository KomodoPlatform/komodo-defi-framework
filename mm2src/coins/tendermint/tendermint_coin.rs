#[cfg(not(target_arch = "wasm32"))]
use super::tendermint_native_rpc::*;
#[cfg(target_arch = "wasm32")] use super::tendermint_wasm_rpc::*;
use crate::utxo::sat_from_big_decimal;
use crate::{big_decimal_from_sat_unsigned, BalanceError, BalanceFut, BigDecimal, CoinBalance, FeeApproxStage,
            FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin, NegotiateSwapContractAddrErr,
            RawTransactionFut, RawTransactionRequest, SearchForSwapTxSpendInput, SignatureResult, SwapOps, TradeFee,
            TradePreimageFut, TradePreimageResult, TradePreimageValue, TransactionDetails, TransactionEnum,
            TransactionFut, TransactionType, UnexpectedDerivationMethod, ValidateAddressResult, ValidatePaymentInput,
            VerificationResult, WithdrawError, WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use common::Future01CompatExt;
use cosmrs::bank::MsgSend;
use cosmrs::crypto::secp256k1::SigningKey;
use cosmrs::proto::cosmos::auth::v1beta1::{BaseAccount, QueryAccountRequest, QueryAccountResponse};
use cosmrs::proto::cosmos::bank::v1beta1::{QueryBalanceRequest, QueryBalanceResponse};
use cosmrs::tendermint::abci::Path as AbciPath;
use cosmrs::tendermint::chain::Id as ChainId;
use cosmrs::tx::{self, AccountNumber, Fee, Msg, Raw, SignDoc, SignerInfo};
use cosmrs::{AccountId, Coin, Denom};
use derive_more::Display;
use futures::lock::Mutex as AsyncMutex;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::MmNumber;
use prost::Message;
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;

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
    account_number: AccountNumber,
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

pub struct TendermintInitError {
    pub ticker: String,
    pub kind: TendermintInitErrorKind,
}

#[derive(Display)]
pub enum TendermintInitErrorKind {
    InvalidPrivKey(String),
    CouldNotGenerateAccountId(String),
    EmptyRpcUrls,
    RpcClientInitError(String),
    InvalidChainId(String),
    InvalidDenom(String),
    RpcError(String),
}

fn account_id_from_privkey(priv_key: &[u8], prefix: &str) -> MmResult<AccountId, TendermintInitErrorKind> {
    let signing_key =
        SigningKey::from_bytes(priv_key).map_to_mm(|e| TendermintInitErrorKind::InvalidPrivKey(e.to_string()))?;

    signing_key
        .public_key()
        .account_id(prefix)
        .map_to_mm(|e| TendermintInitErrorKind::CouldNotGenerateAccountId(e.to_string()))
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
                ticker: ticker.clone(),
                kind: TendermintInitErrorKind::EmptyRpcUrls,
            });
        }

        let account_id =
            account_id_from_privkey(priv_key, &protocol_info.account_prefix).mm_err(|kind| TendermintInitError {
                ticker: ticker.clone(),
                kind,
            })?;

        let rpc_client =
            HttpClient::new(activation_params.rpc_urls[0].as_str()).map_to_mm(|e| TendermintInitError {
                ticker: ticker.clone(),
                kind: TendermintInitErrorKind::RpcClientInitError(e.to_string()),
            })?;

        let chain_id = ChainId::from_str(&protocol_info.chain_id).map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::InvalidChainId(e.to_string()),
        })?;

        let denom = Denom::from_str(&protocol_info.denom).map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::InvalidDenom(e.to_string()),
        })?;

        let path = AbciPath::from_str("/cosmos.auth.v1beta1.Query/Account").expect("valid path");
        let request = QueryAccountRequest {
            address: account_id.to_string(),
        };
        let request = AbciRequest::new(Some(path), request.encode_to_vec(), None, false);

        let response = rpc_client.perform(request).await.map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::RpcError(e.to_string()),
        })?;
        let account_response =
            QueryAccountResponse::decode(response.response.value.as_slice()).map_to_mm(|e| TendermintInitError {
                ticker: ticker.clone(),
                kind: TendermintInitErrorKind::RpcError(e.to_string()),
            })?;
        let account = account_response.account.or_mm_err(|| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::RpcError("No account field in account response".into()),
        })?;
        let account_info = BaseAccount::decode(account.value.as_slice()).map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::RpcError(e.to_string()),
        })?;

        Ok(TendermintCoin(Arc::new(TendermintCoinImpl {
            ticker,
            rpc_client,
            account_id,
            account_number: account_info.account_number,
            account_prefix: protocol_info.account_prefix,
            priv_key: priv_key.to_vec(),
            decimals: protocol_info.decimals,
            denom,
            chain_id,
            sequence_lock: AsyncMutex::new(()),
        })))
    }
}

#[async_trait]
#[allow(unused_variables)]
impl MmCoin for TendermintCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        let coin = self.clone();
        let fut = async move {
            let current_block = coin
                .current_block()
                .compat()
                .await
                .map_to_mm(WithdrawError::Transport)?;
            let to_address =
                AccountId::from_str(&req.to).map_to_mm(|e| WithdrawError::InvalidAddress(e.to_string()))?;
            if to_address.prefix() != coin.account_prefix {
                return MmError::err(WithdrawError::InvalidAddress(format!(
                    "expected {} address prefix",
                    coin.account_prefix
                )));
            }

            let amount = sat_from_big_decimal(&req.amount, coin.decimals)?;
            let msg_send = MsgSend {
                from_address: coin.account_id.clone(),
                to_address,
                amount: vec![Coin {
                    denom: coin.denom.clone(),
                    amount: amount.into(),
                }],
            }
            .to_any()
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let _sequence_lock = coin.sequence_lock.lock().await;
            let path = AbciPath::from_str("/cosmos.auth.v1beta1.Query/Account").expect("valid path");
            let request = QueryAccountRequest {
                address: coin.account_id.to_string(),
            };
            let request = AbciRequest::new(Some(path), request.encode_to_vec(), None, false);

            let response = coin
                .rpc_client
                .perform(request)
                .await
                .map_to_mm(|e| WithdrawError::Transport(e.to_string()))?;
            let account_response = QueryAccountResponse::decode(response.response.value.as_slice())
                .map_to_mm(|e| WithdrawError::Transport(e.to_string()))?;
            let account = account_response
                .account
                .or_mm_err(|| WithdrawError::Transport("No account field in account_response".into()))?;

            let account_info =
                BaseAccount::decode(account.value.as_slice()).map_to_mm(|e| WithdrawError::Transport(e.to_string()))?;

            let sequence_number = account_info.sequence;

            let gas = 100_000;
            let fee_amount = Coin {
                denom: coin.denom.clone(),
                amount: 1000u16.into(),
            };
            let fee = Fee::from_amount_and_gas(fee_amount, gas);
            let timeout_height = current_block + 100;

            let privkey =
                SigningKey::from_bytes(&coin.priv_key).map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let tx_body = tx::Body::new(vec![msg_send], "", timeout_height as u32);
            let auth_info = SignerInfo::single_direct(Some(privkey.public_key()), sequence_number).auth_info(fee);
            let sign_doc = SignDoc::new(&tx_body, &auth_info, &coin.chain_id, coin.account_number)
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;
            let tx_raw = sign_doc
                .sign(&privkey)
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            Ok(TransactionDetails {
                tx_hex: tx_raw
                    .to_bytes()
                    .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?
                    .into(),
                tx_hash: "".to_string(),
                from: vec![coin.account_id.to_string()],
                to: vec![req.to],
                total_amount: Default::default(),
                spent_by_me: Default::default(),
                received_by_me: Default::default(),
                my_balance_change: Default::default(),
                block_height: 0,
                timestamp: 0,
                fee_details: None,
                coin: coin.ticker.to_string(),
                internal_id: Default::default(),
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

    fn mature_confirmations(&self) -> Option<u32> { todo!() }

    fn coin_protocol_info(&self) -> Vec<u8> { todo!() }

    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool { todo!() }
}

#[allow(unused_variables)]
impl MarketCoinOps for TendermintCoin {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> Result<String, String> { Ok(self.account_id.to_string()) }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> { todo!() }

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> { todo!() }

    fn sign_message(&self, _message: &str) -> SignatureResult<String> { todo!() }

    fn verify_message(&self, _signature: &str, _message: &str, _address: &str) -> VerificationResult<bool> { todo!() }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            let path = AbciPath::from_str("/cosmos.bank.v1beta1.Query/Balance").expect("valid path");
            let request = QueryBalanceRequest {
                address: coin.account_id.to_string(),
                denom: coin.denom.to_string(),
            };
            let request = AbciRequest::new(Some(path), request.encode_to_vec(), None, false);

            let response = coin
                .rpc_client
                .perform(request)
                .await
                .map_to_mm(|e| BalanceError::Transport(e.to_string()))?;
            let response = QueryBalanceResponse::decode(response.response.value.as_slice())
                .map_to_mm(|e| BalanceError::InvalidResponse(format!("{:?}", response)))?;
            let balance_uatom: u64 = response
                .balance
                .or_mm_err(|| BalanceError::InvalidResponse("balance is None".into()))?
                .amount
                .parse()
                .map_to_mm(|e| BalanceError::InvalidResponse(format!("balance is not u64, err {}", e)))?;
            Ok(CoinBalance {
                spendable: big_decimal_from_sat_unsigned(balance_uatom, coin.decimals),
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
        let tx = try_fus!(Raw::from_bytes(&tx_bytes));
        let coin = self.clone();
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

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> { todo!() }

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

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        todo!()
    }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        todo!()
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

use super::{check_decoded_length, validate_from_to_and_status, validate_payment_args, validate_payment_state,
            EthPaymentType, PaymentMethod, PrepareTxDataError, ZERO_VALUE};
use crate::eth::{decode_contract_call, get_function_input_data, wei_from_big_decimal, EthCoin, EthCoinType,
                 ParseCoinAssocTypes, RefundFundingSecretArgs, RefundTakerPaymentArgs, SendTakerFundingArgs,
                 SignedEthTx, SwapTxTypeWithSecretHash, TakerPaymentStateV2, TransactionErr, ValidateSwapV2TxError,
                 ValidateSwapV2TxResult, ValidateTakerFundingArgs, TAKER_SWAP_V2};
use crate::{FundingTxSpend, GenTakerFundingSpendArgs, GenTakerPaymentSpendArgs, SearchForFundingSpendErr,
            WaitForPaymentSpendError};
use common::executor::Timer;
use common::now_sec;
use ethabi::{Function, Token};
use ethcore_transaction::Action;
use ethereum_types::{Address, Public, U256};
use ethkey::public_to_address;
use futures::compat::Future01CompatExt;
use mm2_err_handle::prelude::{MapToMmResult, MmError, MmResult};
use std::convert::TryInto;
use web3::types::TransactionId;

const ETH_TAKER_PAYMENT: &str = "ethTakerPayment";
const ERC20_TAKER_PAYMENT: &str = "erc20TakerPayment";
const TAKER_PAYMENT_APPROVE: &str = "takerPaymentApprove";

/// state index for `TakerPayment` structure from `EtomicSwapTakerV2.sol`
///
///     struct TakerPayment {
///         bytes20 paymentHash;
///         uint32 preApproveLockTime;
///         uint32 paymentLockTime;
///         TakerPaymentState state;
///     }
const TAKER_PAYMENT_STATE_INDX: usize = 3;

struct TakerFundingArgs {
    dex_fee: U256,
    payment_amount: U256,
    maker_address: Address,
    taker_secret_hash: [u8; 32],
    maker_secret_hash: [u8; 32],
    funding_time_lock: u64,
    payment_time_lock: u64,
}

struct TakerRefundTimelockArgs {
    dex_fee: U256,
    payment_amount: U256,
    maker_address: Address,
    taker_secret_hash: [u8; 32],
    maker_secret_hash: [u8; 32],
    payment_time_lock: u64,
    token_address: Address,
}

struct TakerRefundSecretArgs {
    dex_fee: U256,
    payment_amount: U256,
    maker_address: Address,
    taker_secret: [u8; 32],
    maker_secret_hash: [u8; 32],
    payment_time_lock: u64,
    token_address: Address,
}

struct TakerValidationArgs<'a> {
    swap_id: Vec<u8>,
    amount: U256,
    dex_fee: U256,
    receiver: Address,
    taker_secret_hash: &'a [u8],
    maker_secret_hash: &'a [u8],
    funding_time_lock: u64,
    payment_time_lock: u64,
}

impl EthCoin {
    /// Calls `"ethTakerPayment"` or `"erc20TakerPayment"` swap contract methods.
    /// Returns taker sent payment transaction.
    pub(crate) async fn send_taker_funding_impl(
        &self,
        args: SendTakerFundingArgs<'_>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let taker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.taker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;
        // TODO add burnFee support
        let dex_fee = try_tx_s!(wei_from_big_decimal(&args.dex_fee.fee_amount().into(), self.decimals));

        let payment_amount = try_tx_s!(wei_from_big_decimal(
            &(args.trading_amount.clone() + args.premium_amount.clone()),
            self.decimals
        ));
        let funding_args = {
            let maker_address = public_to_address(&Public::from_slice(args.maker_pub));
            TakerFundingArgs {
                dex_fee,
                payment_amount,
                maker_address,
                taker_secret_hash: try_tx_s!(args.taker_secret_hash.try_into()),
                maker_secret_hash: try_tx_s!(args.maker_secret_hash.try_into()),
                funding_time_lock: args.funding_time_lock,
                payment_time_lock: args.payment_time_lock,
            }
        };
        match &self.coin_type {
            EthCoinType::Eth => {
                let data = try_tx_s!(self.prepare_taker_eth_funding_data(&funding_args).await);
                let eth_total_payment = payment_amount.checked_add(dex_fee).ok_or_else(|| {
                    TransactionErr::Plain(ERRL!("Overflow occurred while calculating eth_total_payment"))
                })?;
                self.sign_and_send_transaction(
                    eth_total_payment,
                    Action::Call(taker_swap_v2_contract),
                    data,
                    U256::from(self.gas_limit_v2.taker.eth_payment),
                )
                .compat()
                .await
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let data = try_tx_s!(self.prepare_taker_erc20_funding_data(&funding_args, *token_addr).await);
                self.handle_allowance(taker_swap_v2_contract, payment_amount, args.funding_time_lock)
                    .await?;
                self.sign_and_send_transaction(
                    U256::from(ZERO_VALUE),
                    Action::Call(taker_swap_v2_contract),
                    data,
                    U256::from(self.gas_limit_v2.taker.erc20_payment),
                )
                .compat()
                .await
            },
            EthCoinType::Nft { .. } => Err(TransactionErr::ProtocolNotSupported(ERRL!(
                "NFT protocol is not supported for ETH and ERC20 Swaps"
            ))),
        }
    }

    pub(crate) async fn validate_taker_funding_impl(
        &self,
        args: ValidateTakerFundingArgs<'_, Self>,
    ) -> ValidateSwapV2TxResult {
        if let EthCoinType::Nft { .. } = self.coin_type {
            return MmError::err(ValidateSwapV2TxError::ProtocolNotSupported(
                "NFT protocol is not supported for ETH and ERC20 Swaps".to_string(),
            ));
        }
        let taker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.taker_swap_v2_contract)
            .ok_or_else(|| {
                ValidateSwapV2TxError::Internal("Expected swap_v2_contracts to be Some, but found None".to_string())
            })?;
        validate_payment_args(args.taker_secret_hash, args.maker_secret_hash, &args.trading_amount)
            .map_err(ValidateSwapV2TxError::Internal)?;
        let taker_address = public_to_address(args.taker_pub);
        let swap_id = self.etomic_swap_id_v2(args.payment_time_lock, args.maker_secret_hash);
        let taker_status = self
            .payment_status_v2(
                taker_swap_v2_contract,
                Token::FixedBytes(swap_id.clone()),
                &TAKER_SWAP_V2,
                EthPaymentType::TakerPayments,
                TAKER_PAYMENT_STATE_INDX,
            )
            .await?;

        let tx_from_rpc = self.transaction(TransactionId::Hash(args.funding_tx.tx_hash())).await?;
        let tx_from_rpc = tx_from_rpc.as_ref().ok_or_else(|| {
            ValidateSwapV2TxError::TxDoesNotExist(format!(
                "Didn't find provided tx {:?} on ETH node",
                args.funding_tx.tx_hash()
            ))
        })?;
        validate_from_to_and_status(
            tx_from_rpc,
            taker_address,
            taker_swap_v2_contract,
            taker_status,
            TakerPaymentStateV2::PaymentSent as u8,
        )?;

        let validation_args = {
            let dex_fee = wei_from_big_decimal(&args.dex_fee.fee_amount().into(), self.decimals)?;
            let payment_amount = wei_from_big_decimal(&(args.trading_amount + args.premium_amount), self.decimals)?;
            TakerValidationArgs {
                swap_id,
                amount: payment_amount,
                dex_fee,
                receiver: self.my_addr().await,
                taker_secret_hash: args.taker_secret_hash,
                maker_secret_hash: args.maker_secret_hash,
                funding_time_lock: args.funding_time_lock,
                payment_time_lock: args.payment_time_lock,
            }
        };
        match self.coin_type {
            EthCoinType::Eth => {
                let function = TAKER_SWAP_V2.function(ETH_TAKER_PAYMENT)?;
                let decoded = decode_contract_call(function, &tx_from_rpc.input.0)?;
                validate_eth_taker_payment_data(&decoded, &validation_args, function, tx_from_rpc.value)?;
            },
            EthCoinType::Erc20 { token_addr, .. } => {
                let function = TAKER_SWAP_V2.function(ERC20_TAKER_PAYMENT)?;
                let decoded = decode_contract_call(function, &tx_from_rpc.input.0)?;
                validate_erc20_taker_payment_data(&decoded, &validation_args, function, token_addr)?;
            },
            EthCoinType::Nft { .. } => unreachable!(),
        }
        Ok(())
    }

    /// Taker approves payment calling `takerPaymentApprove` for EVM based chains.
    /// Function accepts taker payment transaction, returns taker approve payment transaction.
    pub(crate) async fn taker_payment_approve(
        &self,
        args: &GenTakerFundingSpendArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let gas_limit = match self.coin_type {
            EthCoinType::Eth | EthCoinType::Erc20 { .. } => U256::from(self.gas_limit_v2.taker.approve_payment),
            EthCoinType::Nft { .. } => {
                return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                    "NFT protocol is not supported for ETH and ERC20 Swaps"
                )))
            },
        };
        let (taker_swap_v2_contract, send_func, token_address) = self
            .taker_swap_v2_details(ETH_TAKER_PAYMENT, ERC20_TAKER_PAYMENT)
            .await?;
        let decoded = try_tx_s!(decode_contract_call(send_func, args.funding_tx.unsigned().data()));
        let taker_status = try_tx_s!(
            self.payment_status_v2(
                taker_swap_v2_contract,
                decoded[0].clone(),
                &TAKER_SWAP_V2,
                EthPaymentType::TakerPayments,
                TAKER_PAYMENT_STATE_INDX,
            )
            .await
        );
        validate_payment_state(args.funding_tx, taker_status, TakerPaymentStateV2::PaymentSent as u8)
            .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;
        let data = try_tx_s!(
            self.prepare_taker_payment_approve_data(args, decoded, token_address)
                .await
        );
        let approve_tx = self
            .sign_and_send_transaction(
                U256::from(ZERO_VALUE),
                Action::Call(taker_swap_v2_contract),
                data,
                gas_limit,
            )
            .compat()
            .await?;
        Ok(approve_tx)
    }

    pub(crate) async fn refund_taker_payment_with_timelock_impl(
        &self,
        args: RefundTakerPaymentArgs<'_>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let (token_address, gas_limit) = self
            .gas_limit_v2
            .gas_limit(
                &self.coin_type,
                EthPaymentType::TakerPayments,
                PaymentMethod::RefundTimelock,
            )
            .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;

        let taker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.taker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;
        let dex_fee = try_tx_s!(wei_from_big_decimal(
            &args.dex_fee.fee_amount().to_decimal(),
            self.decimals
        ));
        let payment_amount = try_tx_s!(wei_from_big_decimal(
            &(args.trading_amount + args.premium_amount),
            self.decimals
        ));
        let maker_address = public_to_address(&Public::from_slice(args.maker_pub));
        let (maker_secret_hash, taker_secret_hash) = match args.tx_type_with_secret_hash {
            SwapTxTypeWithSecretHash::TakerPaymentV2 {
                maker_secret_hash,
                taker_secret_hash,
            } => (maker_secret_hash, taker_secret_hash),
            _ => {
                return Err(TransactionErr::Plain(ERRL!(
                    "Unsupported swap tx type for timelock refund"
                )))
            },
        };

        let args = TakerRefundTimelockArgs {
            dex_fee,
            payment_amount,
            maker_address,
            taker_secret_hash: try_tx_s!(taker_secret_hash.try_into()),
            maker_secret_hash: try_tx_s!(maker_secret_hash.try_into()),
            payment_time_lock: args.time_lock,
            token_address,
        };
        let data = try_tx_s!(self.prepare_taker_refund_payment_timelock_data(args).await);

        self.sign_and_send_transaction(
            U256::from(ZERO_VALUE),
            Action::Call(taker_swap_v2_contract),
            data,
            U256::from(gas_limit),
        )
        .compat()
        .await
    }

    pub(crate) async fn refund_taker_funding_secret_impl(
        &self,
        args: RefundFundingSecretArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let (token_address, gas_limit) = self
            .gas_limit_v2
            .gas_limit(
                &self.coin_type,
                EthPaymentType::TakerPayments,
                PaymentMethod::RefundSecret,
            )
            .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;

        let taker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.taker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;
        let taker_secret = try_tx_s!(args.taker_secret.try_into());
        let maker_secret_hash = try_tx_s!(args.maker_secret_hash.try_into());
        let dex_fee = try_tx_s!(wei_from_big_decimal(
            &args.dex_fee.fee_amount().to_decimal(),
            self.decimals
        ));
        let payment_amount = try_tx_s!(wei_from_big_decimal(
            &(args.trading_amount + args.premium_amount),
            self.decimals
        ));
        let maker_address = public_to_address(args.maker_pubkey);

        let refund_args = TakerRefundSecretArgs {
            dex_fee,
            payment_amount,
            maker_address,
            taker_secret,
            maker_secret_hash,
            payment_time_lock: args.payment_time_lock,
            token_address,
        };
        let data = try_tx_s!(self.prepare_taker_refund_payment_secret_data(&refund_args).await);

        self.sign_and_send_transaction(
            U256::from(ZERO_VALUE),
            Action::Call(taker_swap_v2_contract),
            data,
            U256::from(gas_limit),
        )
        .compat()
        .await
    }

    /// Checks that taker payment state is `TakerApproved`.
    /// Accepts a taker-approved payment transaction and returns it if the state is correct.
    pub(crate) async fn search_for_taker_funding_spend_impl(
        &self,
        tx: &SignedEthTx,
    ) -> Result<Option<FundingTxSpend<Self>>, SearchForFundingSpendErr> {
        let (decoded, taker_swap_v2_contract) = self
            .get_decoded_and_swap_contract(tx, TAKER_PAYMENT_APPROVE)
            .await
            .map_err(|e| SearchForFundingSpendErr::Internal(ERRL!("{}", e)))?;
        let taker_status = self
            .payment_status_v2(
                taker_swap_v2_contract,
                decoded[0].clone(), // id from takerPaymentApprove
                &TAKER_SWAP_V2,
                EthPaymentType::TakerPayments,
                TAKER_PAYMENT_STATE_INDX,
            )
            .await
            .map_err(|e| SearchForFundingSpendErr::Internal(ERRL!("{}", e)))?;
        if taker_status == U256::from(TakerPaymentStateV2::TakerApproved as u8) {
            return Ok(Some(FundingTxSpend::TransferredToTakerPayment(tx.clone())));
        }
        Ok(None)
    }

    /// Taker swap contract `spendTakerPayment` method is called for EVM based chains.
    /// Returns maker spent payment transaction.
    pub(crate) async fn sign_and_broadcast_taker_payment_spend_impl(
        &self,
        gen_args: &GenTakerPaymentSpendArgs<'_, Self>,
        secret: &[u8],
    ) -> Result<SignedEthTx, TransactionErr> {
        let (_, gas_limit) = self
            .gas_limit_v2
            .gas_limit(&self.coin_type, EthPaymentType::TakerPayments, PaymentMethod::Spend)
            .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;

        let (taker_swap_v2_contract, approve_func, token_address) = self
            .taker_swap_v2_details(TAKER_PAYMENT_APPROVE, TAKER_PAYMENT_APPROVE)
            .await?;
        let decoded = try_tx_s!(decode_contract_call(approve_func, gen_args.taker_tx.unsigned().data()));
        let taker_status = try_tx_s!(
            self.payment_status_v2(
                taker_swap_v2_contract,
                decoded[0].clone(),
                &TAKER_SWAP_V2,
                EthPaymentType::TakerPayments,
                TAKER_PAYMENT_STATE_INDX,
            )
            .await
        );
        validate_payment_state(
            gen_args.taker_tx,
            taker_status,
            TakerPaymentStateV2::TakerApproved as u8,
        )
        .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;
        let data = try_tx_s!(
            self.prepare_spend_taker_payment_data(gen_args, secret, decoded, token_address)
                .await
        );
        let spend_payment_tx = self
            .sign_and_send_transaction(
                U256::from(ZERO_VALUE),
                Action::Call(taker_swap_v2_contract),
                data,
                U256::from(gas_limit),
            )
            .compat()
            .await?;
        Ok(spend_payment_tx)
    }

    /// Checks that taker payment state is `MakerSpent`.
    /// Accepts maker spent payment transaction and returns it if payment status is correct.
    pub(crate) async fn wait_for_taker_payment_spend_impl(
        &self,
        taker_payment: &SignedEthTx,
        wait_until: u64,
    ) -> MmResult<SignedEthTx, WaitForPaymentSpendError> {
        let (decoded, taker_swap_v2_contract) = self
            .get_decoded_and_swap_contract(taker_payment, "spendTakerPayment")
            .await?;
        loop {
            let taker_status = self
                .payment_status_v2(
                    taker_swap_v2_contract,
                    decoded[0].clone(), // id from spendTakerPayment
                    &TAKER_SWAP_V2,
                    EthPaymentType::TakerPayments,
                    TAKER_PAYMENT_STATE_INDX,
                )
                .await?;
            if taker_status == U256::from(TakerPaymentStateV2::MakerSpent as u8) {
                return Ok(taker_payment.clone());
            }
            let now = now_sec();
            if now > wait_until {
                return MmError::err(WaitForPaymentSpendError::Timeout { wait_until, now });
            }
            Timer::sleep(10.).await;
        }
    }

    /// Prepares data for EtomicSwapTakerV2 contract [ethTakerPayment](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapTakerV2.sol#L44) method
    async fn prepare_taker_eth_funding_data(&self, args: &TakerFundingArgs) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function(ETH_TAKER_PAYMENT)?;
        let id = self.etomic_swap_id_v2(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Uint(args.dex_fee),
            Token::Address(args.maker_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Uint(args.funding_time_lock.into()),
            Token::Uint(args.payment_time_lock.into()),
        ])?;
        Ok(data)
    }

    /// Prepares data for EtomicSwapTakerV2 contract [erc20TakerPayment](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapTakerV2.sol#L83) method
    async fn prepare_taker_erc20_funding_data(
        &self,
        args: &TakerFundingArgs,
        token_address: Address,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function(ERC20_TAKER_PAYMENT)?;
        let id = self.etomic_swap_id_v2(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Uint(args.payment_amount),
            Token::Uint(args.dex_fee),
            Token::Address(token_address),
            Token::Address(args.maker_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Uint(args.funding_time_lock.into()),
            Token::Uint(args.payment_time_lock.into()),
        ])?;
        Ok(data)
    }

    /// Prepares data for EtomicSwapTakerV2 contract [refundTakerPaymentTimelock](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapTakerV2.sol#L208) method
    async fn prepare_taker_refund_payment_timelock_data(
        &self,
        args: TakerRefundTimelockArgs,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function("refundTakerPaymentTimelock")?;
        let id = self.etomic_swap_id_v2(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Uint(args.payment_amount),
            Token::Uint(args.dex_fee),
            Token::Address(args.maker_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Address(args.token_address),
        ])?;
        Ok(data)
    }

    /// Prepares data for EtomicSwapTakerV2 contract [refundTakerPaymentSecret](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapTakerV2.sol#L267) method
    async fn prepare_taker_refund_payment_secret_data(
        &self,
        args: &TakerRefundSecretArgs,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function("refundTakerPaymentSecret")?;
        let id = self.etomic_swap_id_v2(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Uint(args.payment_amount),
            Token::Uint(args.dex_fee),
            Token::Address(args.maker_address),
            Token::FixedBytes(args.taker_secret.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Address(args.token_address),
        ])?;
        Ok(data)
    }

    /// This function constructs the encoded transaction input data required to approve the taker payment ([takerPaymentApprove](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapTakerV2.sol#L128)).
    /// The `decoded` parameter should contain the transaction input data from the `ethTakerPayment` or `erc20TakerPayment` function of the EtomicSwapTakerV2 contract.
    async fn prepare_taker_payment_approve_data(
        &self,
        args: &GenTakerFundingSpendArgs<'_, Self>,
        decoded: Vec<Token>,
        token_address: Address,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function(TAKER_PAYMENT_APPROVE)?;
        let data = match self.coin_type {
            EthCoinType::Eth => {
                check_decoded_length(&decoded, 7)?;
                let dex_fee = match &decoded[1] {
                    Token::Uint(value) => value,
                    _ => return Err(PrepareTxDataError::Internal("Invalid token type for dex fee".into())),
                };
                let amount = args
                    .funding_tx
                    .unsigned()
                    .value()
                    .checked_sub(*dex_fee)
                    .ok_or_else(|| {
                        PrepareTxDataError::Internal("Underflow occurred while calculating amount".into())
                    })?;
                function.encode_input(&[
                    decoded[0].clone(),  // id from ethTakerPayment
                    Token::Uint(amount), // calculated payment amount (tx value - dexFee)
                    decoded[1].clone(),  // dexFee from ethTakerPayment
                    decoded[2].clone(),  // receiver from ethTakerPayment
                    Token::FixedBytes(args.taker_secret_hash.to_vec()),
                    Token::FixedBytes(args.maker_secret_hash.to_vec()),
                    Token::Address(token_address), // should be zero address Address::default()
                ])?
            },
            EthCoinType::Erc20 { .. } => {
                check_decoded_length(&decoded, 9)?;
                function.encode_input(&[
                    decoded[0].clone(), // id from erc20TakerPayment
                    decoded[1].clone(), // amount from erc20TakerPayment
                    decoded[2].clone(), // dexFee from erc20TakerPayment
                    decoded[4].clone(), // receiver from erc20TakerPayment
                    Token::FixedBytes(args.taker_secret_hash.to_vec()),
                    Token::FixedBytes(args.maker_secret_hash.to_vec()),
                    Token::Address(token_address), // erc20 token address from EthCoinType::Erc20
                ])?
            },
            EthCoinType::Nft { .. } => {
                return Err(PrepareTxDataError::Internal("EthCoinType must be ETH or ERC20".into()))
            },
        };
        Ok(data)
    }

    /// Prepares data for EtomicSwapTakerV2 contract [spendTakerPayment](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapTakerV2.sol#L164) method
    async fn prepare_spend_taker_payment_data(
        &self,
        args: &GenTakerPaymentSpendArgs<'_, Self>,
        secret: &[u8],
        decoded: Vec<Token>,
        token_address: Address,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        check_decoded_length(&decoded, 7)?;
        let function = TAKER_SWAP_V2.function("spendTakerPayment")?;
        let taker_address = public_to_address(args.taker_pub);
        let data = function.encode_input(&[
            decoded[0].clone(),                 // id from takerPaymentApprove
            decoded[1].clone(),                 // amount from takerPaymentApprove
            decoded[2].clone(),                 // dexFee from takerPaymentApprove
            Token::Address(taker_address),      // taker address
            decoded[4].clone(),                 // takerSecretHash from ethTakerPayment
            Token::FixedBytes(secret.to_vec()), // makerSecret
            Token::Address(token_address),      // tokenAddress
        ])?;
        Ok(data)
    }

    /// Retrieves the taker smart contract address, the corresponding function, and the token address.
    ///
    /// Depending on the coin type (ETH or ERC20), it fetches the appropriate function name  and token address.
    /// Returns an error if the coin type is NFT or if the `swap_v2_contracts` is None.
    async fn taker_swap_v2_details(
        &self,
        eth_func_name: &str,
        erc20_func_name: &str,
    ) -> Result<(Address, &Function, Address), TransactionErr> {
        let (func, token_address) = match self.coin_type {
            EthCoinType::Eth => (try_tx_s!(TAKER_SWAP_V2.function(eth_func_name)), Address::default()),
            EthCoinType::Erc20 { token_addr, .. } => (try_tx_s!(TAKER_SWAP_V2.function(erc20_func_name)), token_addr),
            EthCoinType::Nft { .. } => {
                return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                    "NFT protocol is not supported for ETH and ERC20 Swaps"
                )))
            },
        };
        let taker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.taker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;
        Ok((taker_swap_v2_contract, func, token_address))
    }

    async fn get_decoded_and_swap_contract(
        &self,
        tx: &SignedEthTx,
        function_name: &str,
    ) -> Result<(Vec<Token>, Address), PrepareTxDataError> {
        let decoded = {
            let func = match self.coin_type {
                EthCoinType::Eth | EthCoinType::Erc20 { .. } => TAKER_SWAP_V2.function(function_name)?,
                EthCoinType::Nft { .. } => {
                    return Err(PrepareTxDataError::Internal(
                        "NFT protocol is not supported for ETH and ERC20 Swaps".to_string(),
                    ));
                },
            };
            decode_contract_call(func, tx.unsigned().data())?
        };
        let taker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.taker_swap_v2_contract)
            .ok_or_else(|| {
                PrepareTxDataError::Internal("Expected swap_v2_contracts to be Some, but found None".to_string())
            })?;

        Ok((decoded, taker_swap_v2_contract))
    }
}

/// Validation function for ETH taker payment data
fn validate_eth_taker_payment_data(
    decoded: &[Token],
    args: &TakerValidationArgs,
    func: &Function,
    tx_value: U256,
) -> Result<(), MmError<ValidateSwapV2TxError>> {
    let checks = vec![
        (0, Token::FixedBytes(args.swap_id.clone()), "id"),
        (1, Token::Uint(args.dex_fee), "dexFee"),
        (2, Token::Address(args.receiver), "receiver"),
        (3, Token::FixedBytes(args.taker_secret_hash.to_vec()), "takerSecretHash"),
        (4, Token::FixedBytes(args.maker_secret_hash.to_vec()), "makerSecretHash"),
        (5, Token::Uint(U256::from(args.funding_time_lock)), "preApproveLockTime"),
        (6, Token::Uint(U256::from(args.payment_time_lock)), "paymentLockTime"),
    ];

    for (index, expected_token, field_name) in checks {
        let token = get_function_input_data(decoded, func, index).map_to_mm(ValidateSwapV2TxError::Internal)?;
        if token != expected_token {
            return MmError::err(ValidateSwapV2TxError::WrongPaymentTx(format!(
                "ETH Taker Payment `{}` {:?} is invalid, expected {:?}",
                field_name,
                decoded.get(index),
                expected_token
            )));
        }
    }
    let total = args.amount.checked_add(args.dex_fee).ok_or_else(|| {
        ValidateSwapV2TxError::Overflow("Overflow occurred while calculating total payment".to_string())
    })?;
    if total != tx_value {
        return MmError::err(ValidateSwapV2TxError::WrongPaymentTx(format!(
            "ETH Taker Payment amount, is invalid, expected {:?}, got {:?}",
            total, tx_value
        )));
    }
    Ok(())
}

/// Validation function for ERC20 taker payment data
fn validate_erc20_taker_payment_data(
    decoded: &[Token],
    args: &TakerValidationArgs,
    func: &Function,
    token_addr: Address,
) -> Result<(), MmError<ValidateSwapV2TxError>> {
    let checks = vec![
        (0, Token::FixedBytes(args.swap_id.clone()), "id"),
        (1, Token::Uint(args.amount), "amount"),
        (2, Token::Uint(args.dex_fee), "dexFee"),
        (3, Token::Address(token_addr), "tokenAddress"),
        (4, Token::Address(args.receiver), "receiver"),
        (5, Token::FixedBytes(args.taker_secret_hash.to_vec()), "takerSecretHash"),
        (6, Token::FixedBytes(args.maker_secret_hash.to_vec()), "makerSecretHash"),
        (7, Token::Uint(U256::from(args.funding_time_lock)), "preApproveLockTime"),
        (8, Token::Uint(U256::from(args.payment_time_lock)), "paymentLockTime"),
    ];

    for (index, expected_token, field_name) in checks {
        let token = get_function_input_data(decoded, func, index).map_to_mm(ValidateSwapV2TxError::Internal)?;
        if token != expected_token {
            return MmError::err(ValidateSwapV2TxError::WrongPaymentTx(format!(
                "ERC20 Taker Payment `{}` {:?} is invalid, expected {:?}",
                field_name,
                decoded.get(index),
                expected_token
            )));
        }
    }
    Ok(())
}

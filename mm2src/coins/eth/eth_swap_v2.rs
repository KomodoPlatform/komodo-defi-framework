use super::eth::{wei_from_big_decimal, EthCoin, EthCoinType, SignedEthTx, TAKER_SWAP_V2};
use super::{decode_contract_call, get_function_input_data, ParseCoinAssocTypes, RefundFundingSecretArgs,
            RefundTakerPaymentArgs, SendTakerFundingArgs, SwapTxTypeWithSecretHash, TakerPaymentStateV2, Transaction,
            TransactionErr, ValidateSwapV2TxError, ValidateSwapV2TxResult, ValidateTakerFundingArgs};
use crate::{GenTakerFundingSpendArgs, GenTakerPaymentSpendArgs};
use enum_derives::EnumFromStringify;
use ethabi::{Contract, Function, Token};
use ethcore_transaction::Action;
use ethereum_types::{Address, Public, U256};
use ethkey::public_to_address;
use futures::compat::Future01CompatExt;
use mm2_err_handle::prelude::{MapToMmResult, MmError};
use mm2_number::BigDecimal;
use std::convert::TryInto;
use std::num::TryFromIntError;
use web3::types::{Transaction as Web3Tx, TransactionId};

const ETH_TAKER_PAYMENT: &str = "ethTakerPayment";
const ERC20_TAKER_PAYMENT: &str = "erc20TakerPayment";

struct TakerFundingArgs {
    dex_fee: U256,
    payment_amount: U256,
    maker_address: Address,
    taker_secret_hash: [u8; 32],
    maker_secret_hash: [u8; 32],
    funding_time_lock: u32,
    payment_time_lock: u32,
}

struct TakerRefundSecretArgs {
    dex_fee: U256,
    payment_amount: U256,
    maker_address: Address,
    taker_secret: [u8; 32],
    maker_secret_hash: [u8; 32],
    payment_time_lock: u32,
    token_address: Address,
}

struct TakerRefundArgs {
    dex_fee: U256,
    payment_amount: U256,
    maker_address: Address,
    taker_secret_hash: [u8; 32],
    maker_secret_hash: [u8; 32],
    payment_time_lock: u32,
    token_address: Address,
}

impl EthCoin {
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
            let funding_time_lock: u32 = try_tx_s!(args.funding_time_lock.try_into());
            let payment_time_lock: u32 = try_tx_s!(args.payment_time_lock.try_into());
            TakerFundingArgs {
                dex_fee,
                payment_amount,
                maker_address,
                taker_secret_hash: try_tx_s!(args.taker_secret_hash.try_into()),
                maker_secret_hash: try_tx_s!(args.maker_secret_hash.try_into()),
                funding_time_lock,
                payment_time_lock,
            }
        };
        match &self.coin_type {
            EthCoinType::Eth => {
                let data = try_tx_s!(self.prepare_taker_eth_funding_data(&funding_args).await);
                let eth_total_payment = dex_fee + payment_amount;
                self.sign_and_send_transaction(
                    eth_total_payment,
                    Action::Call(taker_swap_v2_contract),
                    data,
                    // TODO need new consts and params for v2 calls. now it uses v1
                    U256::from(self.gas_limit.eth_payment),
                )
                .compat()
                .await
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let allowed = self
                    .allowance(taker_swap_v2_contract)
                    .compat()
                    .await
                    .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;
                let data = try_tx_s!(self.prepare_taker_erc20_funding_data(&funding_args, *token_addr).await);
                if allowed < payment_amount {
                    let approved_tx = self.approve(taker_swap_v2_contract, U256::max_value()).compat().await?;
                    self.wait_for_required_allowance(taker_swap_v2_contract, payment_amount, args.funding_time_lock)
                        .compat()
                        .await
                        .map_err(|e| {
                            TransactionErr::Plain(ERRL!(
                                "Allowed value was not updated in time after sending approve transaction {:02x}: {}",
                                approved_tx.tx_hash_as_bytes(),
                                e
                            ))
                        })?;
                }
                self.sign_and_send_transaction(
                    0.into(),
                    Action::Call(taker_swap_v2_contract),
                    data,
                    // TODO need new consts and params for v2 calls. now it uses v1
                    U256::from(self.gas_limit.erc20_payment),
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
            return MmError::err(ValidateSwapV2TxError::Internal(
                "EthCoinType must be ETH or ERC20".to_string(),
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
        let funding_time_lock: u32 = args
            .funding_time_lock
            .try_into()
            .map_to_mm(|e: TryFromIntError| ValidateSwapV2TxError::LocktimeOverflow(e.to_string()))?;
        let payment_time_lock: u32 = args
            .payment_time_lock
            .try_into()
            .map_to_mm(|e: TryFromIntError| ValidateSwapV2TxError::LocktimeOverflow(e.to_string()))?;
        let swap_id = self.etomic_swap_id(payment_time_lock, args.maker_secret_hash);
        let taker_status = self
            .payment_status_v2(
                taker_swap_v2_contract,
                Token::FixedBytes(swap_id.clone()),
                &TAKER_SWAP_V2,
                EthPaymentType::TakerPayments,
                3,
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
                funding_time_lock,
                payment_time_lock,
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

    pub(crate) async fn taker_payment_approve(
        &self,
        args: &GenTakerFundingSpendArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let taker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.taker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;

        let (send_func, token_address) = match self.coin_type {
            EthCoinType::Eth => (try_tx_s!(TAKER_SWAP_V2.function(ETH_TAKER_PAYMENT)), Address::default()),
            EthCoinType::Erc20 { token_addr, .. } => {
                (try_tx_s!(TAKER_SWAP_V2.function(ERC20_TAKER_PAYMENT)), token_addr)
            },
            EthCoinType::Nft { .. } => {
                return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                    "NFT protocol is not supported for ETH and ERC20 Swaps"
                )))
            },
        };
        let decoded = try_tx_s!(decode_contract_call(send_func, args.funding_tx.unsigned().data()));
        let data = try_tx_s!(
            self.prepare_taker_payment_approve_data(args, decoded, token_address)
                .await
        );
        // TODO need new consts and params for v2 calls, create provide approve const
        let gas_limit = match self.coin_type {
            EthCoinType::Eth => U256::from(self.gas_limit.eth_payment),
            EthCoinType::Erc20 { .. } => U256::from(self.gas_limit.eth_payment),
            EthCoinType::Nft { .. } => unreachable!(), // EthCoinType::Nft case was already checked
        };
        let approve_tx = self
            .sign_and_send_transaction(0.into(), Action::Call(taker_swap_v2_contract), data, gas_limit)
            .compat()
            .await?;
        Ok(approve_tx)
    }

    pub(crate) async fn refund_taker_funding_timelock_impl(
        &self,
        args: RefundTakerPaymentArgs<'_>,
    ) -> Result<SignedEthTx, TransactionErr> {
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
        let payment_time_lock: u32 = try_tx_s!(args.time_lock.try_into());
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

        let (token_address, gas_limit) = match &self.coin_type {
            // TODO need new consts and params for v2 calls. now it uses v1
            EthCoinType::Eth => (Address::default(), self.gas_limit.eth_sender_refund),
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
                // TODO need new consts and params for v2 calls. now it uses v1
            } => (*token_addr, self.gas_limit.erc20_sender_refund),
            EthCoinType::Nft { .. } => {
                return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                    "NFT protocol is not supported for ETH and ERC20 Swaps"
                )))
            },
        };
        let args = TakerRefundArgs {
            dex_fee,
            payment_amount,
            maker_address,
            taker_secret_hash: try_tx_s!(taker_secret_hash.try_into()),
            maker_secret_hash: try_tx_s!(maker_secret_hash.try_into()),
            payment_time_lock,
            token_address,
        };
        let data = try_tx_s!(self.prepare_taker_refund_payment_timelock_data(args).await);

        self.sign_and_send_transaction(
            0.into(),
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
        let payment_time_lock: u32 = try_tx_s!(args.payment_time_lock.try_into());
        let (token_address, gas_limit) = match &self.coin_type {
            // TODO need new consts and params for v2 calls. now it uses v1
            EthCoinType::Eth => (Address::default(), self.gas_limit.eth_sender_refund),
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
                // TODO need new consts and params for v2 calls. now it uses v1
            } => (*token_addr, self.gas_limit.erc20_sender_refund),
            EthCoinType::Nft { .. } => {
                return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                    "NFT protocol is not supported for ETH and ERC20 Swaps"
                )))
            },
        };
        let refund_args = TakerRefundSecretArgs {
            dex_fee,
            payment_amount,
            maker_address,
            taker_secret,
            maker_secret_hash,
            payment_time_lock,
            token_address,
        };
        let data = try_tx_s!(self.prepare_taker_refund_payment_secret_data(&refund_args).await);

        self.sign_and_send_transaction(
            0.into(),
            Action::Call(taker_swap_v2_contract),
            data,
            U256::from(gas_limit),
        )
        .compat()
        .await
    }

    pub(crate) async fn sign_and_broadcast_taker_payment_spend_impl(
        &self,
        gen_args: &GenTakerPaymentSpendArgs<'_, Self>,
        secret: &[u8],
    ) -> Result<SignedEthTx, TransactionErr> {
        let taker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.taker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;
        let (approve_func, token_address) = match self.coin_type {
            EthCoinType::Eth => (
                try_tx_s!(TAKER_SWAP_V2.function("takerPaymentApprove")),
                Address::default(),
            ),
            EthCoinType::Erc20 { token_addr, .. } => {
                (try_tx_s!(TAKER_SWAP_V2.function("takerPaymentApprove")), token_addr)
            },
            EthCoinType::Nft { .. } => {
                return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                    "NFT protocol is not supported for ETH and ERC20 Swaps"
                )))
            },
        };
        let (state, decoded) = try_tx_s!(
            self.status_and_decoded_from_tx_data(
                taker_swap_v2_contract,
                &TAKER_SWAP_V2,
                approve_func,
                // taker_tx should be approved tx
                gen_args.taker_tx.unsigned().data(),
                EthPaymentType::TakerPayments,
                3
            )
            .await
        );
        let data = try_tx_s!(
            self.prepare_spend_taker_payment_data(gen_args, secret, decoded, state, token_address)
                .await
        );
        // TODO need new consts and params for v2 calls
        let gas_limit = match self.coin_type {
            EthCoinType::Eth => U256::from(self.gas_limit.eth_receiver_spend),
            EthCoinType::Erc20 { .. } => U256::from(self.gas_limit.erc20_receiver_spend),
            EthCoinType::Nft { .. } => unreachable!(), // EthCoinType::Nft case was already checked
        };
        let payment_tx = self
            .sign_and_send_transaction(0.into(), Action::Call(taker_swap_v2_contract), data, gas_limit)
            .compat()
            .await?;
        Ok(payment_tx)
    }

    /// Prepares data for EtomicSwapTakerV2 contract `ethTakerPayment` method
    ///     function ethTakerPayment(
    ///         bytes32 id,
    ///         uint256 dexFee,
    ///         address receiver,
    ///         bytes32 takerSecretHash,
    ///         bytes32 makerSecretHash,
    ///         uint32 preApproveLockTime,
    ///         uint32 paymentLockTime
    async fn prepare_taker_eth_funding_data(&self, args: &TakerFundingArgs) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function(ETH_TAKER_PAYMENT)?;
        let id = self.etomic_swap_id(args.payment_time_lock, &args.maker_secret_hash);
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

    /// Prepares data for EtomicSwapTakerV2 contract `erc20TakerPayment` method
    ///     function erc20TakerPayment(
    ///         bytes32 id,
    ///         uint256 amount,
    ///         uint256 dexFee,
    ///         address tokenAddress,
    ///         address receiver,
    ///         bytes32 takerSecretHash,
    ///         bytes32 makerSecretHash,
    ///         uint32 preApproveLockTime,
    ///         uint32 paymentLockTime
    async fn prepare_taker_erc20_funding_data(
        &self,
        args: &TakerFundingArgs,
        token_address: Address,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function(ERC20_TAKER_PAYMENT)?;
        let id = self.etomic_swap_id(args.payment_time_lock, &args.maker_secret_hash);
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

    /// Prepares data for EtomicSwapTakerV2 contract `refundTakerPaymentTimelock` method
    /// function refundMakerPaymentTimelock(
    ///         bytes32 id,
    ///         uint256 amount,
    ///         address taker,
    ///         bytes32 takerSecretHash,
    ///         bytes32 makerSecretHash,
    ///         address tokenAddress
    async fn prepare_taker_refund_payment_timelock_data(
        &self,
        args: TakerRefundArgs,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function("refundTakerPaymentTimelock")?;
        let id = self.etomic_swap_id(args.payment_time_lock, &args.maker_secret_hash);
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

    /// Prepares data for EtomicSwapTakerV2 contract `refundTakerPaymentSecret` method
    /// function refundTakerPaymentSecret(
    ///         bytes32 id,
    ///         uint256 amount,
    ///         uint256 dexFee,
    ///         address maker,
    ///         bytes32 takerSecret,
    ///         bytes32 makerSecretHash,
    ///         address tokenAddress
    async fn prepare_taker_refund_payment_secret_data(
        &self,
        args: &TakerRefundSecretArgs,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function("refundTakerPaymentSecret")?;
        let id = self.etomic_swap_id(args.payment_time_lock, &args.maker_secret_hash);
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

    /// This function constructs the encoded transaction input data required to approve the taker payment.
    /// The `decoded` parameter should contain the transaction input data from the `ethTakerPayment` or `erc20TakerPayment` function of the EtomicSwapTakerV2 contract.
    async fn prepare_taker_payment_approve_data(
        &self,
        args: &GenTakerFundingSpendArgs<'_, Self>,
        decoded: Vec<Token>,
        token_address: Address,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = TAKER_SWAP_V2.function("takerPaymentApprove")?;
        let data = match self.coin_type {
            EthCoinType::Eth => {
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
            EthCoinType::Erc20 { .. } => function.encode_input(&[
                decoded[0].clone(), // id from erc20TakerPayment
                decoded[1].clone(), // amount from erc20TakerPayment
                decoded[2].clone(), // dexFee from erc20TakerPayment
                decoded[4].clone(), // receiver from erc20TakerPayment
                Token::FixedBytes(args.taker_secret_hash.to_vec()),
                Token::FixedBytes(args.maker_secret_hash.to_vec()),
                Token::Address(token_address), // erc20 token address from EthCoinType::Erc20
            ])?,
            EthCoinType::Nft { .. } => {
                return Err(PrepareTxDataError::Internal("EthCoinType must be ETH or ERC20".into()))
            },
        };
        Ok(data)
    }

    async fn prepare_spend_taker_payment_data(
        &self,
        args: &GenTakerPaymentSpendArgs<'_, Self>,
        secret: &[u8],
        decoded: Vec<Token>,
        state: U256,
        token_address: Address,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        validate_payment_state(args.taker_tx, state, TakerPaymentStateV2::TakerApproved as u8)?;
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

    /// Retrieves the payment status from a given smart contract address based on the swap ID and state type.
    pub(crate) async fn payment_status_v2(
        &self,
        swap_address: Address,
        swap_id: Token,
        contract_abi: &Contract,
        payment_type: EthPaymentType,
        state_index: usize,
    ) -> Result<U256, PaymentStatusErr> {
        let function_name = payment_type.as_str();
        let function = contract_abi.function(function_name)?;
        let data = function.encode_input(&[swap_id])?;
        let bytes = self
            .call_request(self.my_addr().await, swap_address, None, Some(data.into()))
            .await?;
        let decoded_tokens = function.decode_output(&bytes.0)?;

        let state = decoded_tokens.get(state_index).ok_or_else(|| {
            PaymentStatusErr::Internal(ERRL!(
                "Payment status must contain 'state' as the {} token",
                state_index
            ))
        })?;
        match state {
            Token::Uint(state) => Ok(*state),
            _ => Err(PaymentStatusErr::Internal(ERRL!(
                "Payment status must be Uint, got {:?}",
                state
            ))),
        }
    }

    /// Returns payment status and decoded tx input from bytes of contract call
    async fn status_and_decoded_from_tx_data(
        &self,
        swap_address: Address,
        contract_abi: &Contract,
        func: &Function,
        contract_call_bytes: &[u8],
        payment_type: EthPaymentType,
        state_index: usize,
    ) -> Result<(U256, Vec<Token>), PaymentStatusErr> {
        let decoded = decode_contract_call(func, contract_call_bytes)?;
        let state = self
            .payment_status_v2(
                swap_address,
                // swap_id has 0 index
                decoded[0].clone(),
                contract_abi,
                payment_type,
                state_index,
            )
            .await?;
        Ok((state, decoded))
    }
}

#[derive(Debug, Display, EnumFromStringify)]
pub(crate) enum PrepareTxDataError {
    #[from_stringify("ethabi::Error")]
    #[display(fmt = "Abi error: {}", _0)]
    AbiError(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

// TODO validate premium when add its support in swap_v2
fn validate_payment_args<'a>(
    taker_secret_hash: &'a [u8],
    maker_secret_hash: &'a [u8],
    trading_amount: &BigDecimal,
) -> Result<(), String> {
    if !is_positive(trading_amount) {
        return Err("trading_amount must be a positive value".to_string());
    }
    if taker_secret_hash.len() != 32 {
        return Err("taker_secret_hash must be 32 bytes".to_string());
    }
    if maker_secret_hash.len() != 32 {
        return Err("maker_secret_hash must be 32 bytes".to_string());
    }
    Ok(())
}

/// function to check if BigDecimal is a positive value
#[inline(always)]
fn is_positive(amount: &BigDecimal) -> bool { amount > &BigDecimal::from(0) }

pub(crate) fn validate_from_to_and_status(
    tx_from_rpc: &Web3Tx,
    expected_from: Address,
    expected_to: Address,
    status: U256,
    expected_status: u8,
) -> Result<(), MmError<ValidatePaymentV2Err>> {
    if status != U256::from(expected_status) {
        return MmError::err(ValidatePaymentV2Err::UnexpectedPaymentState(format!(
            "Payment state is not PaymentSent, got {}",
            status
        )));
    }
    if tx_from_rpc.from != Some(expected_from) {
        return MmError::err(ValidatePaymentV2Err::WrongPaymentTx(format!(
            "Payment tx {:?} was sent from wrong address, expected {:?}",
            tx_from_rpc, expected_from
        )));
    }
    // (in NFT case) as NFT owner calls "safeTransferFrom" directly, then in Transaction 'to' field we expect token_address
    if tx_from_rpc.to != Some(expected_to) {
        return MmError::err(ValidatePaymentV2Err::WrongPaymentTx(format!(
            "Payment tx {:?} was sent to wrong address, expected {:?}",
            tx_from_rpc, expected_to,
        )));
    }
    Ok(())
}

struct TakerValidationArgs<'a> {
    swap_id: Vec<u8>,
    amount: U256,
    dex_fee: U256,
    receiver: Address,
    taker_secret_hash: &'a [u8],
    maker_secret_hash: &'a [u8],
    funding_time_lock: u32,
    payment_time_lock: u32,
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
    let total = args.dex_fee + args.amount;
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

pub(crate) fn validate_payment_state(
    tx: &SignedEthTx,
    state: U256,
    expected_state: u8,
) -> Result<(), PrepareTxDataError> {
    if state != U256::from(expected_state) {
        return Err(PrepareTxDataError::Internal(ERRL!(
            "Payment {:?} state is not {}, got {}",
            tx,
            expected_state,
            state
        )));
    }
    Ok(())
}

#[derive(Debug, Display)]
pub(crate) enum ValidatePaymentV2Err {
    UnexpectedPaymentState(String),
    WrongPaymentTx(String),
}

pub(crate) enum EthPaymentType {
    MakerPayments,
    TakerPayments,
}

impl EthPaymentType {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            EthPaymentType::MakerPayments => "makerPayments",
            EthPaymentType::TakerPayments => "takerPayments",
        }
    }
}

#[derive(Debug, Display, EnumFromStringify)]
pub(crate) enum PaymentStatusErr {
    #[from_stringify("ethabi::Error")]
    #[display(fmt = "Abi error: {}", _0)]
    AbiError(String),
    #[from_stringify("web3::Error")]
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
    #[display(fmt = "Tx deserialization error: {}", _0)]
    TxDeserializationError(String),
}

use super::{validate_from_to_and_status, validate_payment_args, EthPaymentType, PrepareTxDataError, ZERO_VALUE};
use crate::coin_errors::{ValidatePaymentError, ValidatePaymentResult};
use crate::eth::{decode_contract_call, get_function_input_data, wei_from_big_decimal, EthCoin, EthCoinType,
                 MakerPaymentStateV2, SignedEthTx, MAKER_SWAP_V2};
use crate::{ParseCoinAssocTypes, RefundMakerPaymentSecretArgs, RefundMakerPaymentTimelockArgs, SendMakerPaymentArgs,
            SpendMakerPaymentArgs, SwapTxTypeWithSecretHash, Transaction, TransactionErr, ValidateMakerPaymentArgs,
            WaitForPaymentSpendError};
use ethabi::{Function, Token};
use ethcore_transaction::Action;
use ethereum_types::{Address, Public, U256};
use ethkey::public_to_address;
use futures::compat::Future01CompatExt;
use mm2_err_handle::mm_error::MmError;
use mm2_err_handle::prelude::{MapToMmResult, MmResult};
use std::convert::TryInto;
use web3::types::TransactionId;

const ETH_MAKER_PAYMENT: &str = "ethMakerPayment";
const ERC20_MAKER_PAYMENT: &str = "erc20MakerPayment";

struct MakerPaymentArgs {
    taker_address: Address,
    taker_secret_hash: [u8; 32],
    maker_secret_hash: [u8; 32],
    payment_time_lock: u64,
}

struct MakerValidationArgs<'a> {
    swap_id: Vec<u8>,
    amount: U256,
    taker: Address,
    taker_secret_hash: &'a [u8],
    maker_secret_hash: &'a [u8],
    payment_time_lock: u64,
}

struct MakerRefundArgs {
    payment_amount: U256,
    taker_address: Address,
    taker_secret: [u8; 32],
    taker_secret_hash: [u8; 32],
    maker_secret_hash: [u8; 32],
    payment_time_lock: u64,
    token_address: Address,
}

impl EthCoin {
    pub(crate) async fn send_maker_payment_v2_impl(
        &self,
        args: SendMakerPaymentArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let maker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.maker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;
        let payment_amount = try_tx_s!(wei_from_big_decimal(&args.amount, self.decimals));
        let payment_args = {
            let taker_address = public_to_address(args.taker_pub);
            MakerPaymentArgs {
                taker_address,
                taker_secret_hash: try_tx_s!(args.taker_secret_hash.try_into()),
                maker_secret_hash: try_tx_s!(args.maker_secret_hash.try_into()),
                payment_time_lock: args.time_lock,
            }
        };
        match &self.coin_type {
            EthCoinType::Eth => {
                let data = try_tx_s!(self.prepare_maker_eth_payment_data(&payment_args).await);
                self.sign_and_send_transaction(
                    payment_amount,
                    Action::Call(maker_swap_v2_contract),
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
                    .allowance(maker_swap_v2_contract)
                    .compat()
                    .await
                    .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;
                let data = try_tx_s!(
                    self.prepare_maker_erc20_payment_data(&payment_args, payment_amount, *token_addr)
                        .await
                );
                if allowed < payment_amount {
                    let approved_tx = self.approve(maker_swap_v2_contract, U256::max_value()).compat().await?;
                    self.wait_for_required_allowance(maker_swap_v2_contract, payment_amount, args.time_lock)
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
                    U256::from(ZERO_VALUE),
                    Action::Call(maker_swap_v2_contract),
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

    pub(crate) async fn validate_maker_payment_v2_impl(
        &self,
        args: ValidateMakerPaymentArgs<'_, Self>,
    ) -> ValidatePaymentResult<()> {
        if let EthCoinType::Nft { .. } = self.coin_type {
            return MmError::err(ValidatePaymentError::ProtocolNotSupported(
                "NFT protocol is not supported for ETH and ERC20 Swaps".to_string(),
            ));
        }
        let maker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.maker_swap_v2_contract)
            .ok_or_else(|| {
                ValidatePaymentError::InternalError("Expected swap_v2_contracts to be Some, but found None".to_string())
            })?;
        validate_payment_args(args.taker_secret_hash, args.maker_secret_hash, &args.amount)
            .map_to_mm(ValidatePaymentError::InternalError)?;
        let maker_address = public_to_address(args.maker_pub);
        let swap_id = self.etomic_swap_id_v2(args.time_lock, args.maker_secret_hash);
        let maker_status = self
            .payment_status_v2(
                maker_swap_v2_contract,
                Token::FixedBytes(swap_id.clone()),
                &MAKER_SWAP_V2,
                EthPaymentType::MakerPayments,
                2,
            )
            .await?;

        let tx_from_rpc = self
            .transaction(TransactionId::Hash(args.maker_payment_tx.tx_hash()))
            .await?;
        let tx_from_rpc = tx_from_rpc.as_ref().ok_or_else(|| {
            ValidatePaymentError::TxDoesNotExist(format!(
                "Didn't find provided tx {:?} on ETH node",
                args.maker_payment_tx.tx_hash()
            ))
        })?;
        validate_from_to_and_status(
            tx_from_rpc,
            maker_address,
            maker_swap_v2_contract,
            maker_status,
            MakerPaymentStateV2::PaymentSent as u8,
        )?;

        let validation_args = {
            let amount = wei_from_big_decimal(&args.amount, self.decimals)?;
            MakerValidationArgs {
                swap_id,
                amount,
                taker: self.my_addr().await,
                taker_secret_hash: args.taker_secret_hash,
                maker_secret_hash: args.maker_secret_hash,
                payment_time_lock: args.time_lock,
            }
        };
        match self.coin_type {
            EthCoinType::Eth => {
                let function = MAKER_SWAP_V2.function(ETH_MAKER_PAYMENT)?;
                let decoded = decode_contract_call(function, &tx_from_rpc.input.0)?;
                validate_eth_maker_payment_data(&decoded, &validation_args, function, tx_from_rpc.value)?;
            },
            EthCoinType::Erc20 { token_addr, .. } => {
                let function = MAKER_SWAP_V2.function(ERC20_MAKER_PAYMENT)?;
                let decoded = decode_contract_call(function, &tx_from_rpc.input.0)?;
                validate_erc20_maker_payment_data(&decoded, &validation_args, function, token_addr)?;
            },
            EthCoinType::Nft { .. } => unreachable!(),
        }
        Ok(())
    }

    pub(crate) async fn refund_maker_payment_v2_timelock_impl(
        &self,
        args: RefundMakerPaymentTimelockArgs<'_>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let (token_address, gas_limit) = match &self.coin_type {
            // TODO need new maker_v2_refund_timelock const and param for v2 calls.
            EthCoinType::Eth => (Address::default(), self.gas_limit.eth_sender_refund),
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => (*token_addr, self.gas_limit.erc20_sender_refund),
            EthCoinType::Nft { .. } => {
                return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                    "NFT protocol is not supported for ETH and ERC20 Swaps"
                )))
            },
        };

        let maker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.maker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;
        let payment_amount = try_tx_s!(wei_from_big_decimal(&args.amount, self.decimals));
        let (maker_secret_hash, taker_secret_hash) = match args.tx_type_with_secret_hash {
            SwapTxTypeWithSecretHash::MakerPaymentV2 {
                maker_secret_hash,
                taker_secret_hash,
            } => (maker_secret_hash, taker_secret_hash),
            _ => {
                return Err(TransactionErr::Plain(ERRL!(
                    "Unsupported swap tx type for timelock refund"
                )))
            },
        };
        let args = {
            let taker_address = public_to_address(&Public::from_slice(args.taker_pub));
            MakerRefundArgs {
                payment_amount,
                taker_address,
                taker_secret: [0u8; 32],
                taker_secret_hash: try_tx_s!(taker_secret_hash.try_into()),
                maker_secret_hash: try_tx_s!(maker_secret_hash.try_into()),
                payment_time_lock: args.time_lock,
                token_address,
            }
        };
        let data = try_tx_s!(self.prepare_maker_refund_payment_timelock_data(args).await);

        self.sign_and_send_transaction(
            U256::from(ZERO_VALUE),
            Action::Call(maker_swap_v2_contract),
            data,
            U256::from(gas_limit),
        )
        .compat()
        .await
    }

    pub(crate) async fn refund_maker_payment_v2_secret_impl(
        &self,
        args: RefundMakerPaymentSecretArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let (token_address, gas_limit) = match &self.coin_type {
            // TODO need new maker_v2_refund_secret const and param for v2 calls.
            EthCoinType::Eth => (Address::default(), self.gas_limit.eth_sender_refund),
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => (*token_addr, self.gas_limit.erc20_sender_refund),
            EthCoinType::Nft { .. } => {
                return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                    "NFT protocol is not supported for ETH and ERC20 Swaps"
                )))
            },
        };

        let maker_swap_v2_contract = self
            .swap_v2_contracts
            .as_ref()
            .map(|contracts| contracts.maker_swap_v2_contract)
            .ok_or_else(|| TransactionErr::Plain(ERRL!("Expected swap_v2_contracts to be Some, but found None")))?;
        let taker_secret = try_tx_s!(args.taker_secret.try_into());
        let maker_secret_hash = try_tx_s!(args.maker_secret_hash.try_into());
        let payment_amount = try_tx_s!(wei_from_big_decimal(&args.amount, self.decimals));
        let args = {
            let taker_address = public_to_address(args.taker_pub);
            MakerRefundArgs {
                payment_amount,
                taker_address,
                taker_secret,
                taker_secret_hash: [0u8; 32],
                maker_secret_hash,
                payment_time_lock: args.time_lock,
                token_address,
            }
        };
        let data = try_tx_s!(self.prepare_maker_refund_payment_secret_data(args).await);

        self.sign_and_send_transaction(
            U256::from(ZERO_VALUE),
            Action::Call(maker_swap_v2_contract),
            data,
            U256::from(gas_limit),
        )
        .compat()
        .await
    }

    pub(crate) async fn spend_maker_payment_v2_impl(
        &self,
        _args: SpendMakerPaymentArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        todo!()
    }

    pub(crate) async fn wait_for_maker_payment_spend_impl(
        &self,
        _maker_payment: &SignedEthTx,
        _wait_until: u64,
    ) -> MmResult<SignedEthTx, WaitForPaymentSpendError> {
        todo!()
    }

    /// Prepares data for EtomicSwapMakerV2 contract [ethMakerPayment](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapMakerV2.sol#L30) method
    async fn prepare_maker_eth_payment_data(&self, args: &MakerPaymentArgs) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = MAKER_SWAP_V2.function(ETH_MAKER_PAYMENT)?;
        let id = self.etomic_swap_id_v2(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Address(args.taker_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Uint(args.payment_time_lock.into()),
        ])?;
        Ok(data)
    }

    /// Prepares data for EtomicSwapMakerV2 contract [erc20MakerPayment](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapMakerV2.sol#L64) method
    async fn prepare_maker_erc20_payment_data(
        &self,
        args: &MakerPaymentArgs,
        payment_amount: U256,
        token_address: Address,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = MAKER_SWAP_V2.function(ERC20_MAKER_PAYMENT)?;
        let id = self.etomic_swap_id_v2(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Uint(payment_amount),
            Token::Address(token_address),
            Token::Address(args.taker_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Uint(args.payment_time_lock.into()),
        ])?;
        Ok(data)
    }

    /// Prepares data for EtomicSwapMakerV2 contract [refundMakerPaymentTimelock](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapMakerV2.sol#L144) method
    async fn prepare_maker_refund_payment_timelock_data(
        &self,
        args: MakerRefundArgs,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = MAKER_SWAP_V2.function("refundMakerPaymentTimelock")?;
        let id = self.etomic_swap_id_v2(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Uint(args.payment_amount),
            Token::Address(args.taker_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Address(args.token_address),
        ])?;
        Ok(data)
    }

    /// Prepares data for EtomicSwapMakerV2 contract [refundMakerPaymentSecret](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapMakerV2.sol#L190) method
    async fn prepare_maker_refund_payment_secret_data(
        &self,
        args: MakerRefundArgs,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let function = MAKER_SWAP_V2.function("refundMakerPaymentSecret")?;
        let id = self.etomic_swap_id_v2(args.payment_time_lock, &args.maker_secret_hash);
        let data = function.encode_input(&[
            Token::FixedBytes(id),
            Token::Uint(args.payment_amount),
            Token::Address(args.taker_address),
            Token::FixedBytes(args.taker_secret.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Address(args.token_address),
        ])?;
        Ok(data)
    }
}

/// Validation function for ETH maker payment data
fn validate_eth_maker_payment_data(
    decoded: &[Token],
    args: &MakerValidationArgs,
    func: &Function,
    tx_value: U256,
) -> Result<(), MmError<ValidatePaymentError>> {
    let checks = vec![
        (0, Token::FixedBytes(args.swap_id.clone()), "id"),
        (1, Token::Address(args.taker), "taker"),
        (2, Token::FixedBytes(args.taker_secret_hash.to_vec()), "takerSecretHash"),
        (3, Token::FixedBytes(args.maker_secret_hash.to_vec()), "makerSecretHash"),
        (4, Token::Uint(U256::from(args.payment_time_lock)), "paymentLockTime"),
    ];

    for (index, expected_token, field_name) in checks {
        let token = get_function_input_data(decoded, func, index).map_to_mm(ValidatePaymentError::InternalError)?;
        if token != expected_token {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                "ETH Maker Payment `{}` {:?} is invalid, expected {:?}",
                field_name,
                decoded.get(index),
                expected_token
            )));
        }
    }
    if args.amount != tx_value {
        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
            "ETH Maker Payment amount, is invalid, expected {:?}, got {:?}",
            args.amount, tx_value
        )));
    }
    Ok(())
}

/// Validation function for ERC20 maker payment data
fn validate_erc20_maker_payment_data(
    decoded: &[Token],
    args: &MakerValidationArgs,
    func: &Function,
    token_addr: Address,
) -> Result<(), MmError<ValidatePaymentError>> {
    let checks = vec![
        (0, Token::FixedBytes(args.swap_id.clone()), "id"),
        (1, Token::Uint(args.amount), "amount"),
        (2, Token::Address(token_addr), "tokenAddress"),
        (3, Token::Address(args.taker), "taker"),
        (4, Token::FixedBytes(args.taker_secret_hash.to_vec()), "takerSecretHash"),
        (5, Token::FixedBytes(args.maker_secret_hash.to_vec()), "makerSecretHash"),
        (6, Token::Uint(U256::from(args.payment_time_lock)), "paymentLockTime"),
    ];

    for (index, expected_token, field_name) in checks {
        let token = get_function_input_data(decoded, func, index).map_to_mm(ValidatePaymentError::InternalError)?;
        if token != expected_token {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                "ERC20 Maker Payment `{}` {:?} is invalid, expected {:?}",
                field_name,
                decoded.get(index),
                expected_token
            )));
        }
    }
    Ok(())
}

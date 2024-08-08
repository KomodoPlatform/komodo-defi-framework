use crate::coin_errors::{ValidatePaymentError, ValidatePaymentResult};
use ethabi::{Contract, Token};
use ethcore_transaction::Action;
use ethereum_types::{Address, U256};
use ethkey::public_to_address;
use futures::compat::Future01CompatExt;
use mm2_err_handle::prelude::{MapToMmResult, MmError, MmResult};
use mm2_number::BigDecimal;
use std::convert::TryInto;
use web3::types::{Transaction as Web3Tx, TransactionId};

pub(crate) mod errors;
use errors::{Erc721FunctionError, HtlcParamsError, PaymentStatusErr, PrepareTxDataError};
mod structs;
use structs::{ExpectedHtlcParams, PaymentType, ValidationParams};

use super::ContractType;
use crate::eth::{decode_contract_call, EthCoin, EthCoinType, MakerPaymentStateV2, SignedEthTx, ERC1155_CONTRACT,
                 ERC721_CONTRACT, NFT_MAKER_SWAP_V2};
use crate::{ParseCoinAssocTypes, RefundNftMakerPaymentArgs, SendNftMakerPaymentArgs, SpendNftMakerPaymentArgs,
            TransactionErr, ValidateNftMakerPaymentArgs};

impl EthCoin {
    pub(crate) async fn send_nft_maker_payment_v2_impl(
        &self,
        args: SendNftMakerPaymentArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        match &self.coin_type {
            EthCoinType::Nft { .. } => {
                try_tx_s!(validate_payment_args(
                    args.taker_secret_hash,
                    args.maker_secret_hash,
                    &args.amount,
                    args.nft_swap_info.contract_type
                ));
                let htlc_data = try_tx_s!(self.prepare_htlc_data(&args));

                let data = try_tx_s!(self.prepare_nft_maker_payment_v2_data(&args, htlc_data).await);
                self.sign_and_send_transaction(
                    0.into(),
                    Action::Call(*args.nft_swap_info.token_address),
                    data,
                    U256::from(self.gas_limit.eth_max_trade_gas), // TODO: fix to a more accurate const or estimated value
                )
                .compat()
                .await
            },
            EthCoinType::Eth | EthCoinType::Erc20 { .. } => Err(TransactionErr::ProtocolNotSupported(
                "ETH and ERC20 Protocols are not supported for NFT Swaps".to_string(),
            )),
        }
    }

    pub(crate) async fn validate_nft_maker_payment_v2_impl(
        &self,
        args: ValidateNftMakerPaymentArgs<'_, Self>,
    ) -> ValidatePaymentResult<()> {
        match self.coin_type {
            EthCoinType::Nft { .. } => {
                let contract_type = args.nft_swap_info.contract_type;
                validate_payment_args(
                    args.taker_secret_hash,
                    args.maker_secret_hash,
                    &args.amount,
                    contract_type,
                )
                .map_err(ValidatePaymentError::InternalError)?;
                let etomic_swap_contract = args.nft_swap_info.swap_contract_address;
                let token_address = args.nft_swap_info.token_address;
                let maker_address = public_to_address(args.maker_pub);
                let time_lock_u32 = args
                    .time_lock
                    .try_into()
                    .map_err(ValidatePaymentError::TimelockOverflow)?;
                let swap_id = self.etomic_swap_id(time_lock_u32, args.maker_secret_hash);
                let maker_status = self
                    .payment_status_v2(
                        *etomic_swap_contract,
                        Token::FixedBytes(swap_id.clone()),
                        &NFT_MAKER_SWAP_V2,
                        PaymentType::MakerPayments,
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
                validate_from_to_and_maker_status(tx_from_rpc, maker_address, *token_address, maker_status).await?;

                let (decoded, bytes_index) = get_decoded_tx_data_and_bytes_index(contract_type, &tx_from_rpc.input.0)?;

                let amount = if matches!(contract_type, &ContractType::Erc1155) {
                    Some(args.amount.to_string())
                } else {
                    None
                };

                let validation_params = ValidationParams {
                    maker_address,
                    etomic_swap_contract: *etomic_swap_contract,
                    token_id: args.nft_swap_info.token_id,
                    amount,
                };
                validate_decoded_data(&decoded, &validation_params)?;

                let taker_address = public_to_address(args.taker_pub);
                let htlc_params = ExpectedHtlcParams {
                    swap_id,
                    taker_address,
                    token_address: *token_address,
                    taker_secret_hash: args.taker_secret_hash.to_vec(),
                    maker_secret_hash: args.maker_secret_hash.to_vec(),
                    time_lock: U256::from(args.time_lock),
                };
                decode_and_validate_htlc_params(decoded, bytes_index, htlc_params)?;
            },
            EthCoinType::Eth | EthCoinType::Erc20 { .. } => {
                return MmError::err(ValidatePaymentError::InternalError(
                    "EthCoinType must be Nft".to_string(),
                ))
            },
        }
        Ok(())
    }

    pub(crate) async fn spend_nft_maker_payment_v2_impl(
        &self,
        args: SpendNftMakerPaymentArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        match self.coin_type {
            EthCoinType::Nft { .. } => {
                let etomic_swap_contract = args.swap_contract_address;
                if args.maker_secret.len() != 32 {
                    return Err(TransactionErr::Plain(ERRL!("maker_secret must be 32 bytes")));
                }
                let (decoded, bytes_index) = try_tx_s!(get_decoded_tx_data_and_bytes_index(
                    args.contract_type,
                    args.maker_payment_tx.unsigned().data()
                ));

                let (state, htlc_params) = try_tx_s!(
                    self.status_and_htlc_params_from_tx_data(
                        *etomic_swap_contract,
                        &NFT_MAKER_SWAP_V2,
                        &decoded,
                        bytes_index,
                        PaymentType::MakerPayments,
                    )
                    .await
                );
                let data = try_tx_s!(self.prepare_spend_nft_maker_v2_data(&args, decoded, htlc_params, state));
                self.sign_and_send_transaction(
                    0.into(),
                    Action::Call(*etomic_swap_contract),
                    data,
                    U256::from(self.gas_limit.eth_max_trade_gas), // TODO: fix to a more accurate const or estimated value
                )
                .compat()
                .await
            },
            EthCoinType::Eth | EthCoinType::Erc20 { .. } => Err(TransactionErr::ProtocolNotSupported(
                "ETH and ERC20 Protocols are not supported for NFT Swaps".to_string(),
            )),
        }
    }

    pub(crate) async fn refund_nft_maker_payment_v2_timelock_impl(
        &self,
        args: RefundNftMakerPaymentArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        match self.coin_type {
            EthCoinType::Nft { .. } => {
                let etomic_swap_contract = args.swap_contract_address;
                let (decoded, bytes_index) = try_tx_s!(get_decoded_tx_data_and_bytes_index(
                    args.contract_type,
                    args.maker_payment_tx.unsigned().data()
                ));

                let (state, htlc_params) = try_tx_s!(
                    self.status_and_htlc_params_from_tx_data(
                        *etomic_swap_contract,
                        &NFT_MAKER_SWAP_V2,
                        &decoded,
                        bytes_index,
                        PaymentType::MakerPayments,
                    )
                    .await
                );
                let data =
                    try_tx_s!(self.prepare_refund_nft_maker_payment_v2_timelock(&args, decoded, htlc_params, state));
                self.sign_and_send_transaction(
                    0.into(),
                    Action::Call(*etomic_swap_contract),
                    data,
                    U256::from(self.gas_limit.eth_max_trade_gas), // TODO: fix to a more accurate const or estimated value
                )
                .compat()
                .await
            },
            EthCoinType::Eth | EthCoinType::Erc20 { .. } => Err(TransactionErr::ProtocolNotSupported(
                "ETH and ERC20 Protocols are not supported for NFT Swaps".to_string(),
            )),
        }
    }

    pub(crate) async fn refund_nft_maker_payment_v2_secret_impl(
        &self,
        args: RefundNftMakerPaymentArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        match self.coin_type {
            EthCoinType::Nft { .. } => {
                let etomic_swap_contract = args.swap_contract_address;
                let (decoded, bytes_index) = try_tx_s!(get_decoded_tx_data_and_bytes_index(
                    args.contract_type,
                    args.maker_payment_tx.unsigned().data()
                ));

                let (state, htlc_params) = try_tx_s!(
                    self.status_and_htlc_params_from_tx_data(
                        *etomic_swap_contract,
                        &NFT_MAKER_SWAP_V2,
                        &decoded,
                        bytes_index,
                        PaymentType::MakerPayments,
                    )
                    .await
                );

                let data =
                    try_tx_s!(self.prepare_refund_nft_maker_payment_v2_secret(&args, decoded, htlc_params, state));
                self.sign_and_send_transaction(
                    0.into(),
                    Action::Call(*etomic_swap_contract),
                    data,
                    U256::from(self.gas_limit.eth_max_trade_gas), // TODO: fix to a more accurate const or estimated value
                )
                .compat()
                .await
            },
            EthCoinType::Eth | EthCoinType::Erc20 { .. } => Err(TransactionErr::ProtocolNotSupported(
                "ETH and ERC20 Protocols are not supported for NFT Swaps".to_string(),
            )),
        }
    }

    async fn prepare_nft_maker_payment_v2_data(
        &self,
        args: &SendNftMakerPaymentArgs<'_, Self>,
        htlc_data: Vec<u8>,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        match args.nft_swap_info.contract_type {
            ContractType::Erc1155 => {
                let function = ERC1155_CONTRACT.function("safeTransferFrom")?;
                let amount_u256 = U256::from_dec_str(&args.amount.to_string())
                    .map_err(|e| PrepareTxDataError::Internal(e.to_string()))?;
                let data = function.encode_input(&[
                    Token::Address(self.my_addr().await),
                    Token::Address(*args.nft_swap_info.swap_contract_address),
                    Token::Uint(U256::from(args.nft_swap_info.token_id)),
                    Token::Uint(amount_u256),
                    Token::Bytes(htlc_data),
                ])?;
                Ok(data)
            },
            ContractType::Erc721 => {
                let function = erc721_transfer_with_data()?;
                let data = function.encode_input(&[
                    Token::Address(self.my_addr().await),
                    Token::Address(*args.nft_swap_info.swap_contract_address),
                    Token::Uint(U256::from(args.nft_swap_info.token_id)),
                    Token::Bytes(htlc_data),
                ])?;
                Ok(data)
            },
        }
    }

    fn prepare_htlc_data(&self, args: &SendNftMakerPaymentArgs<'_, Self>) -> Result<Vec<u8>, PrepareTxDataError> {
        let taker_address = public_to_address(args.taker_pub);
        let time_lock_u32 = args
            .time_lock
            .try_into()
            .map_err(|e| PrepareTxDataError::Internal(ERRL!("{}", e)))?;
        let id = self.etomic_swap_id(time_lock_u32, args.maker_secret_hash);
        let encoded = ethabi::encode(&[
            Token::FixedBytes(id),
            Token::Address(taker_address),
            Token::Address(*args.nft_swap_info.token_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Uint(U256::from(time_lock_u32)),
        ]);
        Ok(encoded)
    }

    /// Retrieves the payment status from a given smart contract address based on the swap ID and state type.
    async fn payment_status_v2(
        &self,
        swap_address: Address,
        swap_id: Token,
        contract_abi: &Contract,
        state_type: PaymentType,
    ) -> Result<U256, PaymentStatusErr> {
        let function_name = state_type.as_str();
        let function = contract_abi.function(function_name)?;
        let data = function.encode_input(&[swap_id])?;
        let bytes = self
            .call_request(self.my_addr().await, swap_address, None, Some(data.into()))
            .await?;
        let decoded_tokens = function.decode_output(&bytes.0)?;

        let state = decoded_tokens
            .get(2)
            .ok_or_else(|| PaymentStatusErr::Internal(ERRL!("Payment status must contain 'state' as the 2nd token")))?;
        match state {
            Token::Uint(state) => Ok(*state),
            _ => Err(PaymentStatusErr::Internal(ERRL!(
                "Payment status must be Uint, got {:?}",
                state
            ))),
        }
    }

    /// Prepares the encoded transaction data for spending a maker's NFT payment on the blockchain.
    ///
    /// This function selects the appropriate contract function based on the NFT's contract type (ERC1155 or ERC721)
    /// and encodes the input parameters required for the blockchain transaction.
    fn prepare_spend_nft_maker_v2_data(
        &self,
        args: &SpendNftMakerPaymentArgs<'_, Self>,
        decoded: Vec<Token>,
        htlc_params: Vec<Token>,
        state: U256,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        validate_payment_state(args.maker_payment_tx, state, MakerPaymentStateV2::PaymentSent as u8)?;

        let spend_func = match args.contract_type {
            ContractType::Erc1155 => NFT_MAKER_SWAP_V2.function("spendErc1155MakerPayment")?,
            ContractType::Erc721 => NFT_MAKER_SWAP_V2.function("spendErc721MakerPayment")?,
        };
        // Initialize tokens with common elements
        let mut input_tokens = vec![
            htlc_params[0].clone(), // swapId
            Token::Address(args.maker_payment_tx.sender()),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret.to_vec()),
            htlc_params[2].clone(), // tokenAddress
            decoded[2].clone(),     // tokenId
        ];
        // Add specific elements based on contract type
        if let ContractType::Erc1155 = args.contract_type {
            input_tokens.push(decoded[3].clone()); // amount
        }

        let data = spend_func.encode_input(&input_tokens)?;
        Ok(data)
    }

    fn prepare_refund_nft_maker_payment_v2_timelock(
        &self,
        args: &RefundNftMakerPaymentArgs<'_, Self>,
        decoded: Vec<Token>,
        htlc_params: Vec<Token>,
        state: U256,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        validate_payment_state(args.maker_payment_tx, state, MakerPaymentStateV2::PaymentSent as u8)?;

        let refund_func = match args.contract_type {
            ContractType::Erc1155 => NFT_MAKER_SWAP_V2.function("refundErc1155MakerPaymentTimelock")?,
            ContractType::Erc721 => NFT_MAKER_SWAP_V2.function("refundErc721MakerPaymentTimelock")?,
        };
        // Initialize tokens with common elements
        let mut input_tokens = vec![
            htlc_params[0].clone(), // swapId
            htlc_params[1].clone(), // takerAddress
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            htlc_params[2].clone(), // tokenAddress
            decoded[2].clone(),     // tokenId
        ];
        // Add specific elements based on contract type
        if let ContractType::Erc1155 = args.contract_type {
            input_tokens.push(decoded[3].clone()); // amount
        }

        let data = refund_func.encode_input(&input_tokens)?;
        Ok(data)
    }

    fn prepare_refund_nft_maker_payment_v2_secret(
        &self,
        args: &RefundNftMakerPaymentArgs<'_, Self>,
        decoded: Vec<Token>,
        htlc_params: Vec<Token>,
        state: U256,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        validate_payment_state(args.maker_payment_tx, state, MakerPaymentStateV2::PaymentSent as u8)?;

        let refund_func = match args.contract_type {
            ContractType::Erc1155 => NFT_MAKER_SWAP_V2.function("refundErc1155MakerPaymentSecret")?,
            ContractType::Erc721 => NFT_MAKER_SWAP_V2.function("refundErc721MakerPaymentSecret")?,
        };
        // Initialize tokens with common elements
        let mut input_tokens = vec![
            htlc_params[0].clone(), // swapId
            htlc_params[1].clone(), // takerAddress
            Token::FixedBytes(args.taker_secret.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            htlc_params[2].clone(), // tokenAddress
            decoded[2].clone(),     // tokenId
        ];
        // Add specific elements based on contract type
        if let ContractType::Erc1155 = args.contract_type {
            input_tokens.push(decoded[3].clone()); // amount
        }
        let data = refund_func.encode_input(&input_tokens)?;
        Ok(data)
    }

    async fn status_and_htlc_params_from_tx_data(
        &self,
        swap_address: Address,
        contract_abi: &Contract,
        decoded_data: &[Token],
        index: usize,
        state_type: PaymentType,
    ) -> Result<(U256, Vec<Token>), PaymentStatusErr> {
        let data_bytes = match decoded_data.get(index) {
            Some(Token::Bytes(data_bytes)) => data_bytes,
            _ => {
                return Err(PaymentStatusErr::TxDeserializationError(ERRL!(
                    "Failed to decode HTLCParams from data_bytes"
                )))
            },
        };

        let htlc_params = match ethabi::decode(htlc_params(), data_bytes) {
            Ok(htlc_params) => htlc_params,
            Err(_) => {
                return Err(PaymentStatusErr::TxDeserializationError(ERRL!(
                    "Failed to decode HTLCParams from data_bytes"
                )))
            },
        };

        let state = self
            .payment_status_v2(swap_address, htlc_params[0].clone(), contract_abi, state_type)
            .await?;

        Ok((state, htlc_params))
    }
}

/// Validates decoded data from tx input, related to `safeTransferFrom` contract call
fn validate_decoded_data(decoded: &[Token], params: &ValidationParams) -> Result<(), MmError<ValidatePaymentError>> {
    let checks = vec![
        (0, Token::Address(params.maker_address), "maker_address"),
        (1, Token::Address(params.etomic_swap_contract), "etomic_swap_contract"),
        (2, Token::Uint(U256::from(params.token_id)), "token_id"),
    ];

    for (index, expected_token, field_name) in checks {
        if decoded.get(index) != Some(&expected_token) {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                "NFT Maker Payment `{}` {:?} is invalid, expected {:?}",
                field_name,
                decoded.get(index),
                expected_token
            )));
        }
    }
    if let Some(amount) = &params.amount {
        let value = U256::from_dec_str(amount).map_to_mm(|e| ValidatePaymentError::InternalError(e.to_string()))?;
        if decoded[3] != Token::Uint(value) {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                "NFT Maker Payment `amount` {:?} is invalid, expected {:?}",
                decoded[3],
                Token::Uint(value)
            )));
        }
    }
    Ok(())
}

fn decode_and_validate_htlc_params(
    decoded: Vec<Token>,
    index: usize,
    expected_params: ExpectedHtlcParams,
) -> MmResult<(), HtlcParamsError> {
    let data_bytes = match decoded.get(index) {
        Some(Token::Bytes(bytes)) => bytes,
        _ => {
            return MmError::err(HtlcParamsError::TxDeserializationError(
                "Expected Bytes for HTLCParams data".to_string(),
            ))
        },
    };

    let decoded_params = match ethabi::decode(htlc_params(), data_bytes) {
        Ok(params) => params,
        Err(_) => {
            return MmError::err(HtlcParamsError::TxDeserializationError(
                "Failed to decode HTLCParams from data_bytes".to_string(),
            ))
        },
    };

    let expected_taker_secret_hash = Token::FixedBytes(expected_params.taker_secret_hash.clone());
    let expected_maker_secret_hash = Token::FixedBytes(expected_params.maker_secret_hash.clone());

    let checks = vec![
        (0, Token::FixedBytes(expected_params.swap_id.clone()), "swap_id"),
        (1, Token::Address(expected_params.taker_address), "taker_address"),
        (2, Token::Address(expected_params.token_address), "token_address"),
        (3, expected_taker_secret_hash, "taker_secret_hash"),
        (4, expected_maker_secret_hash, "maker_secret_hash"),
        (5, Token::Uint(expected_params.time_lock), "time_lock"),
    ];

    for (index, expected_token, param_name) in checks.into_iter() {
        if decoded_params[index] != expected_token {
            return MmError::err(HtlcParamsError::WrongPaymentTx(format!(
                "Invalid '{}' {:?}, expected {:?}",
                param_name, decoded_params[index], expected_token
            )));
        }
    }

    Ok(())
}

/// Representation of the Solidity HTLCParams struct.
///
/// struct HTLCParams {
///     bytes32 id;
///     address taker;
///     address tokenAddress;
///     bytes32 takerSecretHash;
///     bytes32 makerSecretHash;
///     uint32 paymentLockTime;
/// }
fn htlc_params() -> &'static [ethabi::ParamType] {
    &[
        ethabi::ParamType::FixedBytes(32),
        ethabi::ParamType::Address,
        ethabi::ParamType::Address,
        ethabi::ParamType::FixedBytes(32),
        ethabi::ParamType::FixedBytes(32),
        ethabi::ParamType::Uint(256),
    ]
}

/// function to check if BigDecimal is a positive integer
#[inline(always)]
fn is_positive_integer(amount: &BigDecimal) -> bool { amount == &amount.with_scale(0) && amount > &BigDecimal::from(0) }

fn validate_payment_args<'a>(
    taker_secret_hash: &'a [u8],
    maker_secret_hash: &'a [u8],
    amount: &BigDecimal,
    contract_type: &ContractType,
) -> Result<(), String> {
    match contract_type {
        ContractType::Erc1155 => {
            if !is_positive_integer(amount) {
                return Err("ERC-1155 amount must be a positive integer".to_string());
            }
        },
        ContractType::Erc721 => {
            if amount != &BigDecimal::from(1) {
                return Err("ERC-721 amount must be 1".to_string());
            }
        },
    }
    if taker_secret_hash.len() != 32 {
        return Err("taker_secret_hash must be 32 bytes".to_string());
    }
    if maker_secret_hash.len() != 32 {
        return Err("maker_secret_hash must be 32 bytes".to_string());
    }

    Ok(())
}

async fn validate_from_to_and_maker_status(
    tx_from_rpc: &Web3Tx,
    expected_from: Address,
    expected_to: Address,
    maker_status: U256,
) -> ValidatePaymentResult<()> {
    if maker_status != U256::from(MakerPaymentStateV2::PaymentSent as u8) {
        return MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
            "NFT Maker Payment state is not PAYMENT_STATE_SENT, got {}",
            maker_status
        )));
    }
    if tx_from_rpc.from != Some(expected_from) {
        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
            "NFT Maker Payment tx {:?} was sent from wrong address, expected {:?}",
            tx_from_rpc, expected_from
        )));
    }
    // As NFT owner calls "safeTransferFrom" directly, then in Transaction 'to' field we expect token_address
    if tx_from_rpc.to != Some(expected_to) {
        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
            "NFT Maker Payment tx {:?} was sent to wrong address, expected {:?}",
            tx_from_rpc, expected_to,
        )));
    }
    Ok(())
}

/// Identifies the correct `"safeTransferFrom"` function based on the contract type (either ERC1155 or ERC721)
/// and decodes the provided contract call bytes using the ABI of the identified function. Additionally, it returns
/// the index position of the "bytes" field within the function's parameters.
pub(crate) fn get_decoded_tx_data_and_bytes_index(
    contract_type: &ContractType,
    contract_call_bytes: &[u8],
) -> Result<(Vec<Token>, usize), PrepareTxDataError> {
    let (send_func, bytes_index) = match contract_type {
        ContractType::Erc1155 => (ERC1155_CONTRACT.function("safeTransferFrom")?, 4),
        ContractType::Erc721 => (erc721_transfer_with_data()?, 3),
    };
    let decoded = decode_contract_call(send_func, contract_call_bytes)?;
    Ok((decoded, bytes_index))
}

/// ERC721 contract has overloaded versions of the `safeTransferFrom` function,
/// but `Contract::function` method returns only the first if there are overloaded versions of the same function.
/// Provided function retrieves the `safeTransferFrom` variant that includes a `bytes` parameter.
/// This variant is specifically used for transferring ERC721 tokens with additional data.
fn erc721_transfer_with_data<'a>() -> Result<&'a ethabi::Function, Erc721FunctionError> {
    let functions = ERC721_CONTRACT
        .functions_by_name("safeTransferFrom")
        .map_err(|e| Erc721FunctionError::AbiError(ERRL!("{}", e)))?;

    // Find the correct function variant by inspecting the input parameters.
    let function = functions
        .iter()
        .find(|f| {
            f.inputs.len() == 4
                && matches!(
                    f.inputs.last().map(|input| &input.kind),
                    Some(&ethabi::ParamType::Bytes)
                )
        })
        .ok_or_else(|| {
            Erc721FunctionError::FunctionNotFound(
                "Failed to find the correct safeTransferFrom function variant".to_string(),
            )
        })?;
    Ok(function)
}

fn validate_payment_state(tx: &SignedEthTx, state: U256, expected_state: u8) -> Result<(), PrepareTxDataError> {
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

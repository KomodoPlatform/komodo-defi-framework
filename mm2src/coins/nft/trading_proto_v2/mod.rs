use crate::coin_errors::{ValidatePaymentError, ValidatePaymentResult};
use ethabi::{Contract, Token};
use ethcore_transaction::{Action, UnverifiedTransaction};
use ethereum_types::{Address, U256};
use futures::compat::Future01CompatExt;
use mm2_err_handle::prelude::{MapToMmResult, MmError, MmResult};
use mm2_number::BigDecimal;
use std::convert::TryInto;
use web3::types::TransactionId;

pub(crate) mod errors;
use errors::{Erc721FunctionError, HtlcParamsError, PaymentStatusErr};
pub(crate) mod structs;
use structs::{ExpectedHtlcParams, StateType, ValidationParams};

use super::ContractType;
use crate::eth::{addr_from_raw_pubkey, decode_contract_call, EthCoin, EthCoinType, MakerPaymentStateV2, SignedEthTx,
                 TryToAddress, ERC1155_CONTRACT, ERC721_CONTRACT, ETH_GAS, NFT_SWAP_CONTRACT};
use crate::nft::trading_proto_v2::errors::PrepareTxDataError;
use crate::{NftAssocTypes, RefundPaymentArgs, SendNftMakerPaymentArgs, SpendNftMakerPaymentArgs, TransactionErr,
            ValidateNftMakerPaymentArgs};

impl EthCoin {
    pub(crate) async fn send_nft_maker_payment_v2_impl(
        &self,
        args: SendNftMakerPaymentArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let contract_type = try_tx_s!(self.parse_contract_type(args.contract_type));
        try_tx_s!(self.validate_payment_args(
            args.taker_secret_hash,
            args.maker_secret_hash,
            &args.amount,
            &contract_type,
        ));

        let taker_address = try_tx_s!(addr_from_raw_pubkey(args.taker_pub));
        let token_address = try_tx_s!(self.parse_contract_address(args.token_address));
        let swap_contract_address = try_tx_s!(self.parse_contract_address(args.swap_contract_address));
        let time_lock_u32 = try_tx_s!(args.time_lock.try_into());
        let token_id_u256 = U256::from(args.token_id);
        let htlc_data = self.prepare_htlc_data(&args, taker_address, token_address, time_lock_u32);

        match &self.coin_type {
            EthCoinType::Nft { .. } => {
                let data = try_tx_s!(self.prepare_nft_maker_payment_v2_data(
                    contract_type,
                    swap_contract_address,
                    token_id_u256,
                    &args,
                    htlc_data
                ));
                self.sign_and_send_transaction(0.into(), Action::Call(token_address), data, U256::from(ETH_GAS))
                    .compat()
                    .await
            },
            EthCoinType::Eth | EthCoinType::Erc20 { .. } => Err(TransactionErr::ProtocolNotSupported(
                "ETH and ERC20 Protocols are not supported for NFT Swaps".to_string(),
            )),
        }
    }

    fn prepare_nft_maker_payment_v2_data(
        &self,
        contract_type: ContractType,
        swap_contract_address: Address,
        token_id: U256,
        args: &SendNftMakerPaymentArgs<'_, Self>,
        htlc_data: Vec<u8>,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        match contract_type {
            ContractType::Erc1155 => {
                let function = ERC1155_CONTRACT.function("safeTransferFrom")?;
                let amount_u256 = U256::from_dec_str(&args.amount.to_string())
                    .map_err(|e| PrepareTxDataError::Internal(e.to_string()))?;
                let data = function.encode_input(&[
                    Token::Address(self.my_address),
                    Token::Address(swap_contract_address),
                    Token::Uint(token_id),
                    Token::Uint(amount_u256),
                    Token::Bytes(htlc_data),
                ])?;
                Ok(data)
            },
            ContractType::Erc721 => {
                let function = self.erc721_transfer_with_data()?;
                let data = function.encode_input(&[
                    Token::Address(self.my_address),
                    Token::Address(swap_contract_address),
                    Token::Uint(token_id),
                    Token::Bytes(htlc_data),
                ])?;
                Ok(data)
            },
        }
    }

    fn validate_payment_args<'a>(
        &self,
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

    fn prepare_htlc_data(
        &self,
        args: &SendNftMakerPaymentArgs<'_, Self>,
        taker_address: Address,
        token_address: Address,
        time_lock: u32,
    ) -> Vec<u8> {
        let id = self.etomic_swap_id(time_lock, args.maker_secret_hash);
        ethabi::encode(&[
            Token::FixedBytes(id),
            Token::Address(taker_address),
            Token::Address(token_address),
            Token::FixedBytes(args.taker_secret_hash.to_vec()),
            Token::FixedBytes(args.maker_secret_hash.to_vec()),
            Token::Uint(U256::from(time_lock)),
        ])
    }

    /// ERC721 contract has overloaded versions of the `safeTransferFrom` function,
    /// but `Contract::function` method returns only the first if there are overloaded versions of the same function.
    /// Provided function retrieves the `safeTransferFrom` variant that includes a `bytes` parameter.
    /// This variant is specifically used for transferring ERC721 tokens with additional data.
    fn erc721_transfer_with_data(&self) -> Result<&ethabi::Function, Erc721FunctionError> {
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

    pub(crate) async fn validate_nft_maker_payment_v2_impl(
        &self,
        args: ValidateNftMakerPaymentArgs<'_, Self>,
    ) -> ValidatePaymentResult<()> {
        let contract_type = self.parse_contract_type(args.contract_type)?;
        self.validate_payment_args(
            args.taker_secret_hash,
            args.maker_secret_hash,
            &args.amount,
            &contract_type,
        )
        .map_err(ValidatePaymentError::InternalError)?;
        let etomic_swap_contract = self.parse_contract_address(args.swap_contract_address)?;
        let token_address = self.parse_contract_address(args.token_address)?;
        let maker_address = addr_from_raw_pubkey(args.maker_pub).map_to_mm(ValidatePaymentError::InternalError)?;
        let time_lock_u32 = args
            .time_lock
            .try_into()
            .map_err(ValidatePaymentError::TimelockOverflow)?;
        let swap_id = self.etomic_swap_id(time_lock_u32, args.maker_secret_hash);
        let maker_status = self
            .payment_status_v2(
                etomic_swap_contract,
                Token::FixedBytes(swap_id.clone()),
                &NFT_SWAP_CONTRACT,
                StateType::MakerPayments,
            )
            .await?;
        if maker_status != U256::from(MakerPaymentStateV2::PaymentSent as u8) {
            return MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                "NFT Maker Payment state is not PAYMENT_STATE_SENT, got {}",
                maker_status
            )));
        }
        let tx_from_rpc = self
            .transaction(TransactionId::Hash(args.maker_payment_tx.hash))
            .await?;
        let tx_from_rpc = tx_from_rpc.as_ref().ok_or_else(|| {
            ValidatePaymentError::TxDoesNotExist(format!(
                "Didn't find provided tx {:?} on ETH node",
                args.maker_payment_tx.hash
            ))
        })?;
        if tx_from_rpc.from != Some(maker_address) {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                "NFT Maker Payment tx {:?} was sent from wrong address, expected {:?}",
                tx_from_rpc, maker_address
            )));
        }
        // As NFT owner calls "safeTransferFrom" directly, then in Transaction 'to' field we expect token_address
        if tx_from_rpc.to != Some(token_address) {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                "NFT Maker Payment tx {:?} was sent to wrong address, expected {:?}",
                tx_from_rpc, token_address,
            )));
        }
        match self.coin_type {
            EthCoinType::Nft { .. } => match contract_type {
                ContractType::Erc1155 => {
                    let function = ERC1155_CONTRACT
                        .function("safeTransferFrom")
                        .map_to_mm(|e| ValidatePaymentError::InternalError(e.to_string()))?;
                    let decoded = decode_contract_call(function, &tx_from_rpc.input.0)
                        .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))?;

                    let validation_params = ValidationParams {
                        maker_address,
                        etomic_swap_contract,
                        token_id: args.token_id,
                        amount: Some(args.amount.to_string()),
                    };
                    validate_decoded_data(&decoded, &validation_params)?;

                    let taker_address =
                        addr_from_raw_pubkey(args.taker_pub).map_to_mm(ValidatePaymentError::InternalError)?;
                    let htlc_params = ExpectedHtlcParams {
                        swap_id,
                        taker_address,
                        token_address,
                        taker_secret_hash: args.taker_secret_hash.to_vec(),
                        maker_secret_hash: args.maker_secret_hash.to_vec(),
                        time_lock: U256::from(args.time_lock),
                    };
                    decode_and_validate_htlc_params(decoded, 4, htlc_params)?;
                },
                ContractType::Erc721 => {
                    let function = self.erc721_transfer_with_data()?;
                    let decoded = decode_contract_call(function, &tx_from_rpc.input.0)
                        .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))?;

                    let validation_params = ValidationParams {
                        maker_address,
                        etomic_swap_contract,
                        token_id: args.token_id,
                        amount: None,
                    };
                    validate_decoded_data(&decoded, &validation_params)?;

                    let taker_address =
                        addr_from_raw_pubkey(args.taker_pub).map_to_mm(ValidatePaymentError::InternalError)?;
                    let htlc_params = ExpectedHtlcParams {
                        swap_id,
                        taker_address,
                        token_address,
                        taker_secret_hash: args.taker_secret_hash.to_vec(),
                        maker_secret_hash: args.maker_secret_hash.to_vec(),
                        time_lock: U256::from(args.time_lock),
                    };
                    decode_and_validate_htlc_params(decoded, 3, htlc_params)?;
                },
            },
            EthCoinType::Eth | EthCoinType::Erc20 { .. } => {
                return MmError::err(ValidatePaymentError::InternalError(
                    "EthCoinType must be Nft".to_string(),
                ))
            },
        }
        Ok(())
    }

    async fn payment_status_v2(
        &self,
        swap_contract: Address,
        swap_id: Token,
        contract: &Contract,
        state_type: StateType,
    ) -> Result<U256, PaymentStatusErr> {
        let function_name = state_type.as_str();
        let function = contract.function(function_name)?;
        let data = function.encode_input(&[swap_id])?;
        let bytes = self.call_request(swap_contract, None, Some(data.into())).await?;
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

    pub(crate) async fn spend_nft_maker_payment_v2_impl(
        &self,
        args: SpendNftMakerPaymentArgs<'_, Self>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let contract_type = try_tx_s!(self.parse_contract_type(args.contract_type));
        let etomic_swap_contract = try_tx_s!(self.parse_contract_address(args.swap_contract_address));
        if args.maker_secret.len() != 32 {
            return Err(TransactionErr::Plain(ERRL!("maker_secret must be 32 bytes")));
        }

        let (send_func, index_bytes) = match contract_type {
            ContractType::Erc1155 => (try_tx_s!(ERC1155_CONTRACT.function("safeTransferFrom")), 4),
            ContractType::Erc721 => (try_tx_s!(self.erc721_transfer_with_data()), 3),
        };
        let decoded = try_tx_s!(decode_contract_call(send_func, &args.maker_payment_tx.data));
        let (state, htlc_params) = try_tx_s!(
            self.status_and_htlc_params_from_tx_data(
                etomic_swap_contract,
                &NFT_SWAP_CONTRACT,
                &decoded,
                index_bytes,
                StateType::MakerPayments,
            )
            .await
        );

        match self.coin_type {
            EthCoinType::Nft { .. } => {
                let data =
                    try_tx_s!(self.prepare_spend_nft_maker_v2_data(contract_type, &args, decoded, htlc_params, state));

                self.sign_and_send_transaction(0.into(), Action::Call(etomic_swap_contract), data, U256::from(ETH_GAS))
                    .compat()
                    .await
            },
            EthCoinType::Eth | EthCoinType::Erc20 { .. } => Err(TransactionErr::ProtocolNotSupported(
                "ETH and ERC20 Protocols are not supported for NFT Swaps".to_string(),
            )),
        }
    }

    fn prepare_spend_nft_maker_v2_data(
        &self,
        contract_type: ContractType,
        args: &SpendNftMakerPaymentArgs<'_, Self>,
        decoded: Vec<Token>,
        htlc_params: Vec<Token>,
        state: U256,
    ) -> Result<Vec<u8>, PrepareTxDataError> {
        let spend_func = match contract_type {
            ContractType::Erc1155 => NFT_SWAP_CONTRACT.function("spendErc1155MakerPayment")?,
            ContractType::Erc721 => NFT_SWAP_CONTRACT.function("spendErc721MakerPayment")?,
        };

        if state != U256::from(MakerPaymentStateV2::PaymentSent as u8) {
            return Err(PrepareTxDataError::Internal(ERRL!(
                "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                args.maker_payment_tx,
                state
            )));
        }

        let input_tokens = match contract_type {
            ContractType::Erc1155 => vec![
                htlc_params[0].clone(), // swap_id
                Token::Address(args.maker_payment_tx.sender()),
                Token::FixedBytes(args.taker_secret_hash.to_vec()),
                Token::FixedBytes(args.maker_secret.to_vec()),
                htlc_params[2].clone(), // tokenAddress
                decoded[2].clone(),     // tokenId
                decoded[3].clone(),     // amount
            ],
            ContractType::Erc721 => vec![
                htlc_params[0].clone(), // swap_id
                Token::Address(args.maker_payment_tx.sender()),
                Token::FixedBytes(args.taker_secret_hash.to_vec()),
                Token::FixedBytes(args.maker_secret.to_vec()),
                htlc_params[2].clone(), // tokenAddress
                decoded[2].clone(),     // tokenId
            ],
        };

        let data = spend_func.encode_input(&input_tokens)?;
        Ok(data)
    }

    async fn status_and_htlc_params_from_tx_data(
        &self,
        swap_contract: Address,
        contract: &Contract,
        decoded_data: &[Token],
        index: usize,
        state_type: StateType,
    ) -> Result<(U256, Vec<Token>), PaymentStatusErr> {
        if let Some(Token::Bytes(data_bytes)) = decoded_data.get(index) {
            if let Ok(htlc_params) = ethabi::decode(htlc_params(), data_bytes) {
                let state = self
                    .payment_status_v2(swap_contract, htlc_params[0].clone(), contract, state_type)
                    .await?;
                Ok((state, htlc_params))
            } else {
                Err(PaymentStatusErr::TxDeserializationError(ERRL!(
                    "Failed to decode HTLCParams from data_bytes"
                )))
            }
        } else {
            Err(PaymentStatusErr::TxDeserializationError(ERRL!(
                "Failed to decode HTLCParams from data_bytes"
            )))
        }
    }

    pub(crate) async fn refund_nft_maker_payment_v2_timelock_impl(
        &self,
        args: RefundPaymentArgs<'_>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let _etomic_swap_contract = try_tx_s!(args.swap_contract_address.try_to_address());
        let tx: UnverifiedTransaction = try_tx_s!(rlp::decode(args.payment_tx));
        let _payment = try_tx_s!(SignedEthTx::new(tx));
        todo!()
    }
}

/// Validates decoded data from tx input, related to `safeTransferFrom` contract call
fn validate_decoded_data(decoded: &[Token], params: &ValidationParams) -> Result<(), MmError<ValidatePaymentError>> {
    if decoded[0] != Token::Address(params.maker_address) {
        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
            "NFT Maker Payment `maker_address` {:?} is invalid, expected {:?}",
            decoded[0],
            Token::Address(params.maker_address)
        )));
    }
    if decoded[1] != Token::Address(params.etomic_swap_contract) {
        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
            "NFT Maker Payment `etomic_swap_contract` {:?} is invalid, expected address {:?}",
            decoded[1],
            Token::Address(params.etomic_swap_contract)
        )));
    }
    let token_id = U256::from(params.token_id);
    if decoded[2] != Token::Uint(token_id) {
        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
            "NFT Maker Payment `token_id` {:?} is invalid, expected {:?}",
            decoded[2],
            Token::Uint(token_id)
        )));
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
    if let Some(Token::Bytes(data_bytes)) = decoded.get(index) {
        if let Ok(decoded_params) = ethabi::decode(htlc_params(), data_bytes) {
            if decoded_params[0] != Token::FixedBytes(expected_params.swap_id.clone()) {
                return MmError::err(HtlcParamsError::WrongPaymentTx(format!(
                    "Invalid 'swap_id' {:?}, expected {:?}",
                    decoded_params[0],
                    Token::FixedBytes(expected_params.swap_id)
                )));
            }
            if decoded_params[1] != Token::Address(expected_params.taker_address) {
                return MmError::err(HtlcParamsError::WrongPaymentTx(format!(
                    "Invalid `taker_address` {:?}, expected {:?}",
                    decoded_params[1],
                    Token::Address(expected_params.taker_address)
                )));
            }
            if decoded_params[2] != Token::Address(expected_params.token_address) {
                return MmError::err(HtlcParamsError::WrongPaymentTx(format!(
                    "Invalid `token_address` {:?}, expected {:?}",
                    decoded_params[2],
                    Token::Address(expected_params.token_address)
                )));
            }
            if decoded_params[3] != Token::FixedBytes(expected_params.taker_secret_hash.clone()) {
                return MmError::err(HtlcParamsError::WrongPaymentTx(format!(
                    "Invalid 'taker_secret_hash' {:?}, expected {:?}",
                    decoded_params[3],
                    Token::FixedBytes(expected_params.taker_secret_hash)
                )));
            }
            if decoded_params[4] != Token::FixedBytes(expected_params.maker_secret_hash.clone()) {
                return MmError::err(HtlcParamsError::WrongPaymentTx(format!(
                    "Invalid 'maker_secret_hash' {:?}, expected {:?}",
                    decoded_params[4],
                    Token::FixedBytes(expected_params.maker_secret_hash)
                )));
            }
            if decoded_params[5] != Token::Uint(expected_params.time_lock) {
                return MmError::err(HtlcParamsError::WrongPaymentTx(format!(
                    "Invalid 'time_lock' {:?}, expected {:?}",
                    decoded_params[5],
                    Token::Uint(expected_params.time_lock)
                )));
            }
        } else {
            return MmError::err(HtlcParamsError::TxDeserializationError(
                "Failed to decode HTLCParams from data_bytes".to_string(),
            ));
        }
    } else {
        return MmError::err(HtlcParamsError::TxDeserializationError(
            "Expected Bytes for HTLCParams data".to_string(),
        ));
    }
    Ok(())
}

// Representation of the Solidity HTLCParams struct.
//
// struct HTLCParams {
//     bytes32 id;
//     address taker;
//     address tokenAddress;
//     bytes32 takerSecretHash;
//     bytes32 makerSecretHash;
//     uint32 paymentLockTime;
// }
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

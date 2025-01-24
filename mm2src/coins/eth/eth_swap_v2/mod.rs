use crate::eth::{EthCoin, EthCoinType, ParseCoinAssocTypes, Transaction, TransactionErr};
use enum_derives::EnumFromStringify;
use ethabi::{Contract, Token};
use ethcore_transaction::SignedTransaction as SignedEthTx;
use ethereum_types::{Address, U256};
use futures::compat::Future01CompatExt;
use mm2_err_handle::mm_error::MmError;
use mm2_number::BigDecimal;
use num_traits::Signed;
use web3::types::Transaction as Web3Tx;

pub(crate) mod eth_maker_swap_v2;
pub(crate) mod eth_taker_swap_v2;

/// ZERO_VALUE is used to represent a 0 amount in transactions where the value is encoded in the transaction input data.
/// This is typically used in function calls where the value is not directly transferred with the transaction, such as in
/// `spendTakerPayment` where the [amount](https://github.com/KomodoPlatform/etomic-swap/blob/5e15641cbf41766cd5b37b4d71842c270773f788/contracts/EtomicSwapTakerV2.sol#L166)
/// is provided as part of the input data rather than as an Ether value
pub(crate) const ZERO_VALUE: u32 = 0;

pub enum EthPaymentType {
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

pub enum PaymentMethod {
    Send,
    Spend,
    RefundTimelock,
    RefundSecret,
}

#[derive(Debug, Display)]
pub(crate) enum ValidatePaymentV2Err {
    UnexpectedPaymentState(String),
    WrongPaymentTx(String),
}

#[derive(Debug, Display, EnumFromStringify)]
pub(crate) enum PaymentStatusErr {
    #[from_stringify("ethabi::Error")]
    #[display(fmt = "ABI error: {}", _0)]
    ABIError(String),
    #[from_stringify("web3::Error")]
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
    #[display(fmt = "Invalid data error: {}", _0)]
    InvalidData(String),
}

#[derive(Debug, Display, EnumFromStringify)]
pub(crate) enum PrepareTxDataError {
    #[from_stringify("ethabi::Error")]
    #[display(fmt = "ABI error: {}", _0)]
    ABIError(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl EthCoin {
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
            PaymentStatusErr::Internal(format!(
                "Payment status must contain 'state' as the {} token",
                state_index
            ))
        })?;
        match state {
            Token::Uint(state) => Ok(*state),
            _ => Err(PaymentStatusErr::InvalidData(format!(
                "Payment status must be Uint, got {:?}",
                state
            ))),
        }
    }

    pub(super) fn get_token_address(&self) -> Result<Address, String> {
        match &self.coin_type {
            EthCoinType::Eth => Ok(Address::default()),
            EthCoinType::Erc20 { token_addr, .. } => Ok(*token_addr),
            EthCoinType::Nft { .. } => Err("NFT protocol is not supported for ETH and ERC20 Swaps".to_string()),
        }
    }
}

pub(crate) fn validate_payment_state(
    tx: &SignedEthTx,
    state: U256,
    expected_state: u8,
) -> Result<(), PrepareTxDataError> {
    if state != U256::from(expected_state) {
        return Err(PrepareTxDataError::Internal(format!(
            "Payment {:?} state is not `{}`, got `{}`",
            tx, expected_state, state
        )));
    }
    Ok(())
}

pub(crate) fn validate_from_to_and_status(
    tx_from_rpc: &Web3Tx,
    expected_from: Address,
    expected_to: Address,
    status: U256,
    expected_status: u8,
) -> Result<(), MmError<ValidatePaymentV2Err>> {
    if status != U256::from(expected_status) {
        return MmError::err(ValidatePaymentV2Err::UnexpectedPaymentState(format!(
            "Payment state is not `PaymentSent`, got {}",
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

// TODO validate premium when add its support in swap_v2
fn validate_amount(trading_amount: &BigDecimal) -> Result<(), String> {
    if !trading_amount.is_positive() {
        return Err("trading_amount must be a positive value".to_string());
    }
    Ok(())
}

fn check_decoded_length(decoded: &Vec<Token>, expected_len: usize) -> Result<(), PrepareTxDataError> {
    if decoded.len() != expected_len {
        return Err(PrepareTxDataError::Internal(format!(
            "Invalid number of tokens in decoded. Expected {}, found {}",
            expected_len,
            decoded.len()
        )));
    }
    Ok(())
}

impl EthCoin {
    async fn handle_allowance(
        &self,
        swap_contract: Address,
        payment_amount: U256,
        time_lock: u64,
    ) -> Result<(), TransactionErr> {
        let allowed = self
            .allowance(swap_contract)
            .compat()
            .await
            .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;

        if allowed < payment_amount {
            let approved_tx = self.approve(swap_contract, U256::max_value()).compat().await?;
            self.wait_for_required_allowance(swap_contract, payment_amount, time_lock)
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
        Ok(())
    }
}

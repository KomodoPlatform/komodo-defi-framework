use std::str::FromStr;

use base58::FromBase58;
use derive_more::Display;
use futures::compat::Future01CompatExt;
use mm2_err_handle::prelude::*;
use mm2_number::bigdecimal::{BigDecimal, ToPrimitive};
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::signature::{Signature, Signer};

use crate::solana::SolanaCommonOps;
use crate::{BalanceError, MarketCoinOps, NumConversError, SignatureError, SignatureResult, SolanaCoin,
            UnexpectedDerivationMethod, VerificationError, VerificationResult, WithdrawError};

#[derive(Debug, Display)]
pub enum SufficientBalanceError {
    #[display(
        fmt = "Not enough {} to withdraw: available {}, required at least {}",
        coin,
        available,
        required
    )]
    NotSufficientBalance {
        coin: String,
        available: BigDecimal,
        required: BigDecimal,
    },
    #[display(fmt = "The amount {} is too small, required at least {}", amount, threshold)]
    AmountTooLow { amount: BigDecimal, threshold: BigDecimal },
    #[display(fmt = "{}", _0)]
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    #[display(fmt = "Wallet storage error: {}", _0)]
    WalletStorageError(String),
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
}

impl From<NumConversError> for SufficientBalanceError {
    fn from(e: NumConversError) -> Self { SufficientBalanceError::Internal(e.to_string()) }
}

impl From<BalanceError> for SufficientBalanceError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(e) => SufficientBalanceError::Transport(e),
            BalanceError::InvalidResponse(e) => SufficientBalanceError::InvalidResponse(e),
            BalanceError::UnexpectedDerivationMethod(e) => SufficientBalanceError::UnexpectedDerivationMethod(e),
            BalanceError::Internal(e) => SufficientBalanceError::Internal(e),
            BalanceError::WalletStorageError(e) => SufficientBalanceError::WalletStorageError(e),
        }
    }
}
impl From<SufficientBalanceError> for WithdrawError {
    fn from(e: SufficientBalanceError) -> Self {
        match e {
            SufficientBalanceError::NotSufficientBalance {
                coin,
                available,
                required,
            } => WithdrawError::NotSufficientBalance {
                coin,
                available,
                required,
            },
            SufficientBalanceError::UnexpectedDerivationMethod(e) => WithdrawError::from(e),
            SufficientBalanceError::InvalidResponse(e) | SufficientBalanceError::Transport(e) => {
                WithdrawError::Transport(e)
            },
            SufficientBalanceError::Internal(e) | SufficientBalanceError::WalletStorageError(e) => {
                WithdrawError::InternalError(e)
            },
            SufficientBalanceError::AmountTooLow { amount, threshold } => {
                WithdrawError::AmountTooLow { amount, threshold }
            },
        }
    }
}

pub struct PrepareTransferData {
    pub to_send: BigDecimal,
    pub my_balance: BigDecimal,
    pub sol_required: BigDecimal,
    pub lamports_to_send: u64,
}

pub fn lamports_to_sol(lamports: u64) -> BigDecimal { BigDecimal::from(lamports) / BigDecimal::from(LAMPORTS_PER_SOL) }

pub fn sol_to_lamports(sol: &BigDecimal) -> Result<u64, MmError<NumConversError>> {
    let maybe_lamports = (sol * BigDecimal::from(LAMPORTS_PER_SOL)).to_u64();
    match maybe_lamports {
        None => MmError::err(NumConversError("Error when converting sol to lamports".to_string())),
        Some(lamports) => Ok(lamports),
    }
}

pub fn ui_amount_to_amount(ui_amount: BigDecimal, decimals: u8) -> Result<u64, MmError<NumConversError>> {
    let maybe_amount = (ui_amount * BigDecimal::from(10_u64.pow(decimals as u32))).to_u64();
    match maybe_amount {
        None => MmError::err(NumConversError("Error when converting ui amount to amount".to_string())),
        Some(amount) => Ok(amount),
    }
}

pub fn amount_to_ui_amount(amount: u64, decimals: u8) -> BigDecimal {
    BigDecimal::from(amount) / BigDecimal::from(10_u64.pow(decimals as u32))
}

pub fn sign_message(coin: &SolanaCoin, message: &str) -> SignatureResult<String> {
    let signature = coin
        .key_pair
        .try_sign_message(message.as_bytes())
        .map_err(|e| SignatureError::InternalError(e.to_string()))?;
    Ok(signature.to_string())
}

pub fn verify_message(
    coin: &SolanaCoin,
    signature: &str,
    message: &str,
    pubkey_bs58: &str,
) -> VerificationResult<bool> {
    let pubkey = pubkey_bs58.from_base58()?;
    let signature =
        Signature::from_str(signature).map_err(|e| VerificationError::SignatureDecodingError(e.to_string()))?;
    let is_valid = signature.verify(&pubkey, message.as_bytes());
    Ok(is_valid)
}

pub async fn check_balance_and_prepare_transfer<T>(
    coin: &T,
    max: bool,
    amount: BigDecimal,
    fees: u64,
) -> Result<PrepareTransferData, MmError<SufficientBalanceError>>
where
    T: SolanaCommonOps + MarketCoinOps,
{
    let base_balance = coin.base_coin_balance().compat().await?;
    let sol_required = lamports_to_sol(fees);
    if base_balance < sol_required {
        return MmError::err(SufficientBalanceError::NotSufficientBalance {
            coin: coin.platform_ticker().to_string(),
            available: base_balance.clone(),
            required: sol_required.clone(),
        });
    }

    let my_balance = coin.my_balance().compat().await?.spendable;
    let to_send = if max { my_balance.clone() } else { amount.clone() };
    let to_check = if max || coin.is_token() {
        to_send.clone()
    } else {
        &to_send + &sol_required
    };
    if to_check > my_balance {
        return MmError::err(SufficientBalanceError::NotSufficientBalance {
            coin: coin.ticker().to_string(),
            available: my_balance,
            required: to_check,
        });
    }

    let lamports_to_send = if !coin.is_token() {
        if max {
            sol_to_lamports(&my_balance)? - sol_to_lamports(&sol_required)?
        } else {
            sol_to_lamports(&amount)?
        }
    } else {
        0_u64
    };

    Ok(PrepareTransferData {
        to_send,
        my_balance,
        sol_required,
        lamports_to_send,
    })
}

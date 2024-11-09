//! This source file is for RPCs specific for EVM platform

use coins::eth::{u256_to_big_decimal, wei_from_big_decimal, EthCoin, Web3RpcError};
use coins::{lp_coinfind_or_err, MmCoin, MmCoinEnum, NumConversError, Transaction, TransactionErr};
use common::HttpStatusCode;
use derive_more::Display;
use enum_derives::EnumFromStringify;
use ethereum_types::Address as EthAddress;
use futures::compat::Future01CompatExt;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::{mm_error::MmError, prelude::MmResult};
use mm2_number::BigDecimal;

#[derive(Debug, Deserialize, Display, EnumFromStringify, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum Erc20CallError {
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "Coin not supported {}", coin)]
    CoinNotSupported { coin: String },
    #[from_stringify("NumConversError")]
    #[display(fmt = "Invalid param: {}", _0)]
    InvalidParam(String),
    #[from_stringify("TransactionErr")]
    #[display(fmt = "Transaction error {}", _0)]
    TransactionError(String),
    #[from_stringify("Web3RpcError")]
    #[display(fmt = "Web3 RPC error {}", _0)]
    Web3RpcError(String),
}

impl HttpStatusCode for Erc20CallError {
    fn status_code(&self) -> StatusCode {
        match self {
            Erc20CallError::NoSuchCoin { .. }
            | Erc20CallError::CoinNotSupported { .. }
            | Erc20CallError::InvalidParam(_) => StatusCode::BAD_REQUEST,
            Erc20CallError::TransactionError(_) | Erc20CallError::Web3RpcError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Erc20AllowanceRequest {
    coin: String,
    spender: EthAddress,
}

/// Call allowance method for ERC20 tokens (see https://eips.ethereum.org/EIPS/eip-20#approve).
/// Returns BigDecimal allowance value.
pub async fn get_token_allowance_rpc(ctx: MmArc, req: Erc20AllowanceRequest) -> MmResult<BigDecimal, Erc20CallError> {
    let eth_coin = find_erc20_eth_coin(&ctx, &req.coin).await?;
    let wei = eth_coin.allowance(req.spender).compat().await?;
    let amount = u256_to_big_decimal(wei, eth_coin.decimals())?;
    Ok(amount)
}

#[derive(Debug, Deserialize)]
pub struct Erc20ApproveRequest {
    coin: String,
    spender: EthAddress,
    amount: BigDecimal,
}

/// Call approve method for ERC20 tokens (see https://eips.ethereum.org/EIPS/eip-20#allowance).
/// Returns approval transaction hash.
pub async fn approve_token_rpc(ctx: MmArc, req: Erc20ApproveRequest) -> MmResult<String, Erc20CallError> {
    let eth_coin = find_erc20_eth_coin(&ctx, &req.coin).await?;
    let amount = wei_from_big_decimal(&req.amount, eth_coin.decimals())?;
    let tx = eth_coin.approve(req.spender, amount).compat().await?;
    Ok(format!("0x{:02x}", tx.tx_hash_as_bytes()))
}

async fn find_erc20_eth_coin(ctx: &MmArc, coin: &str) -> Result<EthCoin, MmError<Erc20CallError>> {
    match lp_coinfind_or_err(ctx, coin).await {
        Ok(MmCoinEnum::EthCoin(eth_coin)) => Ok(eth_coin),
        Ok(_) => Err(MmError::new(Erc20CallError::CoinNotSupported {
            coin: coin.to_string(),
        })),
        Err(_) => Err(MmError::new(Erc20CallError::NoSuchCoin { coin: coin.to_string() })),
    }
}

use std::fmt;

use async_trait::async_trait;
use common::PagingOptionsEnum;
use crypto::{Bip44Chain, RpcDerivationPath};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;

use crate::coin_balance::HDAddressBalance;
use crate::hd_wallet::HDWalletCoinOps;
use crate::rpc_command::hd_account_balance_rpc_error::HDAccountBalanceRpcError;
use crate::{lp_coinfind_or_err, CoinBalance, CoinWithDerivationMethod, MmCoinEnum};

#[derive(Deserialize)]
pub struct HDAccountBalanceRequest {
    coin: String,
    #[serde(flatten)]
    params: AccountBalanceParams,
}

#[derive(Deserialize)]
pub struct AccountBalanceParams {
    pub account_index: u32,
    pub chain: Bip44Chain,
    #[serde(default = "common::ten")]
    pub limit: usize,
    #[serde(default)]
    pub paging_options: PagingOptionsEnum<u32>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct HDAccountBalanceResponse {
    pub account_index: u32,
    pub derivation_path: RpcDerivationPath,
    pub addresses: Vec<HDAddressBalance>,
    pub page_balance: CoinBalance,
    pub limit: usize,
    pub skipped: u32,
    pub total: u32,
    pub total_pages: usize,
    pub paging_options: PagingOptionsEnum<u32>,
}

#[async_trait]
pub trait AccountBalanceRpcOps {
    async fn account_balance_rpc(
        &self,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError>;
}

pub async fn account_balance(
    ctx: MmArc,
    req: HDAccountBalanceRequest,
) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError> {
    match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::UtxoCoin(utxo) => utxo.account_balance_rpc(req.params).await,
        MmCoinEnum::QtumCoin(qtum) => qtum.account_balance_rpc(req.params).await,
        _ => MmError::err(HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet),
    }
}

pub mod common_impl {
    use common::calc_total_pages;

    use super::*;
    use crate::coin_balance::HDWalletBalanceOps;
    use crate::hd_wallet::{HDAccountOps, HDWalletOps};

    pub async fn account_balance_rpc<Coin>(
        coin: &Coin,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError>
    where
        Coin: HDWalletBalanceOps + CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet> + Sync,
        <Coin as HDWalletCoinOps>::Address: fmt::Display + Clone,
    {
        let account_id = params.account_index;
        let hd_account = coin
            .derivation_method()
            .hd_wallet_or_err()?
            .get_account(account_id)
            .await
            .or_mm_err(|| HDAccountBalanceRpcError::UnknownAccount { account_id })?;
        let total_addresses_number = hd_account.known_addresses_number(params.chain)?;

        let from_address_id = match params.paging_options {
            PagingOptionsEnum::FromId(from_address_id) => from_address_id + 1,
            PagingOptionsEnum::PageNumber(page_number) => ((page_number.get() - 1) * params.limit) as u32,
        };
        let to_address_id = std::cmp::min(from_address_id + params.limit as u32, total_addresses_number);

        let addresses = coin
            .known_addresses_balances_with_ids(&hd_account, params.chain, from_address_id..to_address_id)
            .await?;
        let page_balance = addresses.iter().fold(CoinBalance::default(), |total, addr_balance| {
            total + addr_balance.balance.clone()
        });

        let result = HDAccountBalanceResponse {
            account_index: account_id,
            derivation_path: RpcDerivationPath(hd_account.account_derivation_path()),
            addresses,
            page_balance,
            limit: params.limit,
            skipped: std::cmp::min(from_address_id, total_addresses_number),
            total: total_addresses_number,
            total_pages: calc_total_pages(total_addresses_number as usize, params.limit),
            paging_options: params.paging_options,
        };

        Ok(result)
    }
}

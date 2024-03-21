use crate::coin_balance::HDAddressBalance;
use crate::rpc_command::hd_account_balance_rpc_error::HDAccountBalanceRpcError;
use crate::{lp_coinfind_or_err, CoinBalance, CoinBalanceMap, CoinWithDerivationMethod, MmCoinEnum};
use async_trait::async_trait;
use common::PagingOptionsEnum;
use crypto::{Bip44Chain, RpcDerivationPath};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use std::fmt;

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
pub struct HDAccountBalanceResponse<BalanceMap> {
    pub account_index: u32,
    pub derivation_path: RpcDerivationPath,
    pub addresses: Vec<HDAddressBalance<BalanceMap>>,
    // Todo: Add option to get total balance of all addresses in addition to page_balance
    pub page_balance: BalanceMap,
    pub limit: usize,
    pub skipped: u32,
    pub total: u32,
    pub total_pages: usize,
    pub paging_options: PagingOptionsEnum<u32>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum HDAccountBalanceResponseEnum {
    Single(HDAccountBalanceResponse<CoinBalance>),
    Map(HDAccountBalanceResponse<CoinBalanceMap>),
}

#[async_trait]
pub trait AccountBalanceRpcOps {
    type BalanceMap;

    async fn account_balance_rpc(
        &self,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse<Self::BalanceMap>, HDAccountBalanceRpcError>;
}

pub async fn account_balance(
    ctx: MmArc,
    req: HDAccountBalanceRequest,
) -> MmResult<HDAccountBalanceResponseEnum, HDAccountBalanceRpcError> {
    match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::UtxoCoin(utxo) => Ok(HDAccountBalanceResponseEnum::Single(
            utxo.account_balance_rpc(req.params).await?,
        )),
        MmCoinEnum::QtumCoin(qtum) => Ok(HDAccountBalanceResponseEnum::Single(
            qtum.account_balance_rpc(req.params).await?,
        )),
        MmCoinEnum::EthCoin(eth) => Ok(HDAccountBalanceResponseEnum::Map(
            eth.account_balance_rpc(req.params).await?,
        )),
        _ => MmError::err(HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet),
    }
}

pub mod common_impl {
    use super::*;
    use crate::coin_balance::{BalanceMapOps, HDWalletBalanceMap, HDWalletBalanceOps};
    use crate::hd_wallet::{HDAccountOps, HDCoinAddress, HDWalletOps};
    use common::calc_total_pages;

    pub async fn account_balance_rpc<Coin>(
        coin: &Coin,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse<HDWalletBalanceMap<Coin>>, HDAccountBalanceRpcError>
    where
        Coin: HDWalletBalanceOps + CoinWithDerivationMethod + Sync,
        HDCoinAddress<Coin>: fmt::Display + Clone,
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

        let page_balance = addresses
            .iter()
            .fold(HDWalletBalanceMap::<Coin>::new(), |mut total, addr_balance| {
                total.add(addr_balance.balance.clone());
                total
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

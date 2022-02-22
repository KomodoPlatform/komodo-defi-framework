use crate::hd_wallet::{AddressDerivingError, HDWalletCoinOps, InvalidBip44ChainError};
use crate::{lp_coinfind_or_err, BalanceError, BalanceResult, CoinBalance, CoinFindError, CoinWithDerivationMethod,
            DerivationMethod, MarketCoinOps, MmCoinEnum};
use async_trait::async_trait;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{HttpStatusCode, PagingOptionsEnum};
use crypto::{Bip44Chain, RpcDerivationPath};
use derive_more::Display;
use futures::compat::Future01CompatExt;
use http::StatusCode;
use std::fmt;
use std::ops::Range;

pub type AddressIdRange = Range<u32>;

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum HDAccountBalanceRpcError {
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(
        fmt = "'{}' coin is expected to be activated with the HD wallet derivation method",
        coin
    )]
    CoinIsActivatedNotWithHDWallet { coin: String },
    #[display(fmt = "HD account '{}' is not activated", account_id)]
    UnknownAccount { account_id: u32 },
    #[display(fmt = "Coin doesn't support the given BIP44 chain: {:?}", chain)]
    InvalidBip44Chain { chain: Bip44Chain },
    #[display(fmt = "Error deriving an address: {}", _0)]
    ErrorDerivingAddress(String),
    #[display(fmt = "Electrum/Native RPC invalid response: {}", _0)]
    RpcInvalidResponse(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
}

impl HttpStatusCode for HDAccountBalanceRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            HDAccountBalanceRpcError::NoSuchCoin { .. }
            | HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet { .. }
            | HDAccountBalanceRpcError::UnknownAccount { .. }
            | HDAccountBalanceRpcError::InvalidBip44Chain { .. }
            | HDAccountBalanceRpcError::ErrorDerivingAddress(_) => StatusCode::BAD_REQUEST,
            HDAccountBalanceRpcError::Transport(_)
            | HDAccountBalanceRpcError::RpcInvalidResponse(_)
            | HDAccountBalanceRpcError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for HDAccountBalanceRpcError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => HDAccountBalanceRpcError::NoSuchCoin { coin },
        }
    }
}

impl From<BalanceError> for HDAccountBalanceRpcError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(transport) => HDAccountBalanceRpcError::Transport(transport),
            BalanceError::InvalidResponse(rpc) => HDAccountBalanceRpcError::RpcInvalidResponse(rpc),
            // `wallet_balance` should work with both [`DerivationMethod::Iguana`] and [`DerivationMethod::HDWallet`] correctly.
            BalanceError::DerivationMethodNotSupported(error) => HDAccountBalanceRpcError::Internal(error.to_string()),
            BalanceError::Internal(internal) => HDAccountBalanceRpcError::Internal(internal),
        }
    }
}

impl From<InvalidBip44ChainError> for HDAccountBalanceRpcError {
    fn from(e: InvalidBip44ChainError) -> Self { HDAccountBalanceRpcError::InvalidBip44Chain { chain: e.chain } }
}

impl From<AddressDerivingError> for HDAccountBalanceRpcError {
    fn from(e: AddressDerivingError) -> Self {
        match e {
            AddressDerivingError::Bip32Error(bip32) => {
                HDAccountBalanceRpcError::ErrorDerivingAddress(bip32.to_string())
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "wallet_type")]
pub enum EnableCoinBalance {
    Iguana(IguanaWalletBalance),
    HD(HDWalletBalance),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IguanaWalletBalance {
    pub address: String,
    pub balance: CoinBalance,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HDWalletBalance {
    pub accounts: Vec<HDAccountBalance>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HDAccountBalance {
    pub account_index: u32,
    pub derivation_path: RpcDerivationPath,
    pub total_balance: CoinBalance,
    pub addresses: Vec<HDAddressBalance>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HDAddressBalance {
    pub address: String,
    pub derivation_path: RpcDerivationPath,
    pub chain: Bip44Chain,
    pub balance: CoinBalance,
}

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

#[derive(Deserialize)]
pub struct CheckHDAccountBalanceRequest {
    coin: String,
    #[serde(flatten)]
    params: CheckHDAccountBalanceParams,
}

#[derive(Deserialize)]
pub struct CheckHDAccountBalanceParams {
    pub account_index: u32,
    pub gap_limit: Option<u32>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct HDAccountBalanceResponse {
    pub account_index: u32,
    pub derivation_path: RpcDerivationPath,
    pub addresses: Vec<HDAddressBalance>,
    pub limit: usize,
    pub skipped: u32,
    pub total: u32,
    pub total_pages: usize,
    pub paging_options: PagingOptionsEnum<u32>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct CheckHDAccountBalanceResponse {
    pub account_index: u32,
    pub derivation_path: RpcDerivationPath,
    pub new_addresses: Vec<HDAddressBalance>,
}

#[async_trait]
pub trait EnableCoinBalanceOps {
    async fn enable_coin_balance(&self) -> BalanceResult<EnableCoinBalance>;
}

#[async_trait]
impl<Coin> EnableCoinBalanceOps for Coin
where
    Coin: CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet>
        + HDWalletBalanceOps
        + MarketCoinOps
        + Sync,
    <Coin as CoinWithDerivationMethod>::Address: fmt::Display + Sync,
{
    async fn enable_coin_balance(&self) -> BalanceResult<EnableCoinBalance> {
        match self.derivation_method() {
            DerivationMethod::Iguana(my_address) => self.my_balance().compat().await.map(|balance| {
                EnableCoinBalance::Iguana(IguanaWalletBalance {
                    address: my_address.to_string(),
                    balance,
                })
            }),
            DerivationMethod::HDWallet(hd_wallet) => self.enable_hd_wallet(hd_wallet).await.map(EnableCoinBalance::HD),
        }
    }
}

#[async_trait]
pub trait HDWalletBalanceOps: HDWalletCoinOps {
    type HDAddressChecker: HDAddressBalanceChecker<Address = Self::Address>;

    async fn produce_hd_address_checker(&self) -> BalanceResult<Self::HDAddressChecker>;

    /// Scans for the new addresses of every known account by using [`HDWalletBalanceOps::scan_for_new_addresses`].
    /// This method is used on coin initialization to index working addresses and to return the wallet balance to the user.
    async fn enable_hd_wallet(&self, hd_wallet: &Self::HDWallet) -> BalanceResult<HDWalletBalance>;

    /// Scans for the new addresses of the specified `hd_account` using the given `address_checker`.
    /// Returns balances of the new addresses.
    async fn scan_for_new_addresses(
        &self,
        hd_account: &mut Self::HDAccount,
        address_checker: &Self::HDAddressChecker,
        gap_limit: u32,
    ) -> BalanceResult<Vec<HDAddressBalance>>;

    /// Requests balance of the given `address`.
    /// This function is expected to be more efficient than ['HDWalletBalanceOps::is_address_used'] in most cases
    /// since many of RPC clients allow us to request the address balance without the history.
    async fn known_address_balance(&self, address: &Self::Address) -> BalanceResult<CoinBalance>;

    /// Checks if the address has been used by the user by checking if the transaction history of the given `address` is not empty.
    /// Please note the function can return zero balance even if the address has been used before.
    async fn is_address_used(
        &self,
        address: &Self::Address,
        checker: &Self::HDAddressChecker,
    ) -> BalanceResult<AddressBalanceStatus<CoinBalance>> {
        if !checker.is_address_used(address).await? {
            return Ok(AddressBalanceStatus::NotUsed);
        }
        // Now we know that the address has been used.
        let balance = self.known_address_balance(address).await?;
        Ok(AddressBalanceStatus::Used(balance))
    }
}

#[async_trait]
pub trait HDAddressBalanceChecker: Sync {
    type Address;

    async fn is_address_used(&self, address: &Self::Address) -> BalanceResult<bool>;
}

pub enum AddressBalanceStatus<Balance> {
    Used(Balance),
    NotUsed,
}

#[async_trait]
pub trait HDWalletBalanceRpcOps {
    async fn account_balance_rpc(
        &self,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError>;

    async fn scan_for_new_addresses_rpc(
        &self,
        params: CheckHDAccountBalanceParams,
    ) -> MmResult<CheckHDAccountBalanceResponse, HDAccountBalanceRpcError>;
}

pub async fn account_balance(
    ctx: MmArc,
    req: HDAccountBalanceRequest,
) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::UtxoCoin(utxo) => utxo.account_balance_rpc(req.params).await,
        MmCoinEnum::QtumCoin(qtum) => qtum.account_balance_rpc(req.params).await,
        _ => MmError::err(HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet {
            coin: coin.ticker().to_owned(),
        }),
    }
}

pub async fn scan_for_new_addresses(
    ctx: MmArc,
    req: CheckHDAccountBalanceRequest,
) -> MmResult<CheckHDAccountBalanceResponse, HDAccountBalanceRpcError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::UtxoCoin(utxo) => utxo.scan_for_new_addresses_rpc(req.params).await,
        MmCoinEnum::QtumCoin(qtum) => qtum.scan_for_new_addresses_rpc(req.params).await,
        _ => MmError::err(HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet {
            coin: coin.ticker().to_owned(),
        }),
    }
}

pub mod common_impl {
    use super::*;
    use crate::hd_wallet::{HDAccountOps, HDAddress, HDWalletOps};
    use common::calc_total_pages;

    pub(crate) async fn enable_hd_wallet<Coin>(
        coin: &Coin,
        hd_wallet: &Coin::HDWallet,
    ) -> BalanceResult<HDWalletBalance>
    where
        Coin: HDWalletBalanceOps + Sync,
    {
        let mut accounts = hd_wallet.get_accounts_mut().await;
        let gap_limit = hd_wallet.gap_limit();
        let address_checker = coin.produce_hd_address_checker().await?;

        let mut result = HDWalletBalance {
            accounts: Vec::with_capacity(accounts.len()),
        };

        for (account_index, hd_account) in accounts.iter_mut() {
            let addresses = coin
                .scan_for_new_addresses(hd_account, &address_checker, gap_limit)
                .await?;

            let total_balance = addresses.iter().fold(CoinBalance::default(), |total, addr_balance| {
                total + addr_balance.balance.clone()
            });
            let account_balance = HDAccountBalance {
                account_index: *account_index,
                derivation_path: RpcDerivationPath(hd_account.account_derivation_path()),
                total_balance,
                addresses,
            };

            result.accounts.push(account_balance);
        }

        Ok(result)
    }

    pub async fn account_balance_rpc<Coin>(
        coin: &Coin,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError>
    where
        Coin: HDWalletBalanceOps
            + CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet>
            + MarketCoinOps
            + Sync,
        <Coin as HDWalletCoinOps>::Address: fmt::Display,
    {
        let hd_wallet = coin.derivation_method().hd_wallet().or_mm_err(|| {
            HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet {
                coin: coin.ticker().to_owned(),
            }
        })?;

        let account_id = params.account_index;
        let chain = params.chain;
        let hd_account = hd_wallet
            .get_account(account_id)
            .await
            .or_mm_err(|| HDAccountBalanceRpcError::UnknownAccount { account_id })?;
        let total_addresses_number = hd_account.known_addresses_number(params.chain)?;

        let from_address_id = match params.paging_options {
            PagingOptionsEnum::FromId(from_address_id) => from_address_id,
            PagingOptionsEnum::PageNumber(page_number) => ((page_number.get() - 1) * params.limit) as u32,
        };
        let to_address_id = std::cmp::min(from_address_id + params.limit as u32, total_addresses_number);

        let mut result = HDAccountBalanceResponse {
            account_index: params.account_index,
            derivation_path: RpcDerivationPath(hd_account.account_derivation_path()),
            addresses: Vec::with_capacity(params.limit),
            limit: params.limit,
            skipped: std::cmp::min(from_address_id, total_addresses_number),
            total: total_addresses_number,
            total_pages: calc_total_pages(total_addresses_number as usize, params.limit),
            paging_options: params.paging_options,
        };

        for address_id in from_address_id..to_address_id {
            let HDAddress {
                address,
                derivation_path,
                ..
            } = coin.derive_address(&hd_account, chain, address_id)?;
            let balance = coin.known_address_balance(&address).await?;

            result.addresses.push(HDAddressBalance {
                address: address.to_string(),
                derivation_path: RpcDerivationPath(derivation_path),
                chain,
                balance,
            });
        }

        Ok(result)
    }

    pub async fn scan_for_new_addresses_rpc<Coin>(
        coin: &Coin,
        params: CheckHDAccountBalanceParams,
    ) -> MmResult<CheckHDAccountBalanceResponse, HDAccountBalanceRpcError>
    where
        Coin: CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet>
            + HDWalletBalanceOps
            + MarketCoinOps
            + Sync,
        <Coin as HDWalletCoinOps>::Address: fmt::Display,
    {
        let hd_wallet = coin.derivation_method().hd_wallet().or_mm_err(|| {
            HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet {
                coin: coin.ticker().to_owned(),
            }
        })?;

        let account_id = params.account_index;
        let mut hd_account = hd_wallet
            .get_account_mut(account_id)
            .await
            .or_mm_err(|| HDAccountBalanceRpcError::UnknownAccount { account_id })?;
        let account_derivation_path = hd_account.account_derivation_path();
        let address_checker = coin.produce_hd_address_checker().await?;
        let gap_limit = params.gap_limit.unwrap_or_else(|| hd_wallet.gap_limit());

        let new_addresses = coin
            .scan_for_new_addresses(&mut hd_account, &address_checker, gap_limit)
            .await?;

        Ok(CheckHDAccountBalanceResponse {
            account_index: account_id,
            derivation_path: RpcDerivationPath(account_derivation_path),
            new_addresses,
        })
    }
}

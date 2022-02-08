use crate::hd_wallet::{AddressDerivingError, HDWalletCoinOps, InvalidBip44ChainError};
use crate::{lp_coinfind_or_err, BalanceError, BalanceResult, Bip44Chain, CoinBalance, CoinFindError,
            CoinWithDerivationMethod, DerivationMethod, MarketCoinOps, MmCoinEnum};
use async_trait::async_trait;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{HttpStatusCode, PagingOptionsEnum};
use crypto::RpcDerivationPath;
use derive_more::Display;
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
        fmt = "'{}' coin is expected to be enabled with the HD wallet derivation method",
        coin
    )]
    ExpectedHDWalletDerivationMethod { coin: String },
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
            | HDAccountBalanceRpcError::ExpectedHDWalletDerivationMethod { .. }
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
    params: HDAccountBalanceParams,
}

#[derive(Deserialize)]
pub struct HDAccountBalanceParams {
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
impl<Coin, Address, HDWallet, HDAccount, HDAddressChecker> EnableCoinBalanceOps for Coin
where
    Coin: AddressBalanceOps<Address = Address>
        + CoinWithDerivationMethod<Address = Address, HDWallet = HDWallet>
        + HDWalletBalanceOps<HDWallet = HDWallet, HDAccount = HDAccount, HDAddressChecker = HDAddressChecker>
        + Sync,
    Address: fmt::Display + Sync,
    HDWallet: Sync,
    HDAddressChecker: HDAddressBalanceChecker,
{
    async fn enable_coin_balance(&self) -> BalanceResult<EnableCoinBalance> {
        match self.derivation_method() {
            DerivationMethod::Iguana(address) => self.address_balance(address).await.map(|balance| {
                EnableCoinBalance::Iguana(IguanaWalletBalance {
                    address: address.to_string(),
                    balance,
                })
            }),
            DerivationMethod::HDWallet(hd_wallet) => self
                .enable_hd_wallet_balance(hd_wallet)
                .await
                .map(EnableCoinBalance::HD),
        }
    }
}

#[async_trait]
pub trait HDWalletBalanceOps: AddressBalanceOps {
    type HDWallet;
    type HDAccount;
    type HDAddressChecker: HDAddressBalanceChecker<Address = Self::Address>;

    async fn produce_hd_address_checker(&self) -> BalanceResult<Self::HDAddressChecker>;

    /// Checks for addresses of every known account by using [`HDWalletBalanceOps::check_hd_account_balance`].
    /// This method is used on coin initialization to index working addresses and to return the wallet balance to the user.
    async fn enable_hd_wallet_balance(&self, hd_wallet: &Self::HDWallet) -> BalanceResult<HDWalletBalance>;

    /// Checks for the new account addresses using the given `address_checker`.
    /// Returns balances of new addresses.
    async fn check_hd_account_balance(
        &self,
        hd_account: &mut Self::HDAccount,
        address_checker: &Self::HDAddressChecker,
        gap_limit: u32,
    ) -> BalanceResult<Vec<HDAddressBalance>>;

    /// Requests balance of the given `address`.
    /// This function is expected to be more efficient than ['HDWalletBalanceOps::check_address_balance'] in most cases
    /// since many of RPC clients allow us to request the address balance without the history.
    async fn known_address_balance(&self, address: &Self::Address) -> BalanceResult<CoinBalance> {
        self.address_balance(address).await
    }

    /// Checks if the address has been used by the user by checking if the transaction history of the given `address` is not empty.
    /// Please note the function can return zero balance even if the address has been used before.
    async fn check_address_balance(
        &self,
        address: &Self::Address,
        checker: &Self::HDAddressChecker,
    ) -> BalanceResult<AddressBalanceStatus<CoinBalance>> {
        if !checker.is_address_used(address).await? {
            return Ok(AddressBalanceStatus::NotUsed);
        }
        let balance = self.address_balance(address).await?;
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
pub trait AddressBalanceOps {
    type Address: Sync;

    async fn address_balance(&self, address: &Self::Address) -> BalanceResult<CoinBalance>;
}

#[async_trait]
pub trait HDWalletBalanceRpcOps: HDWalletCoinOps {
    async fn hd_account_balance_rpc(
        &self,
        params: HDAccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError>;

    async fn check_hd_account_balance_rpc(
        &self,
        params: CheckHDAccountBalanceParams,
    ) -> MmResult<CheckHDAccountBalanceResponse, HDAccountBalanceRpcError>;
}

pub async fn hd_account_balance(
    ctx: MmArc,
    req: HDAccountBalanceRequest,
) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::UtxoCoin(utxo) => utxo.hd_account_balance_rpc(req.params).await,
        MmCoinEnum::QtumCoin(qtum) => qtum.hd_account_balance_rpc(req.params).await,
        _ => MmError::err(HDAccountBalanceRpcError::ExpectedHDWalletDerivationMethod {
            coin: coin.ticker().to_owned(),
        }),
    }
}

pub async fn check_hd_account_balance(
    ctx: MmArc,
    req: CheckHDAccountBalanceRequest,
) -> MmResult<CheckHDAccountBalanceResponse, HDAccountBalanceRpcError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::UtxoCoin(utxo) => utxo.check_hd_account_balance_rpc(req.params).await,
        MmCoinEnum::QtumCoin(qtum) => qtum.check_hd_account_balance_rpc(req.params).await,
        _ => MmError::err(HDAccountBalanceRpcError::ExpectedHDWalletDerivationMethod {
            coin: coin.ticker().to_owned(),
        }),
    }
}

pub mod common_impl {
    use super::*;
    use crate::hd_wallet::HDAddress;
    use common::calc_total_pages;

    pub(crate) async fn enable_hd_wallet_balance<Coin, Address, HDWallet, HDAccount, HDAccountChecker>(
        coin: &Coin,
        hd_wallet: &HDWallet,
    ) -> BalanceResult<HDWalletBalance>
    where
        Coin: HDWalletCoinOps<Address = Address, HDWallet = HDWallet, HDAccount = HDAccount>
            + HDWalletBalanceOps<
                Address = Address,
                HDWallet = HDWallet,
                HDAccount = HDAccount,
                HDAddressChecker = HDAccountChecker,
            > + Sync,
    {
        let mut accounts = coin.get_accounts_mut(hd_wallet).await;
        let gap_limit = coin.gap_limit(hd_wallet);
        let address_checker = coin.produce_hd_address_checker().await?;

        let mut result = HDWalletBalance {
            accounts: Vec::with_capacity(accounts.len()),
        };

        for (account_index, hd_account) in accounts.iter_mut() {
            let addresses = coin
                .check_hd_account_balance(hd_account, &address_checker, gap_limit)
                .await?;

            let total_balance = addresses.iter().fold(CoinBalance::default(), |total, addr_balance| {
                total + addr_balance.balance.clone()
            });
            let account_balance = HDAccountBalance {
                account_index: *account_index,
                derivation_path: RpcDerivationPath(coin.account_derivation_path(hd_account)),
                total_balance,
                addresses,
            };

            result.accounts.push(account_balance);
        }

        Ok(result)
    }

    pub async fn hd_account_balance_rpc<Coin, Address, HDWallet, HDAccount>(
        coin: &Coin,
        params: HDAccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError>
    where
        Coin: CoinWithDerivationMethod<Address = Address, HDWallet = HDWallet>
            + HDWalletCoinOps<Address = Address, HDWallet = HDWallet, HDAccount = HDAccount>
            + HDWalletBalanceOps<Address = Address, HDWallet = HDWallet, HDAccount = HDAccount>
            + HDWalletBalanceRpcOps<Address = Address, HDWallet = HDWallet, HDAccount = HDAccount>
            + MarketCoinOps
            + Sync,
        Address: fmt::Display,
    {
        let hd_wallet = coin.derivation_method().hd_wallet().or_mm_err(|| {
            HDAccountBalanceRpcError::ExpectedHDWalletDerivationMethod {
                coin: coin.ticker().to_owned(),
            }
        })?;

        let account_id = params.account_index;
        let chain = params.chain;
        let hd_account = coin
            .get_account(hd_wallet, account_id)
            .await
            .or_mm_err(|| HDAccountBalanceRpcError::UnknownAccount { account_id })?;
        let total_addresses_number = coin.number_of_used_account_addresses(&hd_account, params.chain)?;

        let from_address_id = match params.paging_options {
            PagingOptionsEnum::FromId(from_address_id) => from_address_id,
            PagingOptionsEnum::PageNumber(page_number) => ((page_number.get() - 1) * params.limit) as u32,
        };
        let to_address_id = std::cmp::min(from_address_id + params.limit as u32, total_addresses_number);

        let mut result = HDAccountBalanceResponse {
            account_index: params.account_index,
            derivation_path: RpcDerivationPath(coin.account_derivation_path(&hd_account)),
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
            } = coin.derive_address(&hd_account, chain, address_id)?;
            let balance = coin.address_balance(&address).await?;

            result.addresses.push(HDAddressBalance {
                address: address.to_string(),
                derivation_path: RpcDerivationPath(derivation_path),
                chain,
                balance,
            });
        }

        Ok(result)
    }

    pub async fn check_hd_account_balance_rpc<Coin, HDWallet, HDAccount>(
        coin: &Coin,
        params: CheckHDAccountBalanceParams,
    ) -> MmResult<CheckHDAccountBalanceResponse, HDAccountBalanceRpcError>
    where
        Coin: CoinWithDerivationMethod<HDWallet = HDWallet>
            + HDWalletCoinOps<HDWallet = HDWallet, HDAccount = HDAccount>
            + HDWalletBalanceOps<HDWallet = HDWallet, HDAccount = HDAccount>
            + MarketCoinOps
            + Sync,
    {
        let hd_wallet = coin.derivation_method().hd_wallet().or_mm_err(|| {
            HDAccountBalanceRpcError::ExpectedHDWalletDerivationMethod {
                coin: coin.ticker().to_owned(),
            }
        })?;

        let account_id = params.account_index;
        let mut hd_account = coin
            .get_account_mut(hd_wallet, account_id)
            .await
            .or_mm_err(|| HDAccountBalanceRpcError::UnknownAccount { account_id })?;
        let account_derivation_path = coin.account_derivation_path(&hd_account);
        let address_checker = coin.produce_hd_address_checker().await?;
        let gap_limit = params.gap_limit.unwrap_or_else(|| coin.gap_limit(hd_wallet));

        let new_addresses = coin
            .check_hd_account_balance(&mut hd_account, &address_checker, gap_limit)
            .await?;

        Ok(CheckHDAccountBalanceResponse {
            account_index: account_id,
            derivation_path: RpcDerivationPath(account_derivation_path),
            new_addresses,
        })
    }
}

use crate::hd_pubkey::HDXPubExtractor;
use crate::hd_wallet::{AddressDerivingError, HDWalletCoinOps, InvalidBip44ChainError, NewAccountCreatingError};
use crate::{lp_coinfind_or_err, BalanceError, BalanceResult, CoinBalance, CoinFindError, CoinWithDerivationMethod,
            DerivationMethod, HDAddress, MarketCoinOps, MmCoinEnum, UnexpectedDerivationMethod};
use async_trait::async_trait;
use common::log::{debug, info};
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
    #[display(fmt = "Coin is expected to be activated with the HD wallet derivation method")]
    CoinIsActivatedNotWithHDWallet,
    #[display(fmt = "HD account '{}' is not activated", account_id)]
    UnknownAccount { account_id: u32 },
    #[display(fmt = "Coin doesn't support the given BIP44 chain: {:?}", chain)]
    InvalidBip44Chain { chain: Bip44Chain },
    #[display(fmt = "Error deriving an address: {}", _0)]
    ErrorDerivingAddress(String),
    #[display(fmt = "Wallet storage error: {}", _0)]
    WalletStorageError(String),
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
            | HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet
            | HDAccountBalanceRpcError::UnknownAccount { .. }
            | HDAccountBalanceRpcError::InvalidBip44Chain { .. }
            | HDAccountBalanceRpcError::ErrorDerivingAddress(_) => StatusCode::BAD_REQUEST,
            HDAccountBalanceRpcError::Transport(_)
            | HDAccountBalanceRpcError::WalletStorageError(_)
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

impl From<UnexpectedDerivationMethod> for HDAccountBalanceRpcError {
    fn from(e: UnexpectedDerivationMethod) -> Self {
        match e {
            UnexpectedDerivationMethod::HDWalletUnavailable => HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet,
            unexpected_error => HDAccountBalanceRpcError::Internal(unexpected_error.to_string()),
        }
    }
}

impl From<BalanceError> for HDAccountBalanceRpcError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(transport) => HDAccountBalanceRpcError::Transport(transport),
            BalanceError::InvalidResponse(rpc) => HDAccountBalanceRpcError::RpcInvalidResponse(rpc),
            BalanceError::UnexpectedDerivationMethod(der_method) => HDAccountBalanceRpcError::from(der_method),
            BalanceError::WalletStorageError(e) => HDAccountBalanceRpcError::Internal(e),
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

#[derive(Display)]
pub enum EnableCoinBalanceError {
    NewAccountCreatingError(NewAccountCreatingError),
    BalanceError(BalanceError),
}

impl From<NewAccountCreatingError> for EnableCoinBalanceError {
    fn from(e: NewAccountCreatingError) -> Self { EnableCoinBalanceError::NewAccountCreatingError(e) }
}

impl From<BalanceError> for EnableCoinBalanceError {
    fn from(e: BalanceError) -> Self { EnableCoinBalanceError::BalanceError(e) }
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
    pub page_balance: CoinBalance,
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnableCoinScanPolicy {
    /// Don't scan for new addresses.
    DoNotScan,
    /// Scan for new addresses if the coin HD wallet hasn't been enabled *only*.
    /// In other words, scan for new addresses if there were no HD accounts in the HD wallet storage.
    ScanIfNewWallet,
    /// Scan for new addresses even if the coin HD wallet has been enabled before.
    Scan,
}

impl Default for EnableCoinScanPolicy {
    fn default() -> Self { EnableCoinScanPolicy::ScanIfNewWallet }
}

#[async_trait]
pub trait EnableCoinBalanceOps {
    async fn enable_coin_balance<XPubExtractor>(
        &self,
        xpub_extractor: &XPubExtractor,
        scan_policy: EnableCoinScanPolicy,
    ) -> MmResult<EnableCoinBalance, EnableCoinBalanceError>
    where
        XPubExtractor: HDXPubExtractor + Sync;
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
    async fn enable_coin_balance<XPubExtractor>(
        &self,
        xpub_extractor: &XPubExtractor,
        scan_policy: EnableCoinScanPolicy,
    ) -> MmResult<EnableCoinBalance, EnableCoinBalanceError>
    where
        XPubExtractor: HDXPubExtractor + Sync,
    {
        match self.derivation_method() {
            DerivationMethod::Iguana(my_address) => self
                .my_balance()
                .compat()
                .await
                .map(|balance| {
                    EnableCoinBalance::Iguana(IguanaWalletBalance {
                        address: my_address.to_string(),
                        balance,
                    })
                })
                .mm_err(EnableCoinBalanceError::from),
            DerivationMethod::HDWallet(hd_wallet) => self
                .enable_hd_wallet(hd_wallet, xpub_extractor, scan_policy)
                .await
                .map(EnableCoinBalance::HD),
        }
    }
}

#[async_trait]
pub trait HDWalletBalanceOps: HDWalletCoinOps {
    type HDAddressScanner: HDAddressBalanceScanner<Address = Self::Address>;

    async fn produce_hd_address_scanner(&self) -> BalanceResult<Self::HDAddressScanner>;

    /// Requests balances of already known addresses, and if it's prescribed by [`EnableCoinParams::scan_policy`],
    /// scans for new addresses of every HD account by using [`HDWalletBalanceOps::scan_for_new_addresses`].
    /// This method is used on coin initialization to index working addresses and to return the wallet balance to the user.
    async fn enable_hd_wallet<XPubExtractor>(
        &self,
        hd_wallet: &Self::HDWallet,
        xpub_extractor: &XPubExtractor,
        scan_policy: EnableCoinScanPolicy,
    ) -> MmResult<HDWalletBalance, EnableCoinBalanceError>
    where
        XPubExtractor: HDXPubExtractor + Sync;

    /// Scans for the new addresses of the specified `hd_account` using the given `address_scanner`.
    /// Returns balances of the new addresses.
    async fn scan_for_new_addresses(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut Self::HDAccount,
        address_scanner: &Self::HDAddressScanner,
        gap_limit: u32,
    ) -> BalanceResult<Vec<HDAddressBalance>>;

    /// Requests balances of every known addresses of the given `hd_account`.
    async fn all_known_addresses_balances(&self, hd_account: &Self::HDAccount) -> BalanceResult<Vec<HDAddressBalance>>;

    /// Requests balances of known addresses of the given `address_ids` addresses at the specified `chain`.
    async fn known_addresses_balances_with_ids<Ids>(
        &self,
        hd_account: &Self::HDAccount,
        chain: Bip44Chain,
        address_ids: Ids,
    ) -> BalanceResult<Vec<HDAddressBalance>>
    where
        Self::Address: fmt::Display,
        Ids: Iterator<Item = u32> + Send,
    {
        let (lower, upper) = address_ids.size_hint();
        let max_addresses = upper.unwrap_or(lower);

        let mut balances = Vec::with_capacity(max_addresses);
        for address_id in address_ids {
            let HDAddress {
                address,
                derivation_path,
                ..
            } = self.derive_address(hd_account, chain, address_id)?;
            let balance = self.known_address_balance(&address).await?;
            balances.push(HDAddressBalance {
                address: address.to_string(),
                derivation_path: RpcDerivationPath(derivation_path),
                chain,
                balance,
            });
        }
        Ok(balances)
    }

    /// Requests balance of the given `address`.
    /// This function is expected to be more efficient than ['HDWalletBalanceOps::is_address_used'] in most cases
    /// since many of RPC clients allow us to request the address balance without the history.
    async fn known_address_balance(&self, address: &Self::Address) -> BalanceResult<CoinBalance>;

    /// Checks if the address has been used by the user by checking if the transaction history of the given `address` is not empty.
    /// Please note the function can return zero balance even if the address has been used before.
    async fn is_address_used(
        &self,
        address: &Self::Address,
        address_scanner: &Self::HDAddressScanner,
    ) -> BalanceResult<AddressBalanceStatus<CoinBalance>> {
        if !address_scanner.is_address_used(address).await? {
            return Ok(AddressBalanceStatus::NotUsed);
        }
        // Now we know that the address has been used.
        let balance = self.known_address_balance(address).await?;
        Ok(AddressBalanceStatus::Used(balance))
    }
}

#[async_trait]
pub trait HDAddressBalanceScanner: Sync {
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
        _ => MmError::err(HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet),
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
        _ => MmError::err(HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet),
    }
}

pub mod common_impl {
    use super::*;
    use crate::hd_wallet::{HDAccountOps, HDWalletOps};
    use common::calc_total_pages;
    use std::ops::DerefMut;

    pub(crate) async fn enable_hd_account<Coin>(
        coin: &Coin,
        hd_wallet: &Coin::HDWallet,
        hd_account: &mut Coin::HDAccount,
        address_scanner: &Coin::HDAddressScanner,
        scan_new_addresses: bool,
    ) -> MmResult<HDAccountBalance, EnableCoinBalanceError>
    where
        Coin: HDWalletBalanceOps + Sync,
    {
        let gap_limit = hd_wallet.gap_limit();
        let mut addresses = coin.all_known_addresses_balances(hd_account).await?;
        if scan_new_addresses {
            addresses.extend(
                coin.scan_for_new_addresses(hd_wallet, hd_account, address_scanner, gap_limit)
                    .await?,
            );
        }

        let total_balance = addresses.iter().fold(CoinBalance::default(), |total, addr_balance| {
            total + addr_balance.balance.clone()
        });
        let account_balance = HDAccountBalance {
            account_index: hd_account.account_id(),
            derivation_path: RpcDerivationPath(hd_account.account_derivation_path()),
            total_balance,
            addresses,
        };

        Ok(account_balance)
    }

    pub(crate) async fn enable_hd_wallet<Coin, XPubExtractor>(
        coin: &Coin,
        hd_wallet: &Coin::HDWallet,
        xpub_extractor: &XPubExtractor,
        scan_policy: EnableCoinScanPolicy,
    ) -> MmResult<HDWalletBalance, EnableCoinBalanceError>
    where
        Coin: HDWalletBalanceOps + MarketCoinOps + Sync,
        XPubExtractor: HDXPubExtractor + Sync,
    {
        let mut accounts = hd_wallet.get_accounts_mut().await;
        let address_scanner = coin.produce_hd_address_scanner().await?;

        let mut result = HDWalletBalance {
            accounts: Vec::with_capacity(accounts.len() + 1),
        };

        if accounts.is_empty() {
            // Is seems that we couldn't find any HD account from the HD wallet storage.
            drop(accounts);
            info!(
                "{} HD wallet hasn't been enabled before. Create default HD account",
                coin.ticker()
            );

            // Create new HD account.
            let mut new_account = coin.create_new_account(hd_wallet, xpub_extractor).await?;
            let scan_new_addresses = matches!(
                scan_policy,
                EnableCoinScanPolicy::ScanIfNewWallet | EnableCoinScanPolicy::Scan
            );

            let account_balance =
                enable_hd_account(coin, hd_wallet, &mut new_account, &address_scanner, scan_new_addresses).await?;
            result.accounts.push(account_balance);
            return Ok(result);
        }

        debug!(
            "{} HD accounts were found on {} coin activation",
            accounts.len(),
            coin.ticker()
        );
        let scan_new_addresses = matches!(scan_policy, EnableCoinScanPolicy::Scan);
        for (_account_id, hd_account) in accounts.iter_mut() {
            let account_balance =
                enable_hd_account(coin, hd_wallet, hd_account, &address_scanner, scan_new_addresses).await?;
            result.accounts.push(account_balance);
        }

        Ok(result)
    }

    pub async fn account_balance_rpc<Coin>(
        coin: &Coin,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError>
    where
        Coin: HDWalletBalanceOps + CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet> + Sync,
        <Coin as HDWalletCoinOps>::Address: fmt::Display,
    {
        let hd_wallet = coin.derivation_method().hd_wallet_or_err()?;

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

        let addresses = coin
            .known_addresses_balances_with_ids(&hd_account, chain, from_address_id..to_address_id)
            .await?;
        let page_balance = addresses.iter().fold(CoinBalance::default(), |total, addr_balance| {
            total + addr_balance.balance.clone()
        });

        let result = HDAccountBalanceResponse {
            account_index: params.account_index,
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

    pub async fn scan_for_new_addresses_rpc<Coin>(
        coin: &Coin,
        params: CheckHDAccountBalanceParams,
    ) -> MmResult<CheckHDAccountBalanceResponse, HDAccountBalanceRpcError>
    where
        Coin: CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet> + HDWalletBalanceOps + Sync,
        <Coin as HDWalletCoinOps>::Address: fmt::Display,
    {
        let hd_wallet = coin.derivation_method().hd_wallet_or_err()?;

        let account_id = params.account_index;
        let mut hd_account = hd_wallet
            .get_account_mut(account_id)
            .await
            .or_mm_err(|| HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet)?;
        let account_derivation_path = hd_account.account_derivation_path();
        let address_scanner = coin.produce_hd_address_scanner().await?;
        let gap_limit = params.gap_limit.unwrap_or_else(|| hd_wallet.gap_limit());

        let new_addresses = coin
            .scan_for_new_addresses(hd_wallet, hd_account.deref_mut(), &address_scanner, gap_limit)
            .await?;

        Ok(CheckHDAccountBalanceResponse {
            account_index: account_id,
            derivation_path: RpcDerivationPath(account_derivation_path),
            new_addresses,
        })
    }
}

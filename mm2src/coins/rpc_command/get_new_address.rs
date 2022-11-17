use crate::coin_balance::HDAddressBalance;
use crate::hd_wallet::{AddressDerivingError, InvalidBip44ChainError, NewAddressDerivingError};
use crate::{lp_coinfind_or_err, BalanceError, CoinFindError, MmCoinEnum, UnexpectedDerivationMethod};
use async_trait::async_trait;
use common::HttpStatusCode;
use crypto::Bip44Chain;
use derive_more::Display;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;

#[derive(Clone, Debug, Display, PartialEq, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetNewAddressRpcError {
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
    #[display(fmt = "Addresses limit reached. Max number of addresses: {}", max_addresses_number)]
    AddressLimitReached { max_addresses_number: u32 },
    #[display(fmt = "Empty addresses limit reached. Gap limit: {}", gap_limit)]
    EmptyAddressesLimitReached { gap_limit: u32 },
    #[display(fmt = "Electrum/Native RPC invalid response: {}", _0)]
    RpcInvalidResponse(String),
    #[display(fmt = "HD wallet storage error: {}", _0)]
    WalletStorageError(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
}

impl From<BalanceError> for GetNewAddressRpcError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(transport) => GetNewAddressRpcError::Transport(transport),
            BalanceError::InvalidResponse(rpc) => GetNewAddressRpcError::RpcInvalidResponse(rpc),
            BalanceError::UnexpectedDerivationMethod(der_path) => GetNewAddressRpcError::from(der_path),
            BalanceError::WalletStorageError(internal) | BalanceError::Internal(internal) => {
                GetNewAddressRpcError::Internal(internal)
            },
        }
    }
}

impl From<UnexpectedDerivationMethod> for GetNewAddressRpcError {
    fn from(e: UnexpectedDerivationMethod) -> Self {
        match e {
            UnexpectedDerivationMethod::ExpectedHDWallet => GetNewAddressRpcError::CoinIsActivatedNotWithHDWallet,
            unexpected_error => GetNewAddressRpcError::Internal(unexpected_error.to_string()),
        }
    }
}

impl From<CoinFindError> for GetNewAddressRpcError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => GetNewAddressRpcError::NoSuchCoin { coin },
        }
    }
}

impl From<InvalidBip44ChainError> for GetNewAddressRpcError {
    fn from(e: InvalidBip44ChainError) -> Self { GetNewAddressRpcError::InvalidBip44Chain { chain: e.chain } }
}

impl From<NewAddressDerivingError> for GetNewAddressRpcError {
    fn from(e: NewAddressDerivingError) -> Self {
        match e {
            NewAddressDerivingError::AddressLimitReached { max_addresses_number } => {
                GetNewAddressRpcError::AddressLimitReached { max_addresses_number }
            },
            NewAddressDerivingError::InvalidBip44Chain { chain } => GetNewAddressRpcError::InvalidBip44Chain { chain },
            NewAddressDerivingError::Bip32Error(bip32) => GetNewAddressRpcError::Internal(bip32.to_string()),
            NewAddressDerivingError::WalletStorageError(storage) => {
                GetNewAddressRpcError::WalletStorageError(storage.to_string())
            },
            NewAddressDerivingError::Internal(internal) => GetNewAddressRpcError::Internal(internal),
        }
    }
}

impl From<AddressDerivingError> for GetNewAddressRpcError {
    fn from(e: AddressDerivingError) -> Self {
        match e {
            AddressDerivingError::InvalidBip44Chain { chain } => GetNewAddressRpcError::InvalidBip44Chain { chain },
            AddressDerivingError::Bip32Error(bip32) => GetNewAddressRpcError::ErrorDerivingAddress(bip32.to_string()),
            AddressDerivingError::Internal(internal) => GetNewAddressRpcError::Internal(internal),
        }
    }
}

impl HttpStatusCode for GetNewAddressRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetNewAddressRpcError::NoSuchCoin { .. }
            | GetNewAddressRpcError::CoinIsActivatedNotWithHDWallet
            | GetNewAddressRpcError::UnknownAccount { .. }
            | GetNewAddressRpcError::InvalidBip44Chain { .. }
            | GetNewAddressRpcError::ErrorDerivingAddress(_)
            | GetNewAddressRpcError::AddressLimitReached { .. }
            | GetNewAddressRpcError::EmptyAddressesLimitReached { .. } => StatusCode::BAD_REQUEST,
            GetNewAddressRpcError::Transport(_)
            | GetNewAddressRpcError::RpcInvalidResponse(_)
            | GetNewAddressRpcError::WalletStorageError(_)
            | GetNewAddressRpcError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Deserialize)]
pub struct GetNewAddressRequest {
    coin: String,
    #[serde(flatten)]
    params: GetNewAddressParams,
}

#[derive(Deserialize)]
pub struct GetNewAddressParams {
    pub(crate) account_id: u32,
    pub(crate) chain: Option<Bip44Chain>,
    // The max number of empty addresses in a row.
    // If there are more or equal to the `gap_limit` last empty addresses in a row,
    // we'll not allow to generate new address.
    pub(crate) gap_limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct GetNewAddressResponse {
    new_address: HDAddressBalance,
}

#[async_trait]
pub trait GetNewAddressRpcOps {
    async fn get_new_address_rpc(
        &self,
        params: GetNewAddressParams,
    ) -> MmResult<GetNewAddressResponse, GetNewAddressRpcError>;
}

/// Generates a new address.
pub async fn get_new_address(
    ctx: MmArc,
    req: GetNewAddressRequest,
) -> MmResult<GetNewAddressResponse, GetNewAddressRpcError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::UtxoCoin(utxo) => utxo.get_new_address_rpc(req.params).await,
        MmCoinEnum::QtumCoin(qtum) => qtum.get_new_address_rpc(req.params).await,
        _ => MmError::err(GetNewAddressRpcError::CoinIsActivatedNotWithHDWallet),
    }
}

pub mod common_impl {
    use super::*;
    use crate::coin_balance::{HDAddressBalanceScanner, HDWalletBalanceOps};
    use crate::hd_wallet::{HDAccountOps, HDWalletCoinOps, HDWalletOps};
    use crate::{CoinWithDerivationMethod, HDAddress};
    use crypto::RpcDerivationPath;
    use std::fmt;
    use std::ops::DerefMut;

    pub async fn get_new_address_rpc<Coin>(
        coin: &Coin,
        params: GetNewAddressParams,
    ) -> MmResult<GetNewAddressResponse, GetNewAddressRpcError>
    where
        Coin:
            HDWalletBalanceOps + CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet> + Sync + Send,
        <Coin as HDWalletCoinOps>::Address: fmt::Display,
    {
        let hd_wallet = coin.derivation_method().hd_wallet_or_err()?;

        let account_id = params.account_id;
        let mut hd_account = hd_wallet
            .get_account_mut(account_id)
            .await
            .or_mm_err(|| GetNewAddressRpcError::UnknownAccount { account_id })?;

        let chain = params.chain.unwrap_or_else(|| hd_wallet.default_receiver_chain());
        let gap_limit = params.gap_limit.unwrap_or_else(|| hd_wallet.gap_limit());

        // Check if we can generate new address.
        check_if_can_get_new_address(coin, hd_wallet, &hd_account, chain, gap_limit).await?;

        let HDAddress {
            address,
            derivation_path,
            ..
        } = coin
            .generate_new_address(hd_wallet, hd_account.deref_mut(), chain)
            .await?;
        let balance = coin.known_address_balance(&address).await?;

        Ok(GetNewAddressResponse {
            new_address: HDAddressBalance {
                address: address.to_string(),
                derivation_path: RpcDerivationPath(derivation_path),
                chain,
                balance,
            },
        })
    }

    async fn check_if_can_get_new_address<Coin>(
        coin: &Coin,
        hd_wallet: &Coin::HDWallet,
        hd_account: &Coin::HDAccount,
        chain: Bip44Chain,
        gap_limit: u32,
    ) -> MmResult<(), GetNewAddressRpcError>
    where
        Coin: HDWalletBalanceOps + Sync,
        <Coin as HDWalletCoinOps>::Address: fmt::Display,
    {
        let known_addresses_number = hd_account.known_addresses_number(chain)?;
        if known_addresses_number == 0 || gap_limit > known_addresses_number {
            return Ok(());
        }

        let max_addresses_number = hd_wallet.address_limit();
        if known_addresses_number >= max_addresses_number {
            return MmError::err(GetNewAddressRpcError::AddressLimitReached { max_addresses_number });
        }

        let address_scanner = coin.produce_hd_address_scanner().await?;

        // Address IDs start from 0, so the `last_known_address_id = known_addresses_number - 1`.
        // At this point we are sure that `known_addresses_number > 0`.
        let last_address_id = known_addresses_number - 1;

        for address_id in (0..=last_address_id).rev() {
            let HDAddress { address, .. } = coin.derive_address(hd_account, chain, address_id).await?;
            if address_scanner.is_address_used(&address).await? {
                return Ok(());
            }

            let empty_addresses_number = last_address_id - address_id + 1;
            if empty_addresses_number >= gap_limit {
                // We already have `gap_limit` empty addresses.
                return MmError::err(GetNewAddressRpcError::EmptyAddressesLimitReached { gap_limit });
            }
        }

        Ok(())
    }
}

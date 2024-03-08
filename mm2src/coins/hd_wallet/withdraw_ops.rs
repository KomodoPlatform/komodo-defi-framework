use super::{HDAccountAddressId, HDWalletOps, HDWithdrawError};
use crate::hd_wallet::{HDAccountOps, HDAddressOps, HDCoinAddress, HDCoinPubKey, HDWalletCoinOps};
use async_trait::async_trait;
use bip32::DerivationPath;
use crypto::{StandardHDPath, StandardHDPathError};
use mm2_err_handle::prelude::*;
use std::str::FromStr;

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum WithdrawFrom {
    AddressId(HDAccountAddressId),
    /// Don't use `Bip44DerivationPath` or `RpcDerivationPath` because if there is an error in the path,
    /// `serde::Deserialize` returns "data did not match any variant of untagged enum WithdrawFrom".
    /// It's better to show the user an informative error.
    DerivationPath {
        derivation_path: String,
    },
}

impl WithdrawFrom {
    #[allow(clippy::result_large_err)]
    pub fn to_address_path(&self, expected_coin_type: u32) -> MmResult<HDAccountAddressId, HDWithdrawError> {
        match self {
            WithdrawFrom::AddressId(address_id) => Ok(*address_id),
            WithdrawFrom::DerivationPath { derivation_path } => {
                let derivation_path = StandardHDPath::from_str(derivation_path)
                    .map_to_mm(StandardHDPathError::from)
                    .mm_err(|e| HDWithdrawError::UnexpectedFromAddress(e.to_string()))?;
                let coin_type = derivation_path.coin_type();
                if coin_type != expected_coin_type {
                    let error = format!(
                        "Derivation path '{}' must have '{}' coin type",
                        derivation_path, expected_coin_type
                    );
                    return MmError::err(HDWithdrawError::UnexpectedFromAddress(error));
                }
                Ok(HDAccountAddressId::from(derivation_path))
            },
        }
    }
}

pub struct WithdrawSenderAddress<Address, Pubkey> {
    pub(crate) address: Address,
    pub(crate) pubkey: Pubkey,
    pub(crate) derivation_path: Option<DerivationPath>,
}

/// `HDCoinWithdrawOps`: Operations that should be implemented for coins to support withdraw from HD wallets.
#[async_trait]
pub trait HDCoinWithdrawOps: HDWalletCoinOps {
    /// Fetches the sender address for a withdraw operation.
    ///
    /// # Parameters
    ///
    /// * `hd_wallet`: The HD wallet from which the withdraw is being made.
    /// * `from`: The address id or the derivation path of the sender address.
    ///
    /// # Returns
    ///
    /// A struct representing the sender address or an error if the address is not activated.
    async fn get_withdraw_hd_sender(
        &self,
        hd_wallet: &Self::HDWallet,
        from: &WithdrawFrom,
    ) -> MmResult<WithdrawSenderAddress<HDCoinAddress<Self>, HDCoinPubKey<Self>>, HDWithdrawError> {
        let HDAccountAddressId {
            account_id,
            chain,
            address_id,
        } = from.to_address_path(hd_wallet.coin_type())?;

        let hd_account = hd_wallet
            .get_account(account_id)
            .await
            .or_mm_err(|| HDWithdrawError::UnknownAccount { account_id })?;

        let is_address_activated = hd_account
            .is_address_activated(chain, address_id)
            // If [`HDWalletCoinOps::derive_address`] succeeds, [`HDAccountOps::is_address_activated`] shouldn't fails with an `InvalidBip44ChainError`.
            .mm_err(|e| HDWithdrawError::InternalError(e.to_string()))?;

        let hd_address = self.derive_address(&hd_account, chain, address_id).await?;
        let address = hd_address.address();
        if !is_address_activated {
            let error = format!("'{}' address is not activated", address);
            return MmError::err(HDWithdrawError::UnexpectedFromAddress(error));
        }

        Ok(WithdrawSenderAddress {
            address,
            pubkey: hd_address.pubkey(),
            derivation_path: Some(hd_address.derivation_path().clone()),
        })
    }
}

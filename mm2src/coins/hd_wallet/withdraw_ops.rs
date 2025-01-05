use super::{HDPathAccountToAddressId, HDWalletOps, HDWithdrawError};
use crate::hd_wallet::{HDAccountOps, HDAddressOps, HDCoinAddress, HDWalletCoinOps};
use async_trait::async_trait;
use bip32::DerivationPath;
use crypto::{StandardHDPath, StandardHDPathError};
use mm2_err_handle::prelude::*;
use std::str::FromStr;

type HDCoinPubKey<T> =
    <<<<T as HDWalletCoinOps>::HDWallet as HDWalletOps>::HDAccount as HDAccountOps>::HDAddress as HDAddressOps>::Pubkey;

/// Represents the source of the funds for a withdrawal operation.
#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum WithdrawFrom {
    /// The address id of the sender address which is specified by the account id, chain, and address id.
    AddressId(HDPathAccountToAddressId),
    /// The derivation path of the sender address in the BIP-44 format.
    ///
    /// IMPORTANT: Don't use `Bip44DerivationPath` or `RpcDerivationPath` because if there is an error in the path,
    /// `serde::Deserialize` returns "data did not match any variant of untagged enum WithdrawFrom".
    /// It's better to show the user an informative error.
    DerivationPath { derivation_path: String },
}

impl WithdrawFrom {
    #[allow(clippy::result_large_err)]
    pub fn to_address_path(&self, expected_coin_type: u32) -> MmResult<HDPathAccountToAddressId, HDWithdrawError> {
        match self {
            WithdrawFrom::AddressId(address_id) => Ok(*address_id),
            WithdrawFrom::DerivationPath { derivation_path } => {
                // TODO: Why are we reading full derivation path then trimming it to `HDPathAccountToAddressId`?!
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
                Ok(HDPathAccountToAddressId::from(derivation_path))
            },
        }
    }
}

/// Contains the details of the sender address for a withdraw operation.
pub struct WithdrawSenderAddress<Address, Pubkey> {
    pub(crate) address: Address,
    pub(crate) pubkey: Pubkey,
    pub(crate) derivation_path: Option<DerivationPath>,
}

/// `HDCoinWithdrawOps`: Operations that should be implemented for coins to support withdraw from HD wallets.
#[async_trait]
pub trait HDCoinWithdrawOps: HDWalletCoinOps {
    /// Fetches the sender address for a withdraw operation.
    /// This is the address from which the funds will be withdrawn.
    async fn get_withdraw_hd_sender(
        &self,
        hd_wallet: &Self::HDWallet,
        from: &WithdrawFrom,
    ) -> MmResult<WithdrawSenderAddress<HDCoinAddress<Self>, HDCoinPubKey<Self>>, HDWithdrawError> {
        let HDPathAccountToAddressId {
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

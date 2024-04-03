use super::*;
use crate::coin_balance::HDAddressBalanceScanner;
use crate::hd_wallet::{ExtractExtendedPubkey, HDAccount, HDAddress, HDExtractPubkeyError, HDWallet, HDXPubExtractor,
                       TrezorCoinError};
use async_trait::async_trait;
use bip32::DerivationPath;
use crypto::Secp256k1ExtendedPublicKey;
use ethereum_types::{Address, Public};
use std::iter;

pub type EthHDAddress = HDAddress<Address, Public>;
pub type EthHDAccount = HDAccount<EthHDAddress>;
pub type EthHDWallet = HDWallet<EthHDAccount>;

#[async_trait]
impl ExtractExtendedPubkey for EthCoin {
    type ExtendedPublicKey = Secp256k1ExtendedPublicKey;

    async fn extract_extended_pubkey<XPubExtractor>(
        &self,
        xpub_extractor: Option<XPubExtractor>,
        derivation_path: DerivationPath,
    ) -> MmResult<Self::ExtendedPublicKey, HDExtractPubkeyError>
    where
        XPubExtractor: HDXPubExtractor + Send,
    {
        extract_extended_pubkey_impl(self, xpub_extractor, derivation_path).await
    }
}

#[async_trait]
impl HDWalletCoinOps for EthCoin {
    type HDWallet = EthHDWallet;

    fn address_from_extended_pubkey(
        &self,
        extended_pubkey: &Secp256k1ExtendedPublicKey,
        derivation_path: DerivationPath,
    ) -> HDCoinHDAddress<Self> {
        let pubkey = pubkey_from_extended(extended_pubkey);
        let address = public_to_address(&pubkey);
        EthHDAddress {
            address,
            pubkey,
            derivation_path,
        }
    }

    fn trezor_coin(&self) -> MmResult<String, TrezorCoinError> {
        self.trezor_coin.clone().or_mm_err(|| {
            let ticker = self.ticker();
            let error = format!("'{ticker}' coin has 'trezor_coin' field as `None` in the coins config");
            TrezorCoinError::Internal(error)
        })
    }
}

impl HDCoinWithdrawOps for EthCoin {}

#[async_trait]
#[cfg_attr(test, mockable)]
impl HDAddressBalanceScanner for EthCoin {
    type Address = Address;

    async fn is_address_used(&self, address: &Self::Address) -> BalanceResult<bool> {
        // Count calculates the number of transactions sent from the address whether it's for ERC20 or ETH.
        // If the count is greater than 0, then the address is used.
        // If the count is 0, then we check for the balance of the address to make sure there was no received transactions.
        let count = self.transaction_count(*address, None).await?;
        if count > U256::zero() {
            return Ok(true);
        }

        // We check for platform balance only first to reduce the number of requests to the node.
        // If this is a token added using init_token, then we check for this token balance only, and
        // we don't check for platform balance or other tokens that was added before.
        let platform_balance = self.address_balance(*address).compat().await?;
        if !platform_balance.is_zero() {
            return Ok(true);
        }

        // This is done concurrently which increases the cost of the requests to the node. but it's better than doing it sequentially to reduce the time.
        let token_balance_map = self.get_tokens_balance_list_for_address(*address).await?;
        Ok(token_balance_map.values().any(|balance| !balance.get_total().is_zero()))
    }
}

#[async_trait]
impl HDWalletBalanceOps for EthCoin {
    type HDAddressScanner = Self;
    type BalanceObject = CoinBalanceMap;

    async fn produce_hd_address_scanner(&self) -> BalanceResult<Self::HDAddressScanner> { Ok(self.clone()) }

    async fn enable_hd_wallet<XPubExtractor>(
        &self,
        hd_wallet: &Self::HDWallet,
        xpub_extractor: Option<XPubExtractor>,
        params: EnabledCoinBalanceParams,
        path_to_address: &HDAccountAddressId,
    ) -> MmResult<HDWalletBalance<Self::BalanceObject>, EnableCoinBalanceError>
    where
        XPubExtractor: HDXPubExtractor + Send,
    {
        coin_balance::common_impl::enable_hd_wallet(self, hd_wallet, xpub_extractor, params, path_to_address).await
    }

    async fn scan_for_new_addresses(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut HDCoinHDAccount<Self>,
        address_scanner: &Self::HDAddressScanner,
        gap_limit: u32,
    ) -> BalanceResult<Vec<HDAddressBalance<Self::BalanceObject>>> {
        scan_for_new_addresses_impl(
            self,
            hd_wallet,
            hd_account,
            address_scanner,
            Bip44Chain::External,
            gap_limit,
        )
        .await
    }

    async fn all_known_addresses_balances(
        &self,
        hd_account: &HDCoinHDAccount<Self>,
    ) -> BalanceResult<Vec<HDAddressBalance<Self::BalanceObject>>> {
        let external_addresses = hd_account
            .known_addresses_number(Bip44Chain::External)
            // A UTXO coin should support both [`Bip44Chain::External`] and [`Bip44Chain::Internal`].
            .mm_err(|e| BalanceError::Internal(e.to_string()))?;

        self.known_addresses_balances_with_ids(hd_account, Bip44Chain::External, 0..external_addresses)
            .await
    }

    async fn known_address_balance(&self, address: &HDBalanceAddress<Self>) -> BalanceResult<Self::BalanceObject> {
        let balance = self
            .address_balance(*address)
            .and_then(move |result| Ok(u256_to_big_decimal(result, self.decimals())?))
            .compat()
            .await?;

        let coin_balance = CoinBalance {
            spendable: balance,
            unspendable: BigDecimal::from(0),
        };

        let mut balances = CoinBalanceMap::new();
        balances.insert(self.ticker().to_string(), coin_balance);
        let token_balances = self.get_tokens_balance_list_for_address(*address).await?;
        balances.extend(token_balances);
        Ok(balances)
    }

    async fn known_addresses_balances(
        &self,
        addresses: Vec<HDBalanceAddress<Self>>,
    ) -> BalanceResult<Vec<(HDBalanceAddress<Self>, Self::BalanceObject)>> {
        let mut balance_futs = Vec::new();
        for address in addresses {
            let fut = async move {
                let balance = self.known_address_balance(&address).await?;
                Ok((address, balance))
            };
            balance_futs.push(fut);
        }
        try_join_all(balance_futs).await
    }

    async fn prepare_addresses_for_balance_stream_if_enabled(
        &self,
        _addresses: HashSet<String>,
    ) -> MmResult<(), String> {
        Ok(())
    }
}

impl EthCoin {
    // Todo: make a common implementation for all coins
    /// Returns the addresses that are known to the wallet.
    pub async fn my_addresses(&self) -> MmResult<HashSet<Address>, AddressDerivingError> {
        match self.derivation_method() {
            DerivationMethod::SingleAddress(ref my_address) => Ok(iter::once(*my_address).collect()),
            DerivationMethod::HDWallet(ref hd_wallet) => {
                let hd_accounts = hd_wallet.get_accounts().await;

                let mut all_addresses = HashSet::new();
                for (_, hd_account) in hd_accounts {
                    let external_addresses = self.derive_known_addresses(&hd_account, Bip44Chain::External).await?;
                    let internal_addresses = self.derive_known_addresses(&hd_account, Bip44Chain::Internal).await?;

                    let addresses_it = external_addresses
                        .into_iter()
                        .chain(internal_addresses)
                        .map(|hd_address| hd_address.address());
                    all_addresses.extend(addresses_it);
                }

                Ok(all_addresses)
            },
        }
    }
}

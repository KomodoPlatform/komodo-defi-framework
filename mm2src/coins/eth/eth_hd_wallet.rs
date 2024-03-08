use super::*;
use crate::coin_balance::HDAddressBalanceScanner;
use crate::hd_wallet::{ExtractExtendedPubkey, HDAccount, HDAddress, HDExtractPubkeyError, HDWallet, HDXPubExtractor,
                       TrezorCoinError};
use async_trait::async_trait;
use bip32::DerivationPath;
use crypto::Secp256k1ExtendedPublicKey;
use ethereum_types::{Address, Public};

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
        extract_extended_pubkey(self, xpub_extractor, derivation_path).await
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

/// The ETH/ERC20 address balance scanner.
pub enum ETHAddressScanner {
    Web3 {
        web3_instances: AsyncMutex<Vec<Web3Instance>>,
        coin_type: EthCoinType,
    },
}

#[async_trait]
#[cfg_attr(test, mockable)]
impl HDAddressBalanceScanner for ETHAddressScanner {
    type Address = Address;

    async fn is_address_used(&self, _address: &Self::Address) -> BalanceResult<bool> {
        // Todo: reimplement HDAddressBalanceScanner for coins
        Ok(false)

        // match self {
        //     ETHAddressScanner::Web3 { web3_instances, coin_type } => {
        //         let current_block = match web3.eth().block_number().await {
        //             Ok(block) => block,
        //             Err(e) => {
        //                 return Err(BalanceError::Transport(format!("Error {} on eth_block_number", e)).into());
        //             },
        //         };
        //
        //         let from_block = BlockNumber::Earliest;
        //         let to_block = BlockNumber::Number(current_block);
        //
        //         match coin_type {
        //             EthCoinType::Eth => {
        //                 // It makes sense to check transactions to the hd address first since an address
        //                 // should have incoming transactions before making any outgoing ones, so this will
        //                 // avoid an additional call in almost all cases
        //                 let to_traces = eth_traces(web3, vec![], vec![*address], from_block, to_block, Some(1)).await?;
        //
        //                 if !to_traces.is_empty() {
        //                     return Ok(true);
        //                 }
        //
        //                 let from_traces =
        //                     eth_traces(web3, vec![*address], vec![], from_block, to_block, Some(1)).await?;
        //
        //                 Ok(!from_traces.is_empty())
        //             },
        //             EthCoinType::Erc20 { token_addr, .. } => {
        //                 // It makes sense to check transactions to the hd address first since an address
        //                 // should have incoming transactions before making any outgoing ones, so this will
        //                 // avoid an additional call in almost all cases
        //                 let to_events = erc20_transfer_events(
        //                     web3,
        //                     *token_addr,
        //                     None,
        //                     Some(*address),
        //                     from_block,
        //                     to_block,
        //                     Some(1),
        //                 )
        //                 .await?;
        //
        //                 if !to_events.is_empty() {
        //                     return Ok(true);
        //                 }
        //
        //                 let from_events = erc20_transfer_events(
        //                     web3,
        //                     *token_addr,
        //                     Some(*address),
        //                     None,
        //                     from_block,
        //                     to_block,
        //                     Some(1),
        //                 )
        //                 .await?;
        //
        //                 Ok(!from_events.is_empty())
        //             },
        //         }
        //     },
        // }
    }
}

#[async_trait]
impl HDWalletBalanceOps for EthCoin {
    type HDAddressScanner = ETHAddressScanner;

    async fn produce_hd_address_scanner(&self) -> BalanceResult<Self::HDAddressScanner> {
        // Todo: once this https://github.com/KomodoPlatform/komodo-defi-framework/issues/2071 is done it will be a shared Arc
        let web3_instances: Vec<Web3Instance> = self.web3_instances.lock().await.clone();

        Ok(ETHAddressScanner::Web3 {
            // Todo: once this https://github.com/KomodoPlatform/komodo-defi-framework/issues/2071 is done it will be a shared Arc
            web3_instances: AsyncMutex::new(web3_instances),
            coin_type: self.coin_type.clone(),
        })
    }

    async fn enable_hd_wallet<XPubExtractor>(
        &self,
        hd_wallet: &Self::HDWallet,
        xpub_extractor: Option<XPubExtractor>,
        params: EnabledCoinBalanceParams,
        path_to_address: &HDAccountAddressId,
    ) -> MmResult<HDWalletBalance, EnableCoinBalanceError>
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
    ) -> BalanceResult<Vec<HDAddressBalance>> {
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
    ) -> BalanceResult<Vec<HDAddressBalance>> {
        let external_addresses = hd_account
            .known_addresses_number(Bip44Chain::External)
            // A UTXO coin should support both [`Bip44Chain::External`] and [`Bip44Chain::Internal`].
            .mm_err(|e| BalanceError::Internal(e.to_string()))?;

        self.known_addresses_balances_with_ids(hd_account, Bip44Chain::External, 0..external_addresses)
            .await
    }

    async fn known_address_balance(&self, address: &HDBalanceAddress<Self>) -> BalanceResult<CoinBalance> {
        let balance = self
            .address_balance(*address)
            .and_then(move |result| Ok(u256_to_big_decimal(result, self.decimals())?))
            .compat()
            .await?;

        Ok(CoinBalance {
            spendable: balance,
            unspendable: BigDecimal::from(0),
        })
    }

    async fn known_addresses_balances(
        &self,
        addresses: Vec<HDBalanceAddress<Self>>,
    ) -> BalanceResult<Vec<(HDBalanceAddress<Self>, CoinBalance)>> {
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
        todo!()
    }
}

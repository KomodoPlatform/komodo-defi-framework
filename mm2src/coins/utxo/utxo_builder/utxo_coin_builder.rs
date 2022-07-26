use crate::hd_wallet::{HDAccountsMap, HDAccountsMutex};
use crate::hd_wallet_storage::{HDWalletCoinStorage, HDWalletStorageError};
use crate::utxo::rpc_clients::{ElectrumClient, ElectrumClientImpl, ElectrumRpcRequest, EstimateFeeMethod,
                               UtxoRpcClientEnum};
use crate::utxo::tx_cache::{UtxoVerboseCacheOps, UtxoVerboseCacheShared};
use crate::utxo::utxo_block_header_storage::{BlockHeaderStorage, InitBlockHeaderStorageOps};
use crate::utxo::utxo_builder::utxo_conf_builder::{UtxoConfBuilder, UtxoConfError, UtxoConfResult};
use crate::utxo::{output_script, utxo_common, ElectrumBuilderArgs, ElectrumProtoVerifier, RecentlySpentOutPoints,
                  TxFee, UtxoCoinConf, UtxoCoinFields, UtxoHDAccount, UtxoHDWallet, UtxoRpcMode, DEFAULT_GAP_LIMIT,
                  UTXO_DUST_AMOUNT};
use crate::{BlockchainNetwork, CoinTransportMetrics, DerivationMethod, HistorySyncState, PrivKeyBuildPolicy,
            PrivKeyPolicy, RpcClientType, UtxoActivationParams};
use async_trait::async_trait;
use chain::TxHashAlgo;
use common::executor::{spawn, Timer};
use common::log::{error, info};
use common::small_rng;
use crypto::{Bip32DerPathError, Bip44DerPathError, Bip44PathToCoin, CryptoCtx, CryptoInitError, HwWalletType};
use derive_more::Display;
use futures::channel::mpsc;
use futures::compat::Future01CompatExt;
use futures::lock::Mutex as AsyncMutex;
use futures::StreamExt;
use keys::bytes::Bytes;
pub use keys::{Address, AddressFormat as UtxoAddressFormat, AddressHashEnum, KeyPair, Private, Public, Secret,
               Type as ScriptType};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use primitives::hash::H256;
use rand::seq::SliceRandom;
use serde_json::{self as json, Value as Json};
use std::sync::{Arc, Mutex, Weak};

cfg_native! {
    use crate::utxo::coin_daemon_data_dir;
    use crate::utxo::rpc_clients::{ConcurrentRequestMap, NativeClient, NativeClientImpl};
    use dirs::home_dir;
    use std::path::{Path, PathBuf};
}

pub type UtxoCoinBuildResult<T> = Result<T, MmError<UtxoCoinBuildError>>;

#[derive(Debug, Display)]
pub enum UtxoCoinBuildError {
    ConfError(UtxoConfError),
    #[display(fmt = "Native RPC client is only supported in native mode")]
    NativeRpcNotSupportedInWasm,
    ErrorReadingNativeModeConf(String),
    #[display(fmt = "Rpc port is not set neither in `coins` file nor in native daemon config")]
    RpcPortIsNotSet,
    ErrorDetectingFeeMethod(String),
    ErrorDetectingDecimals(String),
    InvalidBlockchainNetwork(String),
    #[display(
        fmt = "Failed to connect to at least 1 of {:?} in {} seconds.",
        electrum_servers,
        seconds
    )]
    FailedToConnectToElectrums {
        electrum_servers: Vec<ElectrumRpcRequest>,
        seconds: u64,
    },
    ElectrumProtocolVersionCheckError(String),
    #[display(fmt = "Can not detect the user home directory")]
    CantDetectUserHome,
    #[display(fmt = "Unexpected derivation method: {}", _0)]
    UnexpectedDerivationMethod(String),
    #[display(fmt = "Hardware Wallet context is not initialized")]
    HwContextNotInitialized,
    HDWalletStorageError(HDWalletStorageError),
    #[display(
        fmt = "Coin should be activated with Hardware Wallet. Please consider using `\"priv_key_policy\": \"Trezor\"` in the activation request"
    )]
    CoinShouldBeActivatedWithHw,
    #[display(
        fmt = "Coin doesn't support Trezor hardware wallet. Please consider adding the 'trezor_coin' field to the coins config"
    )]
    CoinDoesntSupportTrezor,
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<UtxoConfError> for UtxoCoinBuildError {
    fn from(e: UtxoConfError) -> Self { UtxoCoinBuildError::ConfError(e) }
}

impl From<CryptoInitError> for UtxoCoinBuildError {
    /// `CryptoCtx` is expected to be initialized already.
    fn from(crypto_err: CryptoInitError) -> Self { UtxoCoinBuildError::Internal(crypto_err.to_string()) }
}

impl From<Bip32DerPathError> for UtxoCoinBuildError {
    fn from(e: Bip32DerPathError) -> Self { UtxoCoinBuildError::Internal(Bip44DerPathError::from(e).to_string()) }
}

impl From<HDWalletStorageError> for UtxoCoinBuildError {
    fn from(e: HDWalletStorageError) -> Self { UtxoCoinBuildError::HDWalletStorageError(e) }
}

#[async_trait]
pub trait UtxoCoinBuilder: UtxoFieldsWithIguanaPrivKeyBuilder + UtxoFieldsWithHardwareWalletBuilder {
    type ResultCoin;
    type Error: NotMmError;

    fn priv_key_policy(&self) -> PrivKeyBuildPolicy<'_>;

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error>;

    async fn build_utxo_fields(&self) -> UtxoCoinBuildResult<UtxoCoinFields> {
        match self.priv_key_policy() {
            PrivKeyBuildPolicy::IguanaPrivKey(priv_key) => self.build_utxo_fields_with_iguana_priv_key(priv_key).await,
            PrivKeyBuildPolicy::Trezor => self.build_utxo_fields_with_trezor().await,
        }
    }
}

#[async_trait]
pub trait UtxoCoinWithIguanaPrivKeyBuilder: UtxoFieldsWithIguanaPrivKeyBuilder {
    type ResultCoin;
    type Error: NotMmError;

    fn priv_key(&self) -> &[u8];

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error>;
}

#[async_trait]
pub trait UtxoFieldsWithIguanaPrivKeyBuilder: UtxoCoinBuilderCommonOps {
    async fn build_utxo_fields_with_iguana_priv_key(&self, priv_key: &[u8]) -> UtxoCoinBuildResult<UtxoCoinFields> {
        let conf = UtxoConfBuilder::new(self.conf(), self.activation_params(), self.ticker()).build()?;

        if self.is_hw_coin(&conf) {
            return MmError::err(UtxoCoinBuildError::CoinShouldBeActivatedWithHw);
        }

        let private = Private {
            prefix: conf.wif_prefix,
            secret: H256::from(priv_key),
            compressed: true,
            checksum_type: conf.checksum_type,
        };
        let key_pair = KeyPair::from_private(private).map_to_mm(|e| UtxoCoinBuildError::Internal(e.to_string()))?;
        let addr_format = self.address_format()?;
        let my_address = Address {
            prefix: conf.pub_addr_prefix,
            t_addr_prefix: conf.pub_t_addr_prefix,
            hash: AddressHashEnum::AddressHash(key_pair.public().address_hash()),
            checksum_type: conf.checksum_type,
            hrp: conf.bech32_hrp.clone(),
            addr_format,
        };

        let my_script_pubkey = output_script(&my_address, ScriptType::P2PKH).to_bytes();
        let derivation_method = DerivationMethod::Iguana(my_address);
        let priv_key_policy = PrivKeyPolicy::KeyPair(key_pair);

        let rpc_client = self.rpc_client().await?;
        let tx_fee = self.tx_fee(&rpc_client).await?;
        let decimals = self.decimals(&rpc_client).await?;
        let dust_amount = self.dust_amount();

        let initial_history_state = self.initial_history_state();
        let tx_hash_algo = self.tx_hash_algo();
        let check_utxo_maturity = self.check_utxo_maturity();
        let tx_cache = self.tx_cache();
        let block_headers_storage = self.block_headers_storage()?;

        let coin = UtxoCoinFields {
            conf,
            decimals,
            dust_amount,
            rpc_client,
            priv_key_policy,
            derivation_method,
            history_sync_state: Mutex::new(initial_history_state),
            tx_cache,
            block_headers_storage,
            recently_spent_outpoints: AsyncMutex::new(RecentlySpentOutPoints::new(my_script_pubkey)),
            tx_fee,
            tx_hash_algo,
            check_utxo_maturity,
        };
        Ok(coin)
    }
}

#[async_trait]
pub trait UtxoFieldsWithHardwareWalletBuilder: UtxoCoinBuilderCommonOps {
    async fn build_utxo_fields_with_trezor(&self) -> UtxoCoinBuildResult<UtxoCoinFields> {
        let ticker = self.ticker().to_owned();
        let conf = UtxoConfBuilder::new(self.conf(), self.activation_params(), &ticker).build()?;

        if !self.supports_trezor(&conf) {
            return MmError::err(UtxoCoinBuildError::CoinDoesntSupportTrezor);
        }
        self.check_if_trezor_is_initialized()?;

        // For now, use a default script pubkey.
        // TODO change the type of `recently_spent_outpoints` to `AsyncMutex<HashMap<Bytes, RecentlySpentOutPoints>>`
        let my_script_pubkey = Bytes::new();
        let recently_spent_outpoints = AsyncMutex::new(RecentlySpentOutPoints::new(my_script_pubkey));

        let address_format = self.address_format()?;
        let derivation_path = self.derivation_path()?;

        let hd_wallet_storage = HDWalletCoinStorage::init(self.ctx(), ticker).await?;

        let accounts = self
            .load_hd_wallet_accounts(&hd_wallet_storage, &derivation_path)
            .await?;
        let gap_limit = self.gap_limit();
        let hd_wallet = UtxoHDWallet {
            hd_wallet_storage,
            address_format,
            derivation_path,
            accounts: HDAccountsMutex::new(accounts),
            gap_limit,
        };

        let rpc_client = self.rpc_client().await?;
        let tx_fee = self.tx_fee(&rpc_client).await?;
        let decimals = self.decimals(&rpc_client).await?;
        let dust_amount = self.dust_amount();

        let initial_history_state = self.initial_history_state();
        let tx_hash_algo = self.tx_hash_algo();
        let check_utxo_maturity = self.check_utxo_maturity();
        let tx_cache = self.tx_cache();
        let block_headers_storage = self.block_headers_storage()?;

        let coin = UtxoCoinFields {
            conf,
            decimals,
            dust_amount,
            rpc_client,
            priv_key_policy: PrivKeyPolicy::Trezor,
            derivation_method: DerivationMethod::HDWallet(hd_wallet),
            history_sync_state: Mutex::new(initial_history_state),
            block_headers_storage,
            tx_cache,
            recently_spent_outpoints,
            tx_fee,
            tx_hash_algo,
            check_utxo_maturity,
        };
        Ok(coin)
    }

    async fn load_hd_wallet_accounts(
        &self,
        hd_wallet_storage: &HDWalletCoinStorage,
        derivation_path: &Bip44PathToCoin,
    ) -> UtxoCoinBuildResult<HDAccountsMap<UtxoHDAccount>> {
        utxo_common::load_hd_accounts_from_storage(hd_wallet_storage, derivation_path)
            .await
            .mm_err(UtxoCoinBuildError::from)
    }

    fn derivation_path(&self) -> UtxoConfResult<Bip44PathToCoin> {
        if self.conf()["derivation_path"].is_null() {
            return MmError::err(UtxoConfError::DerivationPathIsNotSet);
        }
        json::from_value(self.conf()["derivation_path"].clone())
            .map_to_mm(|e| UtxoConfError::ErrorDeserializingDerivationPath(e.to_string()))
    }

    fn gap_limit(&self) -> u32 { self.activation_params().gap_limit.unwrap_or(DEFAULT_GAP_LIMIT) }

    fn supports_trezor(&self, conf: &UtxoCoinConf) -> bool { conf.trezor_coin.is_some() }

    fn check_if_trezor_is_initialized(&self) -> UtxoCoinBuildResult<()> {
        let crypto_ctx = CryptoCtx::from_ctx(self.ctx())?;
        let hw_ctx = crypto_ctx
            .hw_ctx()
            .or_mm_err(|| UtxoCoinBuildError::HwContextNotInitialized)?;
        match hw_ctx.hw_wallet_type() {
            HwWalletType::Trezor => Ok(()),
        }
    }
}

#[async_trait]
pub trait UtxoCoinBuilderCommonOps {
    fn ctx(&self) -> &MmArc;

    fn conf(&self) -> &Json;

    fn activation_params(&self) -> &UtxoActivationParams;

    fn ticker(&self) -> &str;

    fn block_headers_storage(&self) -> UtxoCoinBuildResult<Option<BlockHeaderStorage>> {
        let params: Option<_> = json::from_value(self.conf()["block_header_params"].clone())
            .map_to_mm(|e| UtxoConfError::InvalidBlockHeaderParams(e.to_string()))?;
        match params {
            None => Ok(None),
            Some(params) => Ok(BlockHeaderStorage::new_from_ctx(self.ctx().clone(), params)),
        }
    }

    fn address_format(&self) -> UtxoCoinBuildResult<UtxoAddressFormat> {
        let format_from_req = self.activation_params().address_format.clone();
        let format_from_conf = json::from_value::<Option<UtxoAddressFormat>>(self.conf()["address_format"].clone())
            .map_to_mm(|e| UtxoConfError::InvalidAddressFormat(e.to_string()))?
            .unwrap_or(UtxoAddressFormat::Standard);

        let mut address_format = match format_from_req {
            Some(from_req) => {
                if from_req.is_segwit() != format_from_conf.is_segwit() {
                    let error = format!(
                        "Both conf {:?} and request {:?} must be either Segwit or Standard/CashAddress",
                        format_from_conf, from_req
                    );
                    return MmError::err(UtxoCoinBuildError::from(UtxoConfError::InvalidAddressFormat(error)));
                } else {
                    from_req
                }
            },
            None => format_from_conf,
        };

        if let UtxoAddressFormat::CashAddress {
            network: _,
            ref mut pub_addr_prefix,
            ref mut p2sh_addr_prefix,
        } = address_format
        {
            *pub_addr_prefix = self.pub_addr_prefix();
            *p2sh_addr_prefix = self.p2sh_address_prefix();
        }

        let is_segwit_in_conf = self.conf()["segwit"].as_bool().unwrap_or(false);
        if address_format.is_segwit() && (!is_segwit_in_conf || self.conf()["bech32_hrp"].is_null()) {
            let error =
                "Cannot use Segwit address format for coin without segwit support or bech32_hrp in config".to_owned();
            return MmError::err(UtxoCoinBuildError::from(UtxoConfError::InvalidAddressFormat(error)));
        }
        Ok(address_format)
    }

    fn pub_addr_prefix(&self) -> u8 {
        let pubtype = self.conf()["pubtype"]
            .as_u64()
            .unwrap_or(if self.ticker() == "BTC" { 0 } else { 60 });
        pubtype as u8
    }

    fn p2sh_address_prefix(&self) -> u8 {
        self.conf()["p2shtype"]
            .as_u64()
            .unwrap_or(if self.ticker() == "BTC" { 5 } else { 85 }) as u8
    }

    fn dust_amount(&self) -> u64 { json::from_value(self.conf()["dust"].clone()).unwrap_or(UTXO_DUST_AMOUNT) }

    fn network(&self) -> UtxoCoinBuildResult<BlockchainNetwork> {
        let conf = self.conf();
        if !conf["network"].is_null() {
            return json::from_value(conf["network"].clone())
                .map_to_mm(|e| UtxoCoinBuildError::InvalidBlockchainNetwork(e.to_string()));
        }
        Ok(BlockchainNetwork::Mainnet)
    }

    async fn decimals(&self, _rpc_client: &UtxoRpcClientEnum) -> UtxoCoinBuildResult<u8> {
        Ok(self.conf()["decimals"].as_u64().unwrap_or(8) as u8)
    }

    async fn tx_fee(&self, rpc_client: &UtxoRpcClientEnum) -> UtxoCoinBuildResult<TxFee> {
        let tx_fee = match self.conf()["txfee"].as_u64() {
            None => TxFee::FixedPerKb(1000),
            Some(0) => {
                let fee_method = match &rpc_client {
                    UtxoRpcClientEnum::Electrum(_) => EstimateFeeMethod::Standard,
                    UtxoRpcClientEnum::Native(client) => client
                        .detect_fee_method()
                        .compat()
                        .await
                        .map_to_mm(UtxoCoinBuildError::ErrorDetectingFeeMethod)?,
                };
                TxFee::Dynamic(fee_method)
            },
            Some(fee) => TxFee::FixedPerKb(fee),
        };
        Ok(tx_fee)
    }

    fn initial_history_state(&self) -> HistorySyncState {
        if self.activation_params().tx_history {
            HistorySyncState::NotStarted
        } else {
            HistorySyncState::NotEnabled
        }
    }

    async fn rpc_client(&self) -> UtxoCoinBuildResult<UtxoRpcClientEnum> {
        match self.activation_params().mode.clone() {
            UtxoRpcMode::Native => {
                #[cfg(target_arch = "wasm32")]
                {
                    MmError::err(UtxoCoinBuildError::NativeRpcNotSupportedInWasm)
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let native = self.native_client()?;
                    Ok(UtxoRpcClientEnum::Native(native))
                }
            },
            UtxoRpcMode::Electrum { servers } => {
                let electrum = self.electrum_client(ElectrumBuilderArgs::default(), servers).await?;
                Ok(UtxoRpcClientEnum::Electrum(electrum))
            },
        }
    }

    async fn electrum_client(
        &self,
        args: ElectrumBuilderArgs,
        mut servers: Vec<ElectrumRpcRequest>,
    ) -> UtxoCoinBuildResult<ElectrumClient> {
        let (on_connect_tx, on_connect_rx) = mpsc::unbounded();
        let ticker = self.ticker().to_owned();
        let ctx = self.ctx();
        let mut event_handlers = vec![];
        if args.collect_metrics {
            event_handlers.push(
                CoinTransportMetrics::new(ctx.metrics.weak(), ticker.clone(), RpcClientType::Electrum).into_shared(),
            );
        }

        if args.negotiate_version {
            event_handlers.push(ElectrumProtoVerifier { on_connect_tx }.into_shared());
        }

        let mut rng = small_rng();
        servers.as_mut_slice().shuffle(&mut rng);
        let client = ElectrumClientImpl::new(ticker, event_handlers);
        for server in servers.iter() {
            match client.add_server(server).await {
                Ok(_) => (),
                Err(e) => error!("Error {:?} connecting to {:?}. Address won't be used", e, server),
            };
        }

        let mut attempts = 0i32;
        while !client.is_connected().await {
            if attempts >= 10 {
                return MmError::err(UtxoCoinBuildError::FailedToConnectToElectrums {
                    electrum_servers: servers.clone(),
                    seconds: 5,
                });
            }

            Timer::sleep(0.5).await;
            attempts += 1;
        }

        let client = Arc::new(client);

        if args.negotiate_version {
            let weak_client = Arc::downgrade(&client);
            let client_name = format!("{} GUI/MM2 {}", ctx.gui().unwrap_or("UNKNOWN"), ctx.mm_version());
            spawn_electrum_version_loop(weak_client, on_connect_rx, client_name);

            wait_for_protocol_version_checked(&client)
                .await
                .map_to_mm(UtxoCoinBuildError::ElectrumProtocolVersionCheckError)?;
        }

        if args.spawn_ping {
            let weak_client = Arc::downgrade(&client);
            spawn_electrum_ping_loop(weak_client, servers);
        }

        Ok(ElectrumClient(client))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn native_client(&self) -> UtxoCoinBuildResult<NativeClient> {
        use base64::{encode_config as base64_encode, URL_SAFE};

        let native_conf_path = self.confpath()?;
        let network = self.network()?;
        let (rpc_port, rpc_user, rpc_password) = read_native_mode_conf(&native_conf_path, &network)
            .map_to_mm(UtxoCoinBuildError::ErrorReadingNativeModeConf)?;
        let auth_str = format!("{}:{}", rpc_user, rpc_password);
        let rpc_port = match rpc_port {
            Some(p) => p,
            None => self.conf()["rpcport"]
                .as_u64()
                .or_mm_err(|| UtxoCoinBuildError::RpcPortIsNotSet)? as u16,
        };

        let ctx = self.ctx();
        let coin_ticker = self.ticker().to_owned();
        let event_handlers =
            vec![
                CoinTransportMetrics::new(ctx.metrics.weak(), coin_ticker.clone(), RpcClientType::Native).into_shared(),
            ];
        let client = Arc::new(NativeClientImpl {
            coin_ticker,
            uri: format!("http://127.0.0.1:{}", rpc_port),
            auth: format!("Basic {}", base64_encode(&auth_str, URL_SAFE)),
            event_handlers,
            request_id: 0u64.into(),
            list_unspent_concurrent_map: ConcurrentRequestMap::new(),
        });

        Ok(NativeClient(client))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn confpath(&self) -> UtxoCoinBuildResult<PathBuf> {
        let conf = self.conf();
        // Documented at https://github.com/jl777/coins#bitcoin-protocol-specific-json
        // "USERHOME/" prefix should be replaced with the user's home folder.
        let declared_confpath = match self.conf()["confpath"].as_str() {
            Some(path) if !path.is_empty() => path.trim(),
            _ => {
                let (name, is_asset_chain) = {
                    match conf["asset"].as_str() {
                        Some(a) => (a, true),
                        None => {
                            let name = conf["name"]
                                .as_str()
                                .or_mm_err(|| UtxoConfError::CurrencyNameIsNotSet)?;
                            (name, false)
                        },
                    }
                };
                let data_dir = coin_daemon_data_dir(name, is_asset_chain);
                let confname = format!("{}.conf", name);

                return Ok(data_dir.join(&confname[..]));
            },
        };

        let (confpath, rel_to_home) = match declared_confpath.strip_prefix("~/") {
            Some(stripped) => (stripped, true),
            None => match declared_confpath.strip_prefix("USERHOME/") {
                Some(stripped) => (stripped, true),
                None => (declared_confpath, false),
            },
        };

        if rel_to_home {
            let home = home_dir().or_mm_err(|| UtxoCoinBuildError::CantDetectUserHome)?;
            Ok(home.join(confpath))
        } else {
            Ok(confpath.into())
        }
    }

    fn tx_hash_algo(&self) -> TxHashAlgo {
        if self.ticker() == "GRS" {
            TxHashAlgo::SHA256
        } else {
            TxHashAlgo::DSHA256
        }
    }

    fn check_utxo_maturity(&self) -> bool {
        // First, check if the flag is set in the activation params.
        if let Some(check_utxo_maturity) = self.activation_params().check_utxo_maturity {
            return check_utxo_maturity;
        }
        self.conf()["check_utxo_maturity"].as_bool().unwrap_or_default()
    }

    fn is_hw_coin(&self, conf: &UtxoCoinConf) -> bool { conf.trezor_coin.is_some() }

    #[cfg(target_arch = "wasm32")]
    fn tx_cache(&self) -> UtxoVerboseCacheShared {
        crate::utxo::tx_cache::wasm_tx_cache::WasmVerboseCache::default().into_shared()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn tx_cache(&self) -> UtxoVerboseCacheShared {
        crate::utxo::tx_cache::fs_tx_cache::FsVerboseCache::new(self.ticker().to_owned(), self.tx_cache_path())
            .into_shared()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn tx_cache_path(&self) -> PathBuf { self.ctx().dbdir().join("TX_CACHE") }
}

/// Attempts to parse native daemon conf file and return rpcport, rpcuser and rpcpassword
#[cfg(not(target_arch = "wasm32"))]
fn read_native_mode_conf(
    filename: &dyn AsRef<Path>,
    network: &BlockchainNetwork,
) -> Result<(Option<u16>, String, String), String> {
    use ini::Ini;

    fn read_property<'a>(conf: &'a ini::Ini, network: &BlockchainNetwork, property: &str) -> Option<&'a String> {
        let subsection = match network {
            BlockchainNetwork::Mainnet => None,
            BlockchainNetwork::Testnet => conf.section(Some("test")),
            BlockchainNetwork::Regtest => conf.section(Some("regtest")),
        };
        subsection
            .and_then(|props| props.get(property))
            .or_else(|| conf.general_section().get(property))
    }

    let conf: Ini = match Ini::load_from_file(&filename) {
        Ok(ini) => ini,
        Err(err) => {
            return ERR!(
                "Error parsing the native wallet configuration '{}': {}",
                filename.as_ref().display(),
                err
            )
        },
    };
    let rpc_port = match read_property(&conf, network, "rpcport") {
        Some(port) => port.parse::<u16>().ok(),
        None => None,
    };
    let rpc_user = try_s!(read_property(&conf, network, "rpcuser").ok_or(ERRL!(
        "Conf file {} doesn't have the rpcuser key",
        filename.as_ref().display()
    )));
    let rpc_password = try_s!(read_property(&conf, network, "rpcpassword").ok_or(ERRL!(
        "Conf file {} doesn't have the rpcpassword key",
        filename.as_ref().display()
    )));
    Ok((rpc_port, rpc_user.clone(), rpc_password.clone()))
}

/// Ping the electrum servers every 30 seconds to prevent them from disconnecting us.
/// According to docs server can do it if there are no messages in ~10 minutes.
/// https://electrumx.readthedocs.io/en/latest/protocol-methods.html?highlight=keep#server-ping
/// Weak reference will allow to stop the thread if client is dropped.
fn spawn_electrum_ping_loop(weak_client: Weak<ElectrumClientImpl>, servers: Vec<ElectrumRpcRequest>) {
    spawn(async move {
        loop {
            if let Some(client) = weak_client.upgrade() {
                if let Err(e) = ElectrumClient(client).server_ping().compat().await {
                    error!("Electrum servers {:?} ping error: {}", servers, e);
                }
            } else {
                info!("Electrum servers {:?} ping loop stopped", servers);
                break;
            }
            Timer::sleep(30.).await
        }
    });
}

/// Follow the `on_connect_rx` stream and verify the protocol version of each connected electrum server.
/// https://electrumx.readthedocs.io/en/latest/protocol-methods.html?highlight=keep#server-version
/// Weak reference will allow to stop the thread if client is dropped.
fn spawn_electrum_version_loop(
    weak_client: Weak<ElectrumClientImpl>,
    mut on_connect_rx: mpsc::UnboundedReceiver<String>,
    client_name: String,
) {
    spawn(async move {
        while let Some(electrum_addr) = on_connect_rx.next().await {
            spawn(check_electrum_server_version(
                weak_client.clone(),
                client_name.clone(),
                electrum_addr,
            ));
        }

        info!("Electrum server.version loop stopped");
    });
}

async fn check_electrum_server_version(
    weak_client: Weak<ElectrumClientImpl>,
    client_name: String,
    electrum_addr: String,
) {
    // client.remove_server() is called too often
    async fn remove_server(client: ElectrumClient, electrum_addr: &str) {
        if let Err(e) = client.remove_server(electrum_addr).await {
            error!("Error on remove server: {}", e);
        }
    }

    if let Some(c) = weak_client.upgrade() {
        let client = ElectrumClient(c);
        let available_protocols = client.protocol_version();
        let version = match client
            .server_version(&electrum_addr, &client_name, available_protocols)
            .compat()
            .await
        {
            Ok(version) => version,
            Err(e) => {
                error!("Electrum {} server.version error: {:?}", electrum_addr, e);
                if !e.error.is_transport() {
                    remove_server(client, &electrum_addr).await;
                };
                return;
            },
        };

        // check if the version is allowed
        let actual_version = match version.protocol_version.parse::<f32>() {
            Ok(v) => v,
            Err(e) => {
                error!("Error on parse protocol_version: {:?}", e);
                remove_server(client, &electrum_addr).await;
                return;
            },
        };

        if !available_protocols.contains(&actual_version) {
            error!(
                "Received unsupported protocol version {:?} from {:?}. Remove the connection",
                actual_version, electrum_addr
            );
            remove_server(client, &electrum_addr).await;
            return;
        }

        match client.set_protocol_version(&electrum_addr, actual_version).await {
            Ok(()) => info!(
                "Use protocol version {:?} for Electrum {:?}",
                actual_version, electrum_addr
            ),
            Err(e) => error!("Error on set protocol_version: {}", e),
        };
    }
}

/// Wait until the protocol version of at least one client's Electrum is checked.
async fn wait_for_protocol_version_checked(client: &ElectrumClientImpl) -> Result<(), String> {
    let mut attempts = 0;
    loop {
        if attempts >= 10 {
            return ERR!("Failed protocol version verifying of at least 1 of Electrums in 5 seconds.");
        }

        if client.count_connections().await == 0 {
            // All of the connections were removed because of server.version checking
            return ERR!(
                "There are no Electrums with the required protocol version {:?}",
                client.protocol_version()
            );
        }

        if client.is_protocol_version_checked().await {
            break;
        }

        Timer::sleep(0.5).await;
        attempts += 1;
    }

    Ok(())
}

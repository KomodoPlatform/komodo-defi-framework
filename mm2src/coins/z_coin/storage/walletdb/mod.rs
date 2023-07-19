use crate::z_coin::z_rpc::ZRpcOps;
use crate::z_coin::{LightWalletSyncParams, ZCoinBuilder, ZcoinClientInitError};
use common::log::info;
use common::now_sec;
use mm2_err_handle::prelude::*;

cfg_native!(
    use crate::z_coin::{extended_spending_key_from_protocol_info_and_policy, ZcoinConsensusParams};
    use crate::z_coin::z_rpc::create_wallet_db;

    use parking_lot::Mutex;
    use std::sync::Arc;
    use zcash_client_sqlite::WalletDb;
    use zcash_primitives::zip32::ExtendedFullViewingKey;
);

cfg_wasm32!(
    mod wallet_idb;
    use wallet_idb::WalletDbInner;
);

#[derive(Debug, Display)]
pub enum WalletDbError {
    ZcoinClientInitError(ZcoinClientInitError),
    ZCoinBuildError(String),
    IndexedDBError(String),
    GrpcError(String),
}

impl From<tonic::Status> for WalletDbError {
    fn from(err: tonic::Status) -> Self { WalletDbError::GrpcError(err.to_string()) }
}

#[derive(Clone)]
pub struct WalletDbShared {
    #[cfg(not(target_arch = "wasm32"))]
    pub db: Arc<Mutex<WalletDb<ZcoinConsensusParams>>>,
    #[cfg(target_arch = "wasm32")]
    pub db: SharedDb<WalletDbInner>,
    #[allow(unused)]
    ticker: String,
}

fn calculate_starting_height_from_date(date: u64, current_block_height: u64) -> Result<u32, String> {
    let blocks_prod_per_day = 24 * 60;
    let current_time_s = now_sec();

    if current_time_s > date {
        return Err("sync_starting_date must be earlier then current date".to_string());
    };

    let secs_since_date = current_time_s - date;
    let days_since_date = (secs_since_date / 86400) - 1;
    let blocks_to_sync = (days_since_date * blocks_prod_per_day) + blocks_prod_per_day;
    let block_to_sync_from = current_block_height - blocks_to_sync;

    Ok(block_to_sync_from as u32)
}

#[cfg(not(target_arch = "wasm32"))]
impl<'a> WalletDbShared {
    pub async fn new(
        zcoin_builder: &ZCoinBuilder<'a>,
        rpc: &mut impl ZRpcOps,
        starting_param: &Option<LightWalletSyncParams>,
    ) -> MmResult<Self, WalletDbError> {
        let z_spending_key = match zcoin_builder.z_spending_key {
            Some(ref z_spending_key) => z_spending_key.clone(),
            None => extended_spending_key_from_protocol_info_and_policy(
                &zcoin_builder.protocol_info,
                &zcoin_builder.priv_key_policy,
            )
            .mm_err(|err| WalletDbError::ZCoinBuildError(err.to_string()))?,
        };

        let sync_height = match starting_param {
            Some(params) => {
                if let Some(date) = params.date {
                    let current_block_height = rpc
                        .get_block_height()
                        .await
                        .map_err(|err| WalletDbError::GrpcError(err.to_string()))?;

                    let starting_height = calculate_starting_height_from_date(date as u64, current_block_height)
                        .map_err(|err| WalletDbError::ZcoinClientInitError(ZcoinClientInitError::ZcashDBError(err)))?;

                    info!(
                        "Found date in sync params for {}. Walletdb will be built using block height {} as the \
                        starting \
                        block for scanning and updating walletdb",
                        zcoin_builder.ticker, starting_height
                    );
                    Some(starting_height)
                } else {
                    if params.height.is_some() {
                        info!(
                            "Found height in sync params for {}. Walletdb will be built using block height {:?} as the \
                            starting block for scanning and updating walletdb",
                            zcoin_builder.ticker, params.height
                        );
                    }
                    params.height
                }
            },
            None => None,
        };

        let sync_block = match sync_height {
            Some(height) => Some(
                rpc.get_tree_state(height)
                    .await
                    .map_err(|err| WalletDbError::GrpcError(err.to_string()))?,
            ),
            None => None,
        };

        let wallet_db = create_wallet_db(
            zcoin_builder
                .db_dir_path
                .join(format!("{}_wallet.db", zcoin_builder.ticker)),
            zcoin_builder.protocol_info.consensus_params.clone(),
            sync_block,
            ExtendedFullViewingKey::from(&z_spending_key),
        )
        .await
        .map_err(|err| MmError::new(WalletDbError::ZcoinClientInitError(err.into_inner())))?;

        Ok(Self {
            db: Arc::new(Mutex::new(wallet_db)),
            ticker: zcoin_builder.ticker.to_string(),
        })
    }
}

cfg_wasm32!(
    use mm2_db::indexed_db::{ConstructibleDb, DbLocked, SharedDb};

    pub type WalletDbRes<T> = MmResult<T, WalletDbError>;
    pub type WalletDbInnerLocked<'a> = DbLocked<'a, WalletDbInner>;

    impl<'a> WalletDbShared {
        pub async fn new(zcoin_builder: &ZCoinBuilder<'a>) -> MmResult<Self, WalletDbError> {
            Ok(Self {
                db: ConstructibleDb::new(zcoin_builder.ctx).into_shared(),
                ticker: zcoin_builder.ticker.to_string(),
            })
        }

        #[allow(unused)]
        async fn lock_db(&self) -> WalletDbRes<WalletDbInnerLocked<'_>> {
            self.db
                .get_or_initialize()
                .await
                .mm_err(|err| WalletDbError::IndexedDBError(err.to_string()))
        }
    }
);

use crate::utxo::utxo_builder::UtxoCoinBuildError;
use crate::z_coin::{ZCoinBuilder, ZcoinClientInitError};
use mm2_err_handle::prelude::*;

cfg_native!(
    use crate::utxo::utxo_builder::{UtxoCoinBuilderCommonOps, DAY_IN_SECONDS};
    use crate::z_coin::{CheckPointBlockInfo, extended_spending_key_from_protocol_info_and_policy, LightClientSyncParams,
                    ZcoinConsensusParams, ZcoinRpcMode};
    use crate::z_coin::z_rpc::{create_wallet_db, ZRpcOps};

    use common::now_sec;
    use hex::{FromHex, FromHexError};
    use parking_lot::Mutex;
    use rpc::v1::types::{Bytes, H256};
    use std::str::FromStr;
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
    DecodeError(String),
    UtxoCoinBuildError(UtxoCoinBuildError),
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
async fn checkpoint_block_from_height(
    height: u64,
    rpc: &mut impl ZRpcOps,
) -> Result<Option<CheckPointBlockInfo>, WalletDbError> {
    let tree_state = rpc
        .get_tree_state(height)
        .await
        .map_err(|err| WalletDbError::GrpcError(err.to_string()))?;
    let hash = H256::from_str(&tree_state.hash)
        .map_err(|err| WalletDbError::DecodeError(err.to_string()))?
        .reversed();
    let sapling_tree = Bytes::new(
        FromHex::from_hex(&tree_state.tree).map_err(|err: FromHexError| WalletDbError::DecodeError(err.to_string()))?,
    );

    Ok(Some(CheckPointBlockInfo {
        height: tree_state.height as u32,
        hash,
        time: tree_state.time,
        sapling_tree,
    }))
}

#[cfg(not(target_arch = "wasm32"))]
impl<'a> WalletDbShared {
    pub async fn new(zcoin_builder: &ZCoinBuilder<'a>, rpc: &mut impl ZRpcOps) -> MmResult<Self, WalletDbError> {
        let z_spending_key = match zcoin_builder.z_spending_key {
            Some(ref z_spending_key) => z_spending_key.clone(),
            None => extended_spending_key_from_protocol_info_and_policy(
                &zcoin_builder.protocol_info,
                &zcoin_builder.priv_key_policy,
            )
            .mm_err(|err| WalletDbError::ZCoinBuildError(err.to_string()))?,
        };

        let current_block_height = rpc
            .get_block_height()
            .await
            .map_err(|err| WalletDbError::GrpcError(err.to_string()))?;
        let sync_block = match zcoin_builder.z_coin_params.mode.clone() {
            ZcoinRpcMode::Light { sync_params, .. } => {
                let sync_height = match sync_params {
                    Some(LightClientSyncParams::Date(date)) => zcoin_builder
                        .calculate_starting_height_from_date(date, current_block_height)
                        .mm_err(WalletDbError::UtxoCoinBuildError)?,
                    Some(LightClientSyncParams::Height(height)) => height,
                    None => zcoin_builder
                        .calculate_starting_height_from_date(now_sec() - DAY_IN_SECONDS, current_block_height)
                        .mm_err(WalletDbError::UtxoCoinBuildError)?,
                };

                checkpoint_block_from_height(sync_height, rpc).await?
            },

            ZcoinRpcMode::Native => zcoin_builder.protocol_info.check_point_block.clone(),
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
        .mm_err(WalletDbError::ZcoinClientInitError)?;

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

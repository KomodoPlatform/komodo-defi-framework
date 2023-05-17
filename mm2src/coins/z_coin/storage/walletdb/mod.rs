use crate::z_coin::{extended_spending_key_from_protocol_info_and_policy, ZCoinBuildError, ZCoinBuilder,
                    ZcoinClientInitError, ZcoinConsensusParams};
use mm2_core::mm_ctx::{MmArc, MmCtx};
use mm2_err_handle::prelude::*;
use parking_lot::Mutex;
use std::sync::Arc;
use zcash_primitives::zip32::ExtendedFullViewingKey;

cfg_native!(
    use crate::z_coin::z_rpc::create_wallet_db;
    use zcash_client_sqlite::WalletDb;
);

cfg_wasm32!(
    mod wallet_idb;
    use wallet_idb::WalletDbInner;
);

pub type WalletDbShared = WalletDbSharedImpl;

#[derive(Debug, Display)]
pub enum WalletDbError {
    ZcoinClientInitError(ZcoinClientInitError),
    ZCoinBuildError(String),
    IndexedDBError(String),
}

#[derive(Clone)]
pub struct WalletDbSharedImpl {
    #[cfg(not(target_arch = "wasm32"))]
    pub db: Arc<Mutex<WalletDb<ZcoinConsensusParams>>>,
    #[cfg(target_arch = "wasm32")]
    pub db: SharedDb<WalletDbInner>,
    ticker: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl<'a> WalletDbSharedImpl {
    pub async fn new(zcoin_builder: &ZCoinBuilder<'a>) -> MmResult<Self, WalletDbError> {
        let z_spending_key = match zcoin_builder.z_spending_key {
            Some(ref z_spending_key) => z_spending_key.clone(),
            None => extended_spending_key_from_protocol_info_and_policy(
                &zcoin_builder.protocol_info,
                &zcoin_builder.priv_key_policy,
            )
            .map_err(|err| WalletDbError::ZCoinBuildError(err.to_string()))?,
        };
        let wallet_db = create_wallet_db(
            zcoin_builder
                .db_dir_path
                .join(format!("{}_wallet.db", zcoin_builder.ticker)),
            zcoin_builder.protocol_info.consensus_params.clone(),
            zcoin_builder.protocol_info.check_point_block.clone(),
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
    use super::*;
    use mm2_db::indexed_db::{ConstructibleDb, DbLocked, IndexedDb, SharedDb};

    pub type WalletDbRes<T> = MmResult<T, WalletDbError>;
    pub type WalletDbInnerLocked<'a> = DbLocked<'a, WalletDbInner>;

    impl<'a> WalletDbSharedImpl {
        pub async fn new(zcoin_builder: &ZCoinBuilder<'a>) -> MmResult<Self, WalletDbError> {
            Ok(Self {
                db: ConstructibleDb::new(&zcoin_builder.ctx).into_shared(),
                ticker: zcoin_builder.ticker.to_string(),
            })
        }

        async fn lock_db(&self) -> WalletDbRes<WalletDbInnerLocked<'_>> {
            self.db
                .get_or_initialize()
                .await
                .mm_err(|err| WalletDbError::IndexedDBError(err.to_string()))
        }
    }
);

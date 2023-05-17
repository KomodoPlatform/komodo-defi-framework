#[cfg(target_arch = "wasm32")] mod indexeddb;

use crate::z_coin::z_rpc::create_wallet_db;
use crate::z_coin::{ZCoinBuilder, ZcoinClientInitError, ZcoinConsensusParams};
use mm2_err_handle::prelude::{MmError, MmResult};
use parking_lot::Mutex;
use std::sync::Arc;
use zcash_client_sqlite::WalletDb;
use zcash_primitives::zip32::ExtendedFullViewingKey;

pub type WalletDbShared = WalletDbSharedImpl;

#[derive(Debug, Display)]
pub enum WalletDbError {
    ZcoinClientInitError(ZcoinClientInitError),
}

#[derive(Clone)]
pub struct WalletDbSharedImpl {
    #[cfg(not(target_arch = "wasm32"))]
    pub db: Arc<Mutex<WalletDb<ZcoinConsensusParams>>>,
    #[cfg(target_arch = "wasm32")]
    pub db: Arc<Mutex<WalletDb<ZcoinConsensusParams>>>,
}

impl<'a> WalletDbSharedImpl {
    pub async fn new(zcoin_builder: &ZCoinBuilder<'a>, evk: ExtendedFullViewingKey) -> MmResult<Self, WalletDbError> {
        let wallet_db = create_wallet_db(
            zcoin_builder.wallet_db_path(),
            zcoin_builder.protocol_info.consensus_params.clone(),
            zcoin_builder.protocol_info.check_point_block.clone(),
            evk,
        )
        .await
        .map_err(|err| MmError::new(WalletDbError::ZcoinClientInitError(err.into_inner())))?;

        Ok(Self {
            db: Arc::new(Mutex::new(wallet_db)),
        })
    }
}

use crate::hd_wallet_storage::{HDAccountInfo, HDWalletId, HDWalletStorageInternalOps, HDWalletStorageResult};
use async_trait::async_trait;
use common::mm_ctx::MmArc;

pub struct HDWalletSqliteStorage {}

#[async_trait]
impl HDWalletStorageInternalOps for HDWalletSqliteStorage {
    fn new(ctx: &MmArc) -> HDWalletStorageResult<Self>
    where
        Self: Sized,
    {
        todo!()
    }

    async fn load_accounts(&self, wallet_id: HDWalletId) -> HDWalletStorageResult<Vec<HDAccountInfo>> { todo!() }

    async fn load_account(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<HDAccountInfo>> {
        todo!()
    }

    async fn update_external_addresses_number(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_external_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        todo!()
    }

    async fn update_internal_addresses_number(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        todo!()
    }

    async fn update_addresses_numbers(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_external_addresses_number: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        todo!()
    }

    async fn upload_new_account(&self, wallet_id: HDWalletId, account: HDAccountInfo) -> HDWalletStorageResult<()> {
        todo!()
    }
}

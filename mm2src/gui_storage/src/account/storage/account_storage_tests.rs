use crate::account::storage::{AccountStorageBuilder, AccountStorageError};
use crate::account::{AccountId, AccountInfo, HwPubkey};
use mm2_number::BigDecimal;
use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;

async fn test_init_collection_impl() {
    let ctx = mm_ctx_with_custom_db();
    let storage = AccountStorageBuilder::new(&ctx).build().unwrap();

    storage.init().await.unwrap();
    // repetitive init must not fail
    storage.init().await.unwrap();
}

async fn test_upload_account_impl() {
    let ctx = mm_ctx_with_custom_db();
    let storage = AccountStorageBuilder::new(&ctx).build().unwrap();
    storage.init().await.unwrap();

    let accounts = vec![
        AccountId::Iguana,
        AccountId::HD { account_idx: 0 },
        AccountId::HW {
            device_pubkey: HwPubkey::from("1549128bbfb33b997949b4105b6a6371c998e212"),
        },
        AccountId::HW {
            device_pubkey: HwPubkey::from("f97d3a43dbea0993f1b7a6a299377d4ee164c849"),
        },
        AccountId::HW {
            device_pubkey: HwPubkey::from("69a20008cea0c15ee483b5bbdff942752634aa07"),
        },
        AccountId::HD { account_idx: 1 },
    ];

    for (i, account_id) in accounts.iter().enumerate() {
        let account = AccountInfo {
            account_id: account_id.clone(),
            name: format!("Account {}", i),
            description: format!("Description {}", i),
            balance_usd: BigDecimal::from(i as u64),
        };
        storage.upload_account(account.clone()).await.unwrap();

        let error = storage.upload_account(account).await.expect_err(&format!(
            "Uploading should have since the account {:?} has been uploaded already",
            account_id
        ));
        match error.into_inner() {
            AccountStorageError::AccountExistsAlready(found) if found == *account_id => (),
            other => panic!("Expected 'AccountExistsAlready({:?})' found {:?}", account_id, other),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native_tests {
    use common::block_on;

    #[test]
    fn test_init_collection() { block_on(super::test_init_collection_impl()) }

    #[test]
    fn test_upload_account_impl() { block_on(super::test_upload_account_impl()) }
}

#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_init_collection() { super::test_init_collection_impl().await }

    #[wasm_bindgen_test]
    async fn test_upload_account_impl() { super::test_upload_account_impl().await }
}

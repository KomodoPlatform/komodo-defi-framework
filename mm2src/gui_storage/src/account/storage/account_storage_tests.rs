use crate::account::storage::{AccountStorage, AccountStorageBuilder, AccountStorageError, AccountStorageResult};
use crate::account::{AccountId, AccountInfo, AccountWithCoins, AccountWithEnabledFlag, EnabledAccountId, HwPubkey};
use mm2_number::BigDecimal;
use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;
use std::collections::{BTreeMap, BTreeSet};

fn account_ids_for_test() -> Vec<AccountId> {
    vec![
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
    ]
}

fn accounts_for_test() -> Vec<AccountInfo> {
    account_ids_for_test()
        .iter()
        .enumerate()
        .map(|(i, account_id)| AccountInfo {
            account_id: account_id.clone(),
            name: format!("Account {}", i),
            description: format!("Description {}", i),
            balance_usd: BigDecimal::from(i as u64),
        })
        .collect()
}

fn accounts_to_map(accounts: Vec<AccountInfo>) -> BTreeMap<AccountId, AccountInfo> {
    accounts
        .into_iter()
        .map(|account| (account.account_id.clone(), account))
        .collect()
}

fn tag_with_enabled_flag(
    accounts: BTreeMap<AccountId, AccountInfo>,
    enabled: AccountId,
) -> BTreeMap<AccountId, AccountWithEnabledFlag> {
    accounts
        .into_iter()
        .map(|(account_id, account_info)| {
            (account_id.clone(), AccountWithEnabledFlag {
                account_info,
                enabled: account_id == enabled,
            })
        })
        .collect()
}

async fn fill_storage(storage: &dyn AccountStorage, accounts: Vec<AccountInfo>) -> AccountStorageResult<()> {
    for account in accounts {
        storage.upload_account(account.clone()).await?;
    }
    Ok(())
}

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

    for account in accounts_for_test() {
        storage.upload_account(account.clone()).await.unwrap();

        let account_id = account.account_id.clone();
        let error = storage.upload_account(account).await.expect_err(&format!(
            "Uploading should have since the account {:?} has been uploaded already",
            account_id
        ));
        match error.into_inner() {
            AccountStorageError::AccountExistsAlready(found) if found == account_id => (),
            other => panic!("Expected 'AccountExistsAlready({:?})' found {:?}", account_id, other),
        }
    }
}

async fn test_enable_account_impl() {
    let ctx = mm_ctx_with_custom_db();
    let storage = AccountStorageBuilder::new(&ctx).build().unwrap();
    storage.init().await.unwrap();

    let error = storage
        .enable_account(EnabledAccountId::Iguana)
        .await
        .expect_err("'enable_account' should have failed due to the selected account is not present in the storage");
    match error.into_inner() {
        AccountStorageError::NoSuchAccount(AccountId::Iguana) => (),
        other => panic!("Expected 'NoSuchAccount(Iguana)', found {:?}", other),
    }

    let accounts = accounts_to_map(accounts_for_test());

    let account_iguana = accounts.get(&AccountId::Iguana).unwrap().clone();
    storage.upload_account(account_iguana).await.unwrap();
    storage.enable_account(EnabledAccountId::Iguana).await.unwrap();

    let account_hd_1 = accounts.get(&AccountId::HD { account_idx: 0 }).unwrap().clone();
    storage.upload_account(account_hd_1).await.unwrap();

    let account_hd_2 = accounts.get(&AccountId::HD { account_idx: 1 }).unwrap().clone();
    storage.upload_account(account_hd_2).await.unwrap();

    // Check if Iguana account is still enabled.
    let actual_enabled = storage.load_enabled_account_id().await.unwrap();
    assert_eq!(actual_enabled, EnabledAccountId::Iguana);

    // Enable HD-1 account
    storage
        .enable_account(EnabledAccountId::HD { account_idx: 1 })
        .await
        .unwrap();
    let actual_enabled = storage.load_enabled_account_id().await.unwrap();
    assert_eq!(actual_enabled, EnabledAccountId::HD { account_idx: 1 });
}

async fn test_set_name_desc_balance_impl() {
    let ctx = mm_ctx_with_custom_db();
    let storage = AccountStorageBuilder::new(&ctx).build().unwrap();
    storage.init().await.unwrap();

    let accounts = accounts_for_test();

    fill_storage(storage.as_ref(), accounts.clone()).await.unwrap();
    storage.enable_account(EnabledAccountId::Iguana).await.unwrap();

    storage
        .set_name(AccountId::Iguana, "New name".to_string())
        .await
        .unwrap();

    let hd_1_id = AccountId::HD { account_idx: 1 };
    storage
        .set_description(hd_1_id.clone(), "New description".to_string())
        .await
        .unwrap();

    let hw_3_id = AccountId::HW {
        device_pubkey: HwPubkey::from("69a20008cea0c15ee483b5bbdff942752634aa07"),
    };
    storage
        .set_balance(hw_3_id.clone(), BigDecimal::from(23))
        .await
        .unwrap();

    let mut expected = accounts_to_map(accounts);
    expected.get_mut(&AccountId::Iguana).unwrap().name = "New name".to_string();
    expected.get_mut(&hd_1_id).unwrap().description = "New description".to_string();
    expected.get_mut(&hw_3_id).unwrap().balance_usd = BigDecimal::from(23);

    let actual = storage.load_accounts().await.unwrap();
    assert_eq!(actual, expected);

    let error = storage
        .set_name(AccountId::HD { account_idx: 2 }, "New name 4".to_string())
        .await
        .expect_err("'AccountStorage::set_name' should have failed due to an unknown 'AccountId'");

    match error.into_inner() {
        AccountStorageError::NoSuchAccount(AccountId::HD { account_idx: 2 }) => (),
        other => panic!("Expected 'NoSuchAccount(HD)' error, found: {}", other),
    }
}

async fn test_activate_deactivate_coins_impl() {
    let ctx = mm_ctx_with_custom_db();
    let storage = AccountStorageBuilder::new(&ctx).build().unwrap();
    storage.init().await.unwrap();

    let accounts = accounts_for_test();
    let accounts_map = accounts_to_map(accounts.clone());
    fill_storage(storage.as_ref(), accounts).await.unwrap();
    storage.enable_account(EnabledAccountId::Iguana).await.unwrap();

    // Deactivating unknown coins should never fail.
    storage
        .deactivate_coins(AccountId::Iguana, vec!["RICK".to_string(), "MORTY".to_string()])
        .await
        .unwrap();

    storage
        .activate_coins(AccountId::Iguana, vec!["RICK".to_string()])
        .await
        .unwrap();
    // Try reactivate `RICK` coin, it should be ignored.
    storage
        .activate_coins(AccountId::Iguana, vec!["RICK".to_string(), "MORTY".to_string()])
        .await
        .unwrap();
    storage
        .activate_coins(AccountId::HD { account_idx: 0 }, vec![
            "MORTY".to_string(),
            "QTUM".to_string(),
            "KMD".to_string(),
        ])
        .await
        .unwrap();

    let actual = storage.load_enabled_account_with_coins().await.unwrap();
    let expected = AccountWithCoins {
        account_info: accounts_map.get(&AccountId::Iguana).unwrap().clone(),
        coins: vec!["RICK".to_string(), "MORTY".to_string()].into_iter().collect(),
    };
    assert_eq!(actual, expected);

    // Enable `HD{0}` account to load its activated coins.
    storage
        .enable_account(EnabledAccountId::HD { account_idx: 0 })
        .await
        .unwrap();
    let actual = storage.load_enabled_account_with_coins().await.unwrap();
    let expected = AccountWithCoins {
        account_info: accounts_map.get(&AccountId::HD { account_idx: 0 }).unwrap().clone(),
        coins: vec!["MORTY".to_string(), "QTUM".to_string(), "KMD".to_string()]
            .into_iter()
            .collect(),
    };
    assert_eq!(actual, expected);

    // Deactivate `QTUM` and an unknown `BCH` coins for the `HD{0}` account.
    storage
        .deactivate_coins(AccountId::HD { account_idx: 0 }, vec![
            "BCH".to_string(),
            "QTUM".to_string(),
        ])
        .await
        .unwrap();
    let actual = storage.load_enabled_account_with_coins().await.unwrap();
    let expected = AccountWithCoins {
        account_info: accounts_map.get(&AccountId::HD { account_idx: 0 }).unwrap().clone(),
        coins: vec!["MORTY".to_string(), "KMD".to_string()].into_iter().collect(),
    };
    assert_eq!(actual, expected);

    // Deactivate all `HD{0}` account's coins.
    storage
        .deactivate_coins(AccountId::HD { account_idx: 0 }, vec![
            "MORTY".to_string(),
            "KMD".to_string(),
        ])
        .await
        .unwrap();
    let actual = storage.load_enabled_account_with_coins().await.unwrap();
    let expected = AccountWithCoins {
        account_info: accounts_map.get(&AccountId::HD { account_idx: 0 }).unwrap().clone(),
        coins: BTreeSet::new(),
    };
    assert_eq!(actual, expected);

    // Try to activate a coin for an unknown `HD{2}` account.
    let error = storage
        .activate_coins(AccountId::HD { account_idx: 2 }, vec!["RICK".to_string()])
        .await
        .expect_err("'AccountStorage::activate_coins' should have failed due to an unknown account_id");
    match error.into_inner() {
        AccountStorageError::NoSuchAccount(AccountId::HD { account_idx: 2 }) => (),
        other => panic!("Expected 'NoSuchAccount(HD)' error, found: {}", other),
    }

    // Try to deactivate a coin for an unknown `HD{3}` account.
    let error = storage
        .deactivate_coins(AccountId::HD { account_idx: 3 }, vec!["MORTY".to_string()])
        .await
        .expect_err("'AccountStorage::deactivate_coins' should have failed due to an unknown account_id");
    match error.into_inner() {
        AccountStorageError::NoSuchAccount(AccountId::HD { account_idx: 3 }) => (),
        other => panic!("Expected 'NoSuchAccount(HD)' error, found: {}", other),
    }
}

async fn test_load_accounts_with_enabled_flag_impl() {
    let ctx = mm_ctx_with_custom_db();
    let storage = AccountStorageBuilder::new(&ctx).build().unwrap();
    storage.init().await.unwrap();

    let accounts = accounts_for_test();
    let accounts_map = accounts_to_map(accounts.clone());

    fill_storage(storage.as_ref(), accounts.clone()).await.unwrap();

    let error = storage.load_accounts_with_enabled_flag().await.expect_err(
        "'AccountStorage::load_accounts_with_enabled_flag' should have failed since no account was enabled",
    );
    match error.into_inner() {
        AccountStorageError::NoEnabledAccount => (),
        other => panic!("Expected 'NoEnabledAccount' error, found: {}", other),
    }

    storage
        .enable_account(EnabledAccountId::HD { account_idx: 0 })
        .await
        .unwrap();
    let actual = storage.load_accounts_with_enabled_flag().await.unwrap();
    let expected = tag_with_enabled_flag(accounts_map.clone(), AccountId::HD { account_idx: 0 });
    assert_eq!(actual, expected);

    storage
        .enable_account(EnabledAccountId::HD { account_idx: 1 })
        .await
        .unwrap();
    let actual = storage.load_accounts_with_enabled_flag().await.unwrap();
    let expected = tag_with_enabled_flag(accounts_map.clone(), AccountId::HD { account_idx: 1 });
    assert_eq!(actual, expected);

    // Try to re-enable the same `HD{1}` account.
    storage
        .enable_account(EnabledAccountId::HD { account_idx: 1 })
        .await
        .unwrap();
    let actual = storage.load_accounts_with_enabled_flag().await.unwrap();
    let expected = tag_with_enabled_flag(accounts_map.clone(), AccountId::HD { account_idx: 1 });
    assert_eq!(actual, expected);

    storage.enable_account(EnabledAccountId::Iguana).await.unwrap();
    let actual = storage.load_accounts_with_enabled_flag().await.unwrap();
    let expected = tag_with_enabled_flag(accounts_map.clone(), AccountId::Iguana);
    assert_eq!(actual, expected);
}

#[cfg(not(target_arch = "wasm32"))]
mod native_tests {
    use common::block_on;

    #[test]
    fn test_init_collection() { block_on(super::test_init_collection_impl()) }

    #[test]
    fn test_upload_account() { block_on(super::test_upload_account_impl()) }

    #[test]
    fn test_enable_account() { block_on(super::test_enable_account_impl()) }

    #[test]
    fn test_set_name_desc_balance() { block_on(super::test_set_name_desc_balance_impl()) }

    #[test]
    fn test_activate_deactivate_coins() { block_on(super::test_activate_deactivate_coins_impl()) }

    #[test]
    fn test_load_accounts_with_enabled_flag() { block_on(super::test_load_accounts_with_enabled_flag_impl()) }
}

#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_init_collection() { super::test_init_collection_impl().await }

    #[wasm_bindgen_test]
    async fn test_upload_account() { super::test_upload_account_impl().await }

    #[wasm_bindgen_test]
    async fn test_enable_account() { super::test_enable_account_impl().await }

    #[wasm_bindgen_test]
    async fn test_set_name_desc_balance() { super::test_set_name_desc_balance_impl().await }

    #[wasm_bindgen_test]
    async fn test_activate_deactivate_coins() { super::test_activate_deactivate_coins_impl().await }

    #[wasm_bindgen_test]
    async fn test_load_accounts_with_enabled_flag() { super::test_load_accounts_with_enabled_flag_impl().await }
}

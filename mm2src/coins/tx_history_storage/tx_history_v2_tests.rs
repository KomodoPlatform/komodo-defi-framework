use crate::my_tx_history_v2::{GetHistoryResult, TxHistoryStorage};
use crate::tx_history_storage::{GetTxHistoryFilters, TxHistoryStorageBuilder, WalletId};
use crate::{BytesJson, TransactionDetails};
use common::PagingOptionsEnum;
use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;
use serde_json as json;
use std::num::NonZeroUsize;

fn wallet_id_for_test(test_name: &str) -> WalletId { WalletId::new(test_name.to_owned()) }

#[track_caller]
fn assert_get_history_result(actual: GetHistoryResult, expected_ids: Vec<BytesJson>, skipped: usize, total: usize) {
    let actual_ids: Vec<_> = actual.transactions.into_iter().map(|tx| tx.internal_id).collect();
    assert_eq!(actual_ids, expected_ids);
    assert_eq!(actual.skipped, skipped, "!skipped");
    assert_eq!(actual.total, total, "!total");
}

async fn get_coin_history<Storage: TxHistoryStorage>(
    storage: &Storage,
    wallet_id: &WalletId,
) -> Vec<TransactionDetails> {
    let filters = GetTxHistoryFilters::new();
    let paging_options = PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap());
    let limit = u32::MAX as usize;
    storage
        .get_history(wallet_id, filters, paging_options, limit)
        .await
        .unwrap()
        .transactions
}

async fn test_add_transactions_impl() {
    let wallet_id = wallet_id_for_test("TEST_ADD_TRANSACTIONS");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();
    let tx1_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
    let tx1: TransactionDetails = json::from_str(&tx1_json).unwrap();
    let transactions = [tx1.clone(), tx1.clone()];

    // must fail because we are adding transactions with the same internal_id
    storage
        .add_transactions_to_history(&wallet_id, transactions)
        .await
        .unwrap_err();
    let actual_txs = get_coin_history(&storage, &wallet_id).await;
    assert!(actual_txs.is_empty());

    let tx2_json = r#"{"tx_hex":"0400008085202f890158d6bccb2141e18633171f631f594b7f1ae85985390b534733ea5be4da220426030000006b483045022100895dea201a1dc59480d59790569df8664cf3d1d9332efeea7dcc38b4a96399b402206c183f33a3e87eb473a7d3da1488ee9a7d9580cfc86cc8460c79a69c08818478012102d09f2cb1693be9c0ea73bb48d45ce61805edd1c43590681b02f877206078a5b3ffffffff0400e1f505000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac00c2eb0b000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588aca01f791c000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac500df208ed0000001976a91490a0d8ba62c339ade97a14e81b6f531de03fdbb288ac00000000000000000000000000000000000000","tx_hash":"8d61223938c56ca97e9a0e1a295734c5f7b9dba8e4e0c1c638125190e7e796fa","from":["RNTv4xTLLm26p3SvsQCBy9qNK7s1RgGYSB"],"to":["RNTv4xTLLm26p3SvsQCBy9qNK7s1RgGYSB","RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10188.3504","spent_by_me":"0","received_by_me":"7.777","my_balance_change":"7.777","block_height":793474,"timestamp":1612780908,"fee_details":{"type":"Utxo","amount":"0.0001"},"coin":"RICK","internal_id":"8d61223938c56ca97e9a0e1a295734c5f7b9dba8e4e0c1c638125190e7e796fa"}"#;
    let tx2 = json::from_str(tx2_json).unwrap();
    let transactions = vec![tx1, tx2];
    storage
        .add_transactions_to_history(&wallet_id, transactions.clone())
        .await
        .unwrap();
    let actual_txs = get_coin_history(&storage, &wallet_id).await;
    assert_eq!(actual_txs, transactions);
}

async fn test_remove_transaction_impl() {
    let wallet_id = wallet_id_for_test("TEST_REMOVE_TRANSACTION");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();
    let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
    storage
        .add_transactions_to_history(&wallet_id, [json::from_str(tx_json).unwrap()])
        .await
        .unwrap();

    let remove_res = storage
        .remove_tx_from_history(
            &wallet_id,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        )
        .await
        .unwrap();
    assert!(remove_res.tx_existed());

    let remove_res = storage
        .remove_tx_from_history(
            &wallet_id,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        )
        .await
        .unwrap();
    assert!(!remove_res.tx_existed());
}

async fn test_get_transaction_impl() {
    let wallet_id = wallet_id_for_test("TEST_GET_TRANSACTION");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();

    let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
    storage
        .add_transactions_to_history(&wallet_id, [json::from_str(tx_json).unwrap()])
        .await
        .unwrap();

    let tx = storage
        .get_tx_from_history(
            &wallet_id,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        )
        .await
        .unwrap()
        .unwrap();
    println!("{:?}", tx);

    storage
        .remove_tx_from_history(
            &wallet_id,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        )
        .await
        .unwrap();

    let tx = storage
        .get_tx_from_history(
            &wallet_id,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        )
        .await
        .unwrap();
    assert!(tx.is_none());
}

async fn test_update_transaction_impl() {
    let wallet_id = wallet_id_for_test("TEST_UPDATE_TRANSACTION");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();

    let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
    let mut tx_details: TransactionDetails = json::from_str(tx_json).unwrap();
    storage
        .add_transactions_to_history(&wallet_id, [tx_details.clone()])
        .await
        .unwrap();

    tx_details.block_height = 12345;

    storage.update_tx_in_history(&wallet_id, &tx_details).await.unwrap();

    let updated = storage
        .get_tx_from_history(&wallet_id, &tx_details.internal_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(12345, updated.block_height);
}

async fn test_contains_and_get_unconfirmed_transaction_impl() {
    let wallet_id = wallet_id_for_test("TEST_CONTAINS_AND_GET_UNCONFIRMED_TRANSACTION");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();

    let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
    let mut tx_details: TransactionDetails = json::from_str(tx_json).unwrap();
    tx_details.block_height = 0;
    storage
        .add_transactions_to_history(&wallet_id, [tx_details.clone()])
        .await
        .unwrap();

    let contains_unconfirmed = storage.history_contains_unconfirmed_txes(&wallet_id).await.unwrap();
    assert!(contains_unconfirmed);

    let unconfirmed_transactions = storage.get_unconfirmed_txes_from_history(&wallet_id).await.unwrap();
    assert_eq!(unconfirmed_transactions.len(), 1);

    tx_details.block_height = 12345;
    storage.update_tx_in_history(&wallet_id, &tx_details).await.unwrap();

    let contains_unconfirmed = storage.history_contains_unconfirmed_txes(&wallet_id).await.unwrap();
    assert!(!contains_unconfirmed);

    let unconfirmed_transactions = storage.get_unconfirmed_txes_from_history(&wallet_id).await.unwrap();
    assert!(unconfirmed_transactions.is_empty());
}

async fn test_has_transactions_with_hash_impl() {
    let wallet_id = wallet_id_for_test("TEST_HAS_TRANSACTIONS_WITH_HASH");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();

    let has_tx_hash = storage
        .history_has_tx_hash(
            &wallet_id,
            "2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c",
        )
        .await
        .unwrap();
    assert!(!has_tx_hash);

    let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
    let tx_details: TransactionDetails = json::from_str(tx_json).unwrap();

    storage
        .add_transactions_to_history(&wallet_id, [tx_details])
        .await
        .unwrap();

    let has_tx_hash = storage
        .history_has_tx_hash(
            &wallet_id,
            "2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c",
        )
        .await
        .unwrap();
    assert!(has_tx_hash);
}

async fn test_unique_tx_hashes_num_impl() {
    let wallet_id = wallet_id_for_test("TEST_UNIQUE_TX_HASHES_NUM");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();

    let tx1_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
    let tx1: TransactionDetails = json::from_str(&tx1_json).unwrap();

    let mut tx2 = tx1.clone();
    tx2.internal_id = BytesJson(vec![1; 32]);

    let tx3_json = r#"{"tx_hex":"0400008085202f890158d6bccb2141e18633171f631f594b7f1ae85985390b534733ea5be4da220426030000006b483045022100895dea201a1dc59480d59790569df8664cf3d1d9332efeea7dcc38b4a96399b402206c183f33a3e87eb473a7d3da1488ee9a7d9580cfc86cc8460c79a69c08818478012102d09f2cb1693be9c0ea73bb48d45ce61805edd1c43590681b02f877206078a5b3ffffffff0400e1f505000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac00c2eb0b000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588aca01f791c000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac500df208ed0000001976a91490a0d8ba62c339ade97a14e81b6f531de03fdbb288ac00000000000000000000000000000000000000","tx_hash":"8d61223938c56ca97e9a0e1a295734c5f7b9dba8e4e0c1c638125190e7e796fa","from":["RNTv4xTLLm26p3SvsQCBy9qNK7s1RgGYSB"],"to":["RNTv4xTLLm26p3SvsQCBy9qNK7s1RgGYSB","RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10188.3504","spent_by_me":"0","received_by_me":"7.777","my_balance_change":"7.777","block_height":793474,"timestamp":1612780908,"fee_details":{"type":"Utxo","amount":"0.0001"},"coin":"RICK","internal_id":"8d61223938c56ca97e9a0e1a295734c5f7b9dba8e4e0c1c638125190e7e796fa"}"#;
    let tx3 = json::from_str(tx3_json).unwrap();

    let transactions = [tx1, tx2, tx3];
    storage
        .add_transactions_to_history(&wallet_id, transactions)
        .await
        .unwrap();

    let tx_hashes_num = storage.unique_tx_hashes_num_in_history(&wallet_id).await.unwrap();
    assert_eq!(2, tx_hashes_num);
}

async fn test_add_and_get_tx_from_cache_impl() {
    let wallet_id_1 = WalletId::new("TEST_ADD_AND_GET_TX_FROM_CACHE".to_owned());
    // `wallet_id_2` has the same `ticker` and a non-empty `hd_wallet_rmd160`.
    let wallet_id_2 = WalletId::new("TEST_ADD_AND_GET_TX_FROM_CACHE".to_owned())
        .with_hd_wallet_rmd160("108f07b8382412612c048d07d13f814118445acd".into());

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id_1).await.unwrap();
    storage.init(&wallet_id_2).await.unwrap();

    let tx = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
    let tx: TransactionDetails = json::from_str(tx).unwrap();

    storage
        .add_tx_to_cache(&wallet_id_1, &tx.tx_hash, &tx.tx_hex)
        .await
        .unwrap();

    let tx_hex_from_1 = storage
        .tx_bytes_from_cache(&wallet_id_1, &tx.tx_hash)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tx_hex_from_1, tx.tx_hex);

    // Since `wallet_id_1` and `wallet_id_2` wallets have the same `ticker`, the wallets must have one transaction cache.
    let tx_hex_from_2 = storage
        .tx_bytes_from_cache(&wallet_id_2, &tx.tx_hash)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tx_hex_from_2, tx.tx_hex);
}

async fn test_get_raw_tx_bytes_on_add_transactions_impl() {
    let wallet_id = wallet_id_for_test("TEST_GET_RAW_TX_BYTES_ON_ADD_TRANSACTIONS");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();

    let tx_hash = "2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c";

    let maybe_tx_hex = storage.tx_bytes_from_cache(&wallet_id, &tx_hash).await.unwrap();
    assert!(maybe_tx_hex.is_none());

    let tx1_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
    let tx1: TransactionDetails = json::from_str(tx1_json).unwrap();

    let mut tx2 = tx1.clone();
    tx2.internal_id = BytesJson(vec![1; 32]);

    let expected_tx_hex = tx1.tx_hex.clone();

    let transactions = [tx1, tx2];
    storage
        .add_transactions_to_history(&wallet_id, transactions)
        .await
        .unwrap();

    let tx_hex = storage
        .tx_bytes_from_cache(&wallet_id, &tx_hash)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(tx_hex, expected_tx_hex);
}

async fn test_get_history_page_number_impl() {
    let wallet_id = wallet_id_for_test("TEST_GET_HISTORY_PAGE_NUMBER");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();
    let tx_details = include_str!("../for_tests/tBCH_tx_history_fixtures.json");
    let transactions: Vec<TransactionDetails> = json::from_str(tx_details).unwrap();

    storage
        .add_transactions_to_history(&wallet_id, transactions)
        .await
        .unwrap();

    let filters = GetTxHistoryFilters::new();
    let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap());
    let limit = 4;

    let result = storage.get_history(&wallet_id, filters, paging, limit).await.unwrap();

    let expected_internal_ids: Vec<BytesJson> = vec![
        "6686ee013620d31ba645b27d581fed85437ce00f46b595a576718afac4dd5b69".into(),
        "c07836722bbdfa2404d8fe0ea56700d02e2012cb9dc100ccaf1138f334a759ce".into(),
        "091877294268b2b1734255067146f15c3ac5e6199e72cd4f68a8d9dec32bb0c0".into(),
        "d76723c092b64bc598d5d2ceafd6f0db37dce4032db569d6f26afb35491789a7".into(),
    ];
    assert_get_history_result(result, expected_internal_ids, 0, 123);

    let filters = GetTxHistoryFilters::new()
        .with_token_id("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7".to_owned());
    let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(2).unwrap());
    let limit = 5;

    let result = storage.get_history(&wallet_id, filters, paging, limit).await.unwrap();

    let expected_internal_ids: Vec<BytesJson> = vec![
        "433b641bc89e1b59c22717918583c60ec98421805c8e85b064691705d9aeb970".into(),
        "cd6ec10b0cd9747ddc66ac5c97c2d7b493e8cea191bc2d847b3498719d4bd989".into(),
        "1c1e68357cf5a6dacb53881f13aa5d2048fe0d0fab24b76c9ec48f53884bed97".into(),
        "c4304b5ef4f1b88ed4939534a8ca9eca79f592939233174ae08002e8454e3f06".into(),
        "b0035434a1e7be5af2ed991ee2a21a90b271c5852a684a0b7d315c5a770d1b1c".into(),
    ];
    assert_get_history_result(result, expected_internal_ids, 5, 121);
}

async fn test_get_history_from_id_impl() {
    let wallet_id = wallet_id_for_test("TEST_GET_HISTORY_FROM_ID");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();
    let tx_details = include_str!("../for_tests/tBCH_tx_history_fixtures.json");
    let transactions: Vec<TransactionDetails> = json::from_str(tx_details).unwrap();

    storage
        .add_transactions_to_history(&wallet_id, transactions)
        .await
        .unwrap();

    let filters = GetTxHistoryFilters::new();
    let paging = PagingOptionsEnum::FromId("6686ee013620d31ba645b27d581fed85437ce00f46b595a576718afac4dd5b69".into());
    let limit = 3;

    let result = storage.get_history(&wallet_id, filters, paging, limit).await.unwrap();

    let expected_internal_ids: Vec<BytesJson> = vec![
        "c07836722bbdfa2404d8fe0ea56700d02e2012cb9dc100ccaf1138f334a759ce".into(),
        "091877294268b2b1734255067146f15c3ac5e6199e72cd4f68a8d9dec32bb0c0".into(),
        "d76723c092b64bc598d5d2ceafd6f0db37dce4032db569d6f26afb35491789a7".into(),
    ];
    assert_get_history_result(result, expected_internal_ids, 1, 123);

    let filters = GetTxHistoryFilters::new()
        .with_token_id("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7".to_owned());
    let paging = PagingOptionsEnum::FromId("433b641bc89e1b59c22717918583c60ec98421805c8e85b064691705d9aeb970".into());
    let limit = 4;

    let result = storage.get_history(&wallet_id, filters, paging, limit).await.unwrap();

    let expected_internal_ids: Vec<BytesJson> = vec![
        "cd6ec10b0cd9747ddc66ac5c97c2d7b493e8cea191bc2d847b3498719d4bd989".into(),
        "1c1e68357cf5a6dacb53881f13aa5d2048fe0d0fab24b76c9ec48f53884bed97".into(),
        "c4304b5ef4f1b88ed4939534a8ca9eca79f592939233174ae08002e8454e3f06".into(),
        "b0035434a1e7be5af2ed991ee2a21a90b271c5852a684a0b7d315c5a770d1b1c".into(),
    ];
    assert_get_history_result(result, expected_internal_ids, 6, 121);
}

async fn test_get_history_for_addresses_impl() {
    let wallet_id = wallet_id_for_test("TEST_GET_HISTORY_FROM_ID");

    let ctx = mm_ctx_with_custom_db();
    let storage = TxHistoryStorageBuilder::new(&ctx).build().unwrap();

    storage.init(&wallet_id).await.unwrap();
    let tx_details = include_str!("../for_tests/tBCH_tx_history_fixtures.json");
    let transactions: Vec<TransactionDetails> = json::from_str(tx_details).unwrap();

    storage
        .add_transactions_to_history(&wallet_id, transactions)
        .await
        .unwrap();

    let for_addresses = vec![
        "slptest:ppfdp6t2qs7rc79wxjppwv0hwvr776x5vu2enth4zh".to_owned(),
        "slptest:pqgk69yyj6dzag4mdyur9lykye89ucz9vskelzwhck".to_owned(),
    ];
    let filters = GetTxHistoryFilters::new()
        .with_for_addresses(for_addresses)
        .with_token_id("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7".to_owned());
    let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap());
    let limit = 5;

    let result = storage.get_history(&wallet_id, filters, paging, limit).await.unwrap();
    let expected_ids = vec![
        "660d57aad6e7807ee99459a77ed6b526771db8567fff99ca055d652913555d08".into(),
        "e46fa0836be0534f7799b2ef5b538551ea25b6f430b7e015a95731efb7a0cd4f".into(),
        "fc666307cafcbf29e4b95ccc261a24603c8168535283c6ed8243d4cd8c2543c8".into(),
        "fe78e04399219ef75271019f6d5db5d77179e9f310f8364604a6e4e05c4d7563".into(),
    ];
    assert_get_history_result(result, expected_ids, 0, 4);

    // Try to request by the specified internal ID.

    let for_addresses = vec![
        "slptest:ppfdp6t2qs7rc79wxjppwv0hwvr776x5vu2enth4zh".to_owned(),
        "slptest:pqgk69yyj6dzag4mdyur9lykye89ucz9vskelzwhck".to_owned(),
    ];
    let filters = GetTxHistoryFilters::new()
        .with_for_addresses(for_addresses)
        .with_token_id("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7".to_owned());
    let paging = PagingOptionsEnum::FromId("e46fa0836be0534f7799b2ef5b538551ea25b6f430b7e015a95731efb7a0cd4f".into());
    let limit = 4;

    let result = storage.get_history(&wallet_id, filters, paging, limit).await.unwrap();
    let expected_ids = vec![
        "fc666307cafcbf29e4b95ccc261a24603c8168535283c6ed8243d4cd8c2543c8".into(),
        "fe78e04399219ef75271019f6d5db5d77179e9f310f8364604a6e4e05c4d7563".into(),
    ];
    assert_get_history_result(result, expected_ids, 2, 4);

    // If there are no transactions by the specified filters and paging options,
    // we need to get an empty history.

    let for_addresses = vec![
        "slptest:ppfdp6t2qs7rc79wxjppwv0hwvr776x5vu2enth4zh".to_owned(),
        "slptest:pqgk69yyj6dzag4mdyur9lykye89ucz9vskelzwhck".to_owned(),
    ];
    let filters = GetTxHistoryFilters::new()
        .with_for_addresses(for_addresses)
        .with_token_id("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7".to_owned());
    let paging = PagingOptionsEnum::FromId("6686ee013620d31ba645b27d581fed85437ce00f46b595a576718afac4dd5b69".into());
    let limit = 2;

    let result = storage.get_history(&wallet_id, filters, paging, limit).await.unwrap();
    assert_get_history_result(result, Vec::new(), 0, 4);

    // If there are no transactions by the specified filters and paging option,
    // we need to get an empty history.

    let for_addresses = vec![
        "slptest:ppfdp6t2qs7rc79wxjppwv0hwvr776x5vu2enth4zh".to_owned(),
        "slptest:pqgk69yyj6dzag4mdyur9lykye89ucz9vskelzwhck".to_owned(),
    ];
    let filters = GetTxHistoryFilters::new()
        .with_for_addresses(for_addresses)
        .with_token_id("bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7".to_owned());
    let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(2).unwrap());
    let limit = 4;

    let result = storage.get_history(&wallet_id, filters, paging, limit).await.unwrap();
    assert_get_history_result(result, Vec::new(), 4, 4);
}

#[cfg(test)]
mod native_tests {
    use super::wallet_id_for_test;
    use crate::my_tx_history_v2::TxHistoryStorage;
    use crate::tx_history_storage::sql_tx_history_storage_v2::SqliteTxHistoryStorage;
    use common::block_on;
    use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;

    #[test]
    fn test_init_collection() {
        let wallet_id = wallet_id_for_test("TEST_INIT_COLLECTION");

        let ctx = mm_ctx_with_custom_db();
        let storage = SqliteTxHistoryStorage::new(&ctx).unwrap();

        let initialized = block_on(storage.is_initialized_for(&wallet_id)).unwrap();
        assert!(!initialized);

        block_on(storage.init(&wallet_id)).unwrap();
        // repetitive init must not fail
        block_on(storage.init(&wallet_id)).unwrap();

        let initialized = block_on(storage.is_initialized_for(&wallet_id)).unwrap();
        assert!(initialized);
    }

    #[test]
    fn test_add_transactions() { block_on(super::test_add_transactions_impl()); }

    #[test]
    fn test_remove_transaction() { block_on(super::test_remove_transaction_impl()); }

    #[test]
    fn test_get_transaction() { block_on(super::test_get_transaction_impl()); }

    #[test]
    fn test_update_transaction() { block_on(super::test_update_transaction_impl()); }

    #[test]
    fn test_contains_and_get_unconfirmed_transaction() {
        block_on(super::test_contains_and_get_unconfirmed_transaction_impl());
    }

    #[test]
    fn test_has_transactions_with_hash() { block_on(super::test_has_transactions_with_hash_impl()); }

    #[test]
    fn test_unique_tx_hashes_num() { block_on(super::test_unique_tx_hashes_num_impl()); }

    #[test]
    fn test_add_and_get_tx_from_cache() { block_on(super::test_add_and_get_tx_from_cache_impl()); }

    #[test]
    fn test_get_raw_tx_bytes_on_add_transactions() {
        block_on(super::test_get_raw_tx_bytes_on_add_transactions_impl());
    }

    #[test]
    fn test_get_history_page_number() { block_on(super::test_get_history_page_number_impl()); }

    #[test]
    fn test_get_history_from_id() { block_on(super::test_get_history_from_id_impl()); }

    #[test]
    fn test_get_history_for_addresses() { block_on(super::test_get_history_for_addresses_impl()); }
}

#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use super::wallet_id_for_test;
    use crate::my_tx_history_v2::TxHistoryStorage;
    use crate::tx_history_storage::wasm::tx_history_storage_v2::IndexedDbTxHistoryStorage;
    use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_init_collection() {
        let wallet_id = wallet_id_for_test("TEST_INIT_COLLECTION");

        let ctx = mm_ctx_with_custom_db();
        let storage = IndexedDbTxHistoryStorage::new(&ctx).unwrap();

        // Please note this is the `IndexedDbTxHistoryStorage` specific:
        // [`IndexedDbTxHistoryStorage::is_initialized_for`] always returns `true`.
        let initialized = storage.is_initialized_for(&wallet_id).await.unwrap();
        assert!(initialized);

        // repetitive init must not fail
        storage.init(&wallet_id).await.unwrap();

        let initialized = storage.is_initialized_for(&wallet_id).await.unwrap();
        assert!(initialized);
    }

    #[wasm_bindgen_test]
    async fn test_add_transactions() { super::test_add_transactions_impl().await; }

    #[wasm_bindgen_test]
    async fn test_remove_transaction() { super::test_remove_transaction_impl().await; }

    #[wasm_bindgen_test]
    async fn test_get_transaction() { super::test_get_transaction_impl().await; }

    #[wasm_bindgen_test]
    async fn test_update_transaction() { super::test_update_transaction_impl().await; }

    #[wasm_bindgen_test]
    async fn test_contains_and_get_unconfirmed_transaction() {
        super::test_contains_and_get_unconfirmed_transaction_impl().await;
    }

    #[wasm_bindgen_test]
    async fn test_has_transactions_with_hash() { super::test_has_transactions_with_hash_impl().await; }

    #[wasm_bindgen_test]
    async fn test_unique_tx_hashes_num() { super::test_unique_tx_hashes_num_impl().await; }

    #[wasm_bindgen_test]
    async fn test_add_and_get_tx_from_cache() { super::test_add_and_get_tx_from_cache_impl().await; }

    #[wasm_bindgen_test]
    async fn test_get_raw_tx_bytes_on_add_transactions() {
        super::test_get_raw_tx_bytes_on_add_transactions_impl().await;
    }

    #[wasm_bindgen_test]
    async fn test_get_history_page_number() { super::test_get_history_page_number_impl().await; }

    #[wasm_bindgen_test]
    async fn test_get_history_from_id() { super::test_get_history_from_id_impl().await; }

    #[wasm_bindgen_test]
    async fn test_get_history_for_addresses() { super::test_get_history_for_addresses_impl().await; }
}

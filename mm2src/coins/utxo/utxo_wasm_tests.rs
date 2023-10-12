use mm2_core::mm_ctx::MmCtxBuilder;
use serialization::deserialize;
use wasm_bindgen_test::*;

use super::rpc_clients::{ElectrumClient, UtxoRpcClientOps};
use super::utxo_builder::{UtxoArcBuilder, UtxoCoinBuilderCommonOps};
use super::utxo_standard::UtxoStandardCoin;
use super::*;
use crate::utxo::utxo_common_tests;
use crate::{IguanaPrivKey, PrivKeyBuildPolicy};

wasm_bindgen_test_configure!(run_in_browser);

const TEST_COIN_NAME: &str = "RICK";

pub async fn electrum_client_for_test(servers: &[&str]) -> ElectrumClient {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let servers: Vec<_> = servers
        .iter()
        .map(|server| json!({ "url": server, "protocol": "WSS" }))
        .collect();
    let req = json!({
        "method": "electrum",
        "servers": servers,
    });
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let priv_key_policy = PrivKeyBuildPolicy::IguanaPrivKey(IguanaPrivKey::default());
    let builder = UtxoArcBuilder::new(
        &ctx,
        TEST_COIN_NAME,
        &Json::Null,
        &params,
        priv_key_policy,
        UtxoStandardCoin::from,
    );
    let args = ElectrumBuilderArgs {
        spawn_ping: false,
        negotiate_version: true,
        collect_metrics: false,
    };

    let servers = servers.into_iter().map(|s| json::from_value(s).unwrap()).collect();
    let abortable_system = AbortableQueue::default();
    builder.electrum_client(abortable_system, args, servers).await.unwrap()
}

#[wasm_bindgen_test]
async fn test_electrum_rpc_client() {
    let client = electrum_client_for_test(&["electrum1.cipig.net:30017", "electrum2.cipig.net:30017"]).await;

    let tx_hash: H256Json = hex::decode("0a0fda88364b960000f445351fe7678317a1e0c80584de0413377ede00ba696f")
        .unwrap()
        .as_slice()
        .into();
    let verbose_tx = client
        .get_verbose_transaction(&tx_hash)
        .compat()
        .await
        .expect("!get_verbose_transaction");
    let actual: UtxoTx = deserialize(verbose_tx.hex.as_slice()).unwrap();
    let expected = UtxoTx::from("0400008085202f8902358549fe3cf9a66bf61fb57bca1b3b49434a148a4dc29450b5eefe583f2f9ecf000000006a4730440220112aa3737672f8aa16a58426f5e7656ad13d21a219390c7a0b2e266ee6b216a8022008e9f9e94db91f069f831b0d40b7f75938122cddceaa25197146dfb00fe82599012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffff358549fe3cf9a66bf61fb57bca1b3b49434a148a4dc29450b5eefe583f2f9ecf010000006b483045022100d054464799246254b09f96333bf52537938abe31c24bacf41c9ef600b28155950220527ec33c4a5bef79dcabf97e38aa240fecdd14c96f698560b2f10ec2abc2e992012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffff0240420f00000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788ac66418f00000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788ac0e2aa85f000000000000000000000000000000");
    assert_eq!(actual, expected);
}

#[wasm_bindgen_test]
async fn test_electrum_display_balances() {
    let rpc_client = electrum_client_for_test(&["electrum1.cipig.net:30017", "electrum2.cipig.net:30017"]).await;
    utxo_common_tests::test_electrum_display_balances(&rpc_client).await;
}

#[wasm_bindgen_test]
async fn test_hd_utxo_tx_history() {
    let rpc_client = electrum_client_for_test(&["electrum1.cipig.net:30018", "electrum2.cipig.net:30018"]).await;
    utxo_common_tests::test_hd_utxo_tx_history_impl(rpc_client).await;
}

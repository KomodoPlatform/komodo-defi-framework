// use common::custom_futures::timeout::FutureTimerExt;
// use common::{executor::Timer, Future01CompatExt};
// use mm2_core::mm_ctx::MmCtxBuilder;
// use mm2_test_helpers::for_tests::{pirate_conf, ARRR};
use common::log::warn;
use wasm_bindgen_test::*;
//
// use super::light_zcoin_activation_params;
// use crate::z_coin::tx_history_events::ZCoinTxHistoryEventStreamer;
// use crate::z_coin::z_coin_from_conf_and_params;
// use crate::z_coin::z_htlc::z_send_dex_fee;
// use crate::PrivKeyBuildPolicy;
// use crate::{CoinProtocol, MarketCoinOps, MmCoin};

#[wasm_bindgen_test]
async fn test_zcoin_tx_streaming() {
    warn!("Skipping test_zcoin_tx_streaming since it's failing, check https://github.com/KomodoPlatform/komodo-defi-framework/issues/2366");
    //     let ctx = MmCtxBuilder::default().into_mm_arc();
    //     let conf = pirate_conf();
    //     let params = light_zcoin_activation_params();
    //     // Address: RQX5MnqnxEk6P33LSEAxC2vqA7DfSdWVyH
    //     // Or: zs1n2azlwcj9pvl2eh36qvzgeukt2cpzmw44hya8wyu52j663d0dfs4d5hjx6tr04trz34jxyy433j
    //     let priv_key_policy =
    //         PrivKeyBuildPolicy::IguanaPrivKey("6d862798ef956fb60fb17bcc417dd6d44bfff066a4a49301cd2528e41a4a3e45".into());
    //     let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].clone()).unwrap() {
    //         CoinProtocol::ZHTLC(protocol_info) => protocol_info,
    //         other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    //     };
    //
    //     let coin = z_coin_from_conf_and_params(&ctx, ARRR, &conf, &params, protocol_info, priv_key_policy)
    //         .await
    //         .unwrap();
    //
    //     // Wait till we are synced with the sapling state.
    //     while !coin.is_sapling_state_synced().await {
    //         Timer::sleep(1.).await;
    //     }
    //
    //     // Query the block height to make sure our electrums are actually connected.
    //     log!("current block = {:?}", coin.current_block().compat().await.unwrap());
    //
    //     // Add a new client to use it for listening to tx history events.
    //     let client_id = 1;
    //     let mut event_receiver = ctx.event_stream_manager.new_client(client_id).unwrap();
    //     // Add the streamer that will stream the tx history events.
    //     let streamer = ZCoinTxHistoryEventStreamer::new(coin.clone());
    //     // Subscribe the client to the streamer.
    //     ctx.event_stream_manager
    //         .add(client_id, streamer, coin.spawner())
    //         .await
    //         .unwrap();
    //
    //     // Send a tx to have it in the tx history.
    //     let tx = z_send_dex_fee(&coin, "0.0001".parse().unwrap(), &[1; 16])
    //         .await
    //         .unwrap();
    //
    //     // Wait for the tx history event (should be streamed next block).
    //     let event = Box::pin(event_receiver.recv())
    //         .timeout_secs(120.)
    //         .await
    //         .expect("timed out waiting for tx to showup")
    //         .expect("tx history sender shutdown");
    //
    //     log!("{:?}", event.get());
    //     let (event_type, event_data) = event.get();
    //     // Make sure this is not an error event,
    //     assert!(!event_type.starts_with("ERROR_"));
    //     // from the expected streamer,
    //     assert_eq!(
    //         event_type,
    //         ZCoinTxHistoryEventStreamer::derive_streamer_id(coin.ticker())
    //     );
    //     // and has the expected data.
    //     assert_eq!(event_data["tx_hash"].as_str().unwrap(), tx.txid().to_string());
}

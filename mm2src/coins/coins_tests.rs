use base64::{encode_config as base64_encode, URL_SAFE};
use futures01::Future;

use crate::utxo::rpc_clients::NativeClientImpl;

pub fn test_list_unspent() {
    let client = NativeClientImpl {
        coin_ticker: "RICK".into(),
        uri: "http://127.0.0.1:10271".to_owned(),
        auth: fomat!("Basic " (base64_encode("user481805103:pass97a61c8d048bcf468c6c39a314970e557f57afd1d8a5edee917fb29bafb3a43371", URL_SAFE))),
        event_handlers: Default::default(),
    };
    let unspents = client.list_unspent(0, std::i32::MAX, vec!["RBs52D7pVq7txo6SCz1Tuyw2WrPmdqU3qw".to_owned()]);
    let unspents = unwrap! (unspents.wait());
    log!("Unspents " [unspents]);
}

pub fn test_get_block_count() {
    let client = NativeClientImpl {
        coin_ticker: "RICK".into(),
        uri: "http://127.0.0.1:10271".to_owned(),
        auth: fomat!("Basic " (base64_encode("user481805103:pass97a61c8d048bcf468c6c39a314970e557f57afd1d8a5edee917fb29bafb3a43371", URL_SAFE))),
        event_handlers: Default::default(),
    };
    let block_count = unwrap! (client.validate_address("RBs52D7pVq7txo6SCz1Tuyw2WrPmdqU3qw".to_owned()).wait());
    log!("Block count " [block_count]);
}

pub fn test_import_address() {
    let client = NativeClientImpl {
        coin_ticker: "RICK".into(),
        uri: "http://127.0.0.1:10271".to_owned(),
        auth: fomat!("Basic " (base64_encode("user481805103:pass97a61c8d048bcf468c6c39a314970e557f57afd1d8a5edee917fb29bafb3a43371", URL_SAFE))),
        event_handlers: Default::default(),
    };
    let import_addr = client.import_address(
        "bMjWGCinft5qEvsuf9Wg1fgz1CjpXBXbTB",
        "bMjWGCinft5qEvsuf9Wg1fgz1CjpXBXbTB",
        true
    );
    let import_addr = import_addr.wait().unwrap();
    log!("Block count " [import_addr]);
}

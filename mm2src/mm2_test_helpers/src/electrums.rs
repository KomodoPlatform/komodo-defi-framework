use serde_json::{json, Value as Json};

#[cfg(target_arch = "wasm32")]
pub fn rick_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:30020", "protocol": "WSS" }),
        json!({ "url": "electrum2.cipig.net:30020", "protocol": "WSS" }),
        json!({ "url": "electrum3.cipig.net:30020", "protocol": "WSS" }),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
pub fn rick_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:10020" }),
        json!({ "url": "electrum2.cipig.net:10020" }),
        json!({ "url": "electrum3.cipig.net:10020" }),
    ]
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub fn morty_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:30021", "protocol": "WSS" }),
        json!({ "url": "electrum2.cipig.net:30021", "protocol": "WSS" }),
        json!({ "url": "electrum3.cipig.net:30021", "protocol": "WSS" }),
    ]
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub fn morty_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:10021" }),
        json!({ "url": "electrum2.cipig.net:10021" }),
        json!({ "url": "electrum3.cipig.net:10021" }),
    ]
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub fn btc_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:30000", "protocol": "WSS" }),
        json!({ "url": "electrum2.cipig.net:30000", "protocol": "WSS" }),
        json!({ "url": "electrum3.cipig.net:30000", "protocol": "WSS" }),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
pub fn btc_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:10000" }),
        json!({ "url": "electrum2.cipig.net:10000" }),
        json!({ "url": "electrum3.cipig.net:10000" }),
    ]
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub fn tbtc_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:30068", "protocol": "WSS" }),
        json!({ "url": "electrum2.cipig.net:30068", "protocol": "WSS" }),
        json!({ "url": "electrum3.cipig.net:30068", "protocol": "WSS" }),
    ]
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub fn tbtc_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:10068" }),
        json!({ "url": "electrum2.cipig.net:10068" }),
        json!({ "url": "electrum3.cipig.net:10068" }),
    ]
}

#[cfg(target_arch = "wasm32")]
pub fn qtum_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:30071", "protocol": "WSS" }),
        json!({ "url": "electrum2.cipig.net:30071", "protocol": "WSS" }),
        json!({ "url": "electrum3.cipig.net:30071", "protocol": "WSS" }),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
pub fn qtum_electrums() -> Vec<Json> {
    vec![
        json!({ "url": "electrum1.cipig.net:10071" }),
        json!({ "url": "electrum2.cipig.net:10071" }),
        json!({ "url": "electrum3.cipig.net:10071" }),
    ]
}

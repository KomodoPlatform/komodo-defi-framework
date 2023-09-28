pub mod grpc_web;
pub mod p2p;
pub mod transport;

#[cfg(not(target_arch = "wasm32"))] pub mod ip_addr;
#[cfg(not(target_arch = "wasm32"))] pub mod native_http;
#[cfg(not(target_arch = "wasm32"))] pub mod native_tls;
#[cfg(not(target_arch = "wasm32"))] pub mod network_event;
#[cfg(not(target_arch = "wasm32"))] pub mod sse_handler;
#[cfg(target_arch = "wasm32")] pub mod wasm_http;
#[cfg(target_arch = "wasm32")] pub mod wasm_ws;

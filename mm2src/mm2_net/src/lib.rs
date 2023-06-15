pub mod grpc_web;
pub mod transport;

#[cfg(not(target_arch = "wasm32"))] pub mod ip_addr;
#[cfg(not(target_arch = "wasm32"))] pub mod native_http;
#[cfg(not(target_arch = "wasm32"))] pub mod native_tls;
#[cfg(target_arch = "wasm32")] pub mod wasm_http;
#[cfg(target_arch = "wasm32")] pub mod wasm_ws;

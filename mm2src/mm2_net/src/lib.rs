pub mod grpc_web;
#[cfg(feature = "event-stream")] pub mod network_event;
#[cfg(feature = "p2p")] pub mod p2p;
pub mod transport;

#[cfg(not(target_arch = "wasm32"))] pub mod ip_addr;
#[cfg(not(target_arch = "wasm32"))] pub mod native_http;
#[cfg(not(target_arch = "wasm32"))] pub mod native_tls;
#[cfg(all(feature = "event-stream", not(target_arch = "wasm32")))]
pub mod sse_handler;
#[cfg(target_arch = "wasm32")] pub mod wasm;
#[cfg(all(feature = "event-stream", target_arch = "wasm32"))]
pub mod wasm_event_stream;

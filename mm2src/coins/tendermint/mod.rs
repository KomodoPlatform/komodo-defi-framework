/// Module implementing Tendermint (Cosmos) integration
/// Useful resources
/// https://docs.cosmos.network/
mod tendermint_coin;
#[cfg(not(target_arch = "wasm32"))] mod tendermint_native_rpc;
#[cfg(target_arch = "wasm32")] mod tendermint_wasm_rpc;
pub use tendermint_coin::*;

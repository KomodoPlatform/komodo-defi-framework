#[cfg(not(target_arch = "wasm32"))]
pub mod fs;

pub mod file_lock;

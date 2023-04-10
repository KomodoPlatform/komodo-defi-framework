#[cfg(not(target_arch = "wasm32"))] mod cli;
#[cfg(not(target_arch = "wasm32"))] mod log;
#[cfg(not(target_arch = "wasm32"))] mod scenarios;

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    log::init_logging();
    cli::process_cli();
}

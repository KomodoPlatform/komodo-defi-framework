mod activation_scheme;
mod adex_config;
mod api_commands;
mod cli;
mod data;
mod helpers;
mod log;
mod scenarios;

#[cfg(not(test))]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    log::init_logging();
    cli::process_cli().await;
}

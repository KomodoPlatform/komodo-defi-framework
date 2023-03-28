mod api_commands;
mod cli;
mod data;
mod helpers;
mod log;
mod scenarios;

fn main() {
    log::init_logging();
    cli::process_cli();
}

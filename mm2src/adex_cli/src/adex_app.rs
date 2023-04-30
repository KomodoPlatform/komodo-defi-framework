use crate::adex_config::AdexConfigImpl;
use crate::api_commands::ResponseHandlerImpl;
use crate::cli;
use std::env;

pub struct AdexApp {
    config: AdexConfigImpl,
}

impl AdexApp {
    pub fn new() -> Result<AdexApp, ()> {
        let config = AdexConfigImpl::read_config()?;
        Ok(AdexApp { config })
    }
    pub async fn execute(&self) {
        let response_handler = ResponseHandlerImpl {};
        for arg in env::args() {
            print!("{} ", arg);
        }
        println!("");
        let _ = cli::Cli::execute(env::args(), &self.config, &response_handler).await;
    }
}

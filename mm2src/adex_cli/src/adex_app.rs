use crate::adex_config::AdexConfig;
use crate::api_commands::TablePrinter;
use crate::cli;
use crate::transport::SlurpTransport;
use std::env;

pub struct AdexApp {
    config: AdexConfig,
}

impl AdexApp {
    pub fn new() -> Result<AdexApp, ()> {
        let config = AdexConfig::read_config()?;
        Ok(AdexApp { config })
    }
    pub async fn execute(&self) {
        let printer = TablePrinter {};
        let _ = cli::Cli::execute(env::args(), &self.config, &printer).await;
    }
}

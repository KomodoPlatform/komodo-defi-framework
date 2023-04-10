use crate::adex_config::AdexConfig;
use crate::cli;
use crate::transport::SlurpTransport;
use std::env;

pub struct AdexApp {
    config: AdexConfig,
    transport: SlurpTransport,
}

impl AdexApp {
    pub fn new() -> Result<AdexApp, ()> {
        let config = AdexConfig::read_config()?;
        let rpc_uri = config.rpc_uri();
        Ok(AdexApp {
            config,
            transport: SlurpTransport { rpc_uri },
        })
    }
    pub async fn execute(&self) {
        let _ = cli::Cli::execute(env::args(), &self.transport, self.config.rpc_password()).await;
    }
}

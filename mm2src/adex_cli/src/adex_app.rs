use std::env;
use std::io::Write;

use super::adex_config::AdexConfigImpl;
use super::adex_proc::ResponseHandlerImpl;
use super::cli;

pub(super) struct AdexApp {
    config: AdexConfigImpl,
}

impl AdexApp {
    pub(super) fn new() -> AdexApp {
        let config = AdexConfigImpl::read_config().unwrap_or_default();
        AdexApp { config }
    }

    pub(super) async fn execute(&self) {
        let mut writer = std::io::stdout();
        let response_handler = ResponseHandlerImpl {
            writer: (&mut writer as &mut dyn Write).into(),
        };
        let _ = cli::Cli::execute(env::args(), &self.config, &response_handler).await;
    }
}

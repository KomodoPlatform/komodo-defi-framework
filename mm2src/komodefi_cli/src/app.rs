use std::env;
use std::io::Write;

use super::cli;
use super::config::KomodefiConfigImpl;
use super::komodefi_proc::ResponseHandlerImpl;

pub(super) struct KomodefiApp {
    config: KomodefiConfigImpl,
}

impl KomodefiApp {
    pub(super) fn new() -> KomodefiApp {
        let config = KomodefiConfigImpl::read_config().unwrap_or_default();
        KomodefiApp { config }
    }

    pub(super) async fn execute(&self) {
        let mut writer = std::io::stdout();
        let response_handler = ResponseHandlerImpl {
            writer: (&mut writer as &mut dyn Write).into(),
        };
        let _ = cli::Cli::execute(env::args(), &self.config, &response_handler).await;
    }
}

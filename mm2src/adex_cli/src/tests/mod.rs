use mm2_rpc::mm_protocol::{OrderbookResponse, VersionResponse};

use mockall::{mock, predicate};
use serde_json::Value as Json;
use std::fmt::{Debug, Display};
use std::fs::File;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::adex_config::{AdexConfig, AdexConfigImpl};
use crate::api_commands::{GetEnabledResponse, ResponseHandler};
use crate::cli::Cli;

mock! {
    ResponseHandler {
    }
    impl ResponseHandler for ResponseHandler {
        fn print_response(&self, response: Json) -> Result<(), ()>;
        fn display_response<T: Display + 'static>(&self, response: &T) -> Result<(), ()>;
        fn debug_response<T: Debug + 'static>(&self, response: &T) -> Result<(), ()>;
        fn on_orderbook_response<Cfg: AdexConfig + 'static>(&self, orderbook: OrderbookResponse, config: &Cfg, bids_limit: &Option<usize>,
        asks_limit: &Option<usize>) -> Result<(), ()>;
        fn on_get_enabled_response(&self, enabled: &GetEnabledResponse) -> Result<(), ()>;
    }
}

#[tokio::test]
async fn test_get_version() {
    let file = File::open("src/tests/version.http").unwrap();
    let _ = Command::new("netcat")
        .args(&["-l", "-q", "1", "-p", "7784"])
        .stdin(Stdio::from(file))
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut handler = MockResponseHandler::new();
    let version = VersionResponse {
        result: "1.0.1-beta_824ca36f3".to_string(),
        datetime: "2023-04-06T22:35:43+05:00".to_string(),
    };
    handler
        .expect_display_response()
        .with(predicate::eq(version))
        .returning(|_| Ok(()))
        .times(1);
    let config = AdexConfigImpl::new("dummy", "http://127.0.0.1:7784");
    let args = vec!["adex-cli", "version"];
    let _ = Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &handler).await;
}

use crate::api_commands::Printer;
use crate::api_commands::{Command as AdexCommand, Dummy};
use crate::cli;
use crate::transport::{SlurpTransport, Transport};
use clap::Args;
use futures_util::{TryFuture, TryFutureExt};
use mm2_rpc::mm_protocol::VersionResponse;
use mockall::{automock, mock, predicate, Any};
use serde::{Deserialize, Serialize};
use serde_json::Value::String;
use serde_json::{json, Value as Json};
use std::env;
use std::fmt::Display;
use std::fs::File;
use std::future::Future;
use std::net::ToSocketAddrs;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::task::{Context, Poll};
use std::thread::sleep;
use std::time::Duration;
use tokio::runtime;
use tokio::sync::futures;

mock! {
    Printer {
    }
    impl Printer for Printer {
        fn print_response(&self, result: Json) -> Result<(), ()>;
        fn display_response<T: Display + 'static>(&self, result: T) -> Result<(),()>;
    }
}

#[tokio::test]
async fn test_get_version() {
    let file = File::open("src/tests/version.http").unwrap();
    let _ = Command::new("netcat")
        .args(&["-l", "-q", "1", "-p", "7783"])
        .stdin(Stdio::from(file))
        .spawn()
        .unwrap();

    let mut printer = MockPrinter::new();
    let version = VersionResponse {
        result: "1.0.1-beta_824ca36f3".to_string(),
        datetime: "2023-04-06T22:35:43+05:00".to_string(),
    };
    printer
        .expect_display_response()
        .with(predicate::eq(version))
        .returning(|_| Ok(()))
        .times(1);

    let args = vec!["adex-cli", "version"];
    let _ = cli::Cli::execute(args.iter().map(|arg| arg.to_string()), "password".to_string(), &printer).await;
}

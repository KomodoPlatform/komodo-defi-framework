use crate::cli;
use crate::transport::{SlurpTransport, Transport};
use async_trait::async_trait;
use mockall::{automock, mock, predicate, Any};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::env;
// use std::env;
use clap::Args;
use std::fmt::Display;

mock! {
    SlurpTransport {}     // Name of the mock struct, less the "Mock" prefix
    #[async_trait]
    impl Transport for SlurpTransport {
        async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<Result<OkT, ErrT>, ()>
        where
        ReqT: Serialize + Send + 'static,
        OkT: for<'a> Deserialize<'a> + 'static,
        ErrT: for<'a> Deserialize<'a> + Display + 'static;
    }
}

async fn use_transport(transport: impl Transport) -> Result<Result<u32, u32>, ()> {
    transport.send::<u32, u32, u32>(1).await
}

#[tokio::test]
async fn test_get_version() {
    let mut mock = MockSlurpTransport::new();

    mock.expect_send::<u32, u32, u32>()
        .with(predicate::eq(1))
        .times(1)
        .returning(|x| Ok(Ok(1)));
    cli::Cli::execute(&mock, "asdf".to_string());
}

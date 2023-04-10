// use crate::cli;
// use crate::transport::Transport;
// use async_trait::async_trait;
// use mockall::{automock, mock, predicate, Any};
// use serde::{Deserialize, Serialize};
// use serde_json::{json, Value as Json};
// use std::env;
// // use std::env;
// use crate::api_commands::{Command as AdexCommand, Dummy};
// use clap::Args;
// use mockall_double::double;
// use serde_json::Value::String;
// use std::fmt::Display;
// use std::fs::File;
// use std::process::{Command, Stdio};
//
// // mock! {
// //     SlurpTransport {
// //         pub fn ggbb(&self);
// //     }     // Name of the mock struct, less the "Mock" prefix
// //     #[async_trait]
// //     impl Transport for SlurpTransport {
// //         async fn send<ReqT, OkT, ErrT>(&self, req: ReqT) -> Result<Result<OkT, ErrT>, ()>
// //         where
// //         ReqT: Serialize + Send + 'static,
// //         OkT: for<'a> Deserialize<'a> + 'static,
// //         ErrT: for<'a> Deserialize<'a> + Display + 'static;
// //     }
// // }
//
// #[double] use crate::transport::SlurpTransport;
//
// async fn use_transport(transport: impl Transport) -> Result<Result<u32, u32>, ()> {
//     transport.send::<u32, u32, u32>(1).await
// }
//
// #[tokio::test]
// async fn test_get_version() {
//     let mut mock = MockSlurpTransport::new();
//     mock.gbbb();
//     let file = File::open("src/tests/version.http").unwrap();
//     let _ = Command::new("netcat")
//         .args(&["-l", "-q", "0", "-p", "7783"])
//         .stdin(Stdio::from(file))
//         .spawn()
//         .unwrap();
//
//     mock.expect_send::<AdexCommand<Dummy>, Json, Json>().times(1);
//
//     let args = vec!["adex-cli", "version"];
//     cli::Cli::execute(args.iter().map(|arg| arg.to_string()), &mock, "password".to_string());
// }

use mockall_double::double;

mod mockable {
    #[cfg(test)] use mockall::automock;

    pub struct Foo {}
    #[cfg_attr(test, automock)]
    impl Foo {
        pub fn foo(&self, x: u32) -> u32 {
            println!("asdf asdfa adf");
            x + 1
        }
    }
}

#[double] use mockable::Foo;

fn bar(f: Foo) -> u32 { f.foo(42) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_test() {
        let mut mock = Foo::new();
        mock.expect_foo();
        assert_eq!(43, bar(mock));
    }
}

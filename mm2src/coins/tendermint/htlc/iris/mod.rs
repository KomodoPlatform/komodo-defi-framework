//! IRIS HTLC implementation in Rust on top of Cosmos SDK(cosmrs) for komodo-defi-framework.
//!
//! This module includes HTLC creating & claiming representation structstures
//! and their trait implementations.
//!
//! ** Acquiring testnet assets **
//!
//! Since there is no sdk exists for Rust on Iris Network, we should
//! either implement some of the Iris Network funcionality on Rust or
//! simply use their unit tests.
//!
//! Because we had limited time for the HTLC implementation, for now
//! we can use their unit tests in order to acquire IBC assets.
//! For that, clone https://github.com/onur-ozkan/irishub-sdk-js repository and check
//! dummy.test.ts file(change the asset, amount, target address if needed)
//! and then run the following commands:
//! - yarn
//! - npm run test
//!
//! If the sender address doesn't have enough nyan tokens to complete unit tests,
//! check this page https://www.irisnet.org/docs/get-started/testnet.html#faucet

pub(crate) mod htlc;
pub(crate) mod htlc_proto;

//! # WalletConnect WebSocket Client
//!
//! The WalletConnect client implementation has undergone enhancements to support both WASM and non-WASM targets.
//! Initially, the implementation was restricted to non-WASM targets,
//! which limited its utility in web-based environments.
//!
//! This module contains the WebSocket client code that has been ported and modified to achieve cross-platform compatibility.
//! The changes ensure that the WalletConnect client can function seamlessly in both native
//! and WASM targets.
//!

#[allow(unused)] pub(crate) mod client;
pub(crate) mod error;

extern crate common;
extern crate serde;

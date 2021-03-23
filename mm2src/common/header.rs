//! Indirect routing between crates.
//!
//! Sometimes we need to call downstream, from a dependency and into a dependent crate,
//! such as when calling `mm2::rpc::process_rpc_request` from `common::MarketMakerIt::rpc`.
//! Here we can use C-like headers and/or constructible slots for that.
//!
//! TODO refactor this.

use crate::mm_ctx::MmArc;
use gstuff::Constructible;
use http::request::Parts;
use http::Response;
use serde_json::Value as Json;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

/// Access to `rpc::process_rpc_request` defined downstream.
/// Initialized in `rpc::init_header_slots`.

pub static RPC_SERVICE: Constructible<
    fn(
        ctx: MmArc,
        req: Parts,
        req_json: Json,
        client: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Result<Response<Vec<u8>>, String>> + Send>>,
> = Constructible::new();

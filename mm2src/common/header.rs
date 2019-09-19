//! Indirect routing between crates.
//! 
//! Sometimes we need to call downstream, from a dependency and into a dependent crate,
//! such as when calling `mm2::rpc::rpc_serviceʹ` from `common::MarketMakerIt::rpc`.  
//! Here we can use C-like headers and/or constructible slots for that.

#[cfg(not(feature = "native"))]
use bytes::Bytes;
#[cfg(not(feature = "native"))]
use crate::mm_ctx::MmArc;
#[cfg(not(feature = "native"))]
use futures01::Stream;
#[cfg(not(feature = "native"))]
use gstuff::Constructible;
#[cfg(not(feature = "native"))]
use http::Response;
#[cfg(not(feature = "native"))]
use http::request::Parts;
#[cfg(not(feature = "native"))]
use std::future::Future;

#[cfg(not(feature = "native"))]
use std::net::SocketAddr;

#[cfg(not(feature = "native"))]
use std::pin::Pin;

/// Access to `rpc::rpc_serviceʹ` defined downstream.  
/// Initialized in `rpc::init_header_slots`.
#[cfg(not(feature = "native"))]
pub static RPC_SERVICE: Constructible<
    fn (ctx: MmArc, req: Parts, reqᵇ: Box<dyn Stream<Item=Bytes, Error=String> + Send>, client: SocketAddr)
    -> Pin<Box<dyn Future<Output=Result<Response<Vec<u8>>, String>> + Send>>
> = Constructible::new();

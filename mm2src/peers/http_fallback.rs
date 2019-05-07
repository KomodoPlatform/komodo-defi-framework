use crdts::{CvRDT, Map, Orswot};
use futures::{self, Future};
use gstuff::netstring;
use hashbrown::hash_map::HashMap;
use hyper::{Request, Body};
use hyper::rt::{Stream};
use hyper::service::Service;
use serde_json::{self as json};
use std::io::Write;
use std::net::{SocketAddr};
use std::sync::{Arc, Mutex};
use std::str::from_utf8_unchecked;
use tokio_core::net::TcpListener;

use crate::common::{rpc_response, slurp_req, HyRes, CORE, HTTP};
use crate::common::mm_ctx::{from_ctx, MmArc, MmWeak};

/// Data belonging to this module and owned by the MM2 instance.
pub struct HttpFallbackContext {
    maps: Mutex<HashMap<Vec<u8>, BytesMap>>
}

impl HttpFallbackContext {
    /// Obtains a reference to this mod context, creating it if necessary.
    pub fn from_ctx (ctx: &MmArc) -> Result<Arc<HttpFallbackContext>, String> {
        Ok (try_s! (from_ctx (&ctx.http_fallback_ctx, move || {
            Ok (HttpFallbackContext {
                maps: Mutex::new (HashMap::new())
            })
        })))
    }
}

fn fetch_map_impl (ctx: MmWeak, req: Request<Body>) -> HyRes {
    let f = req.into_body().concat2().then (move |body| -> HyRes {
        let body = try_fus! (body);
        let id = body.to_vec();

        let ctx = try_fus! (MmArc::from_weak (&ctx) .ok_or ("MM stopping"));
        let hfctx = try_fus! (HttpFallbackContext::from_ctx (&ctx));
        let maps = try_fus! (hfctx.maps.lock());
        if let Some (mapʰ) = maps.get (&id) {
            let mapʳ = try_fus! (json::to_string (&mapʰ));
            rpc_response (200, mapʳ)
        } else {
            let map = BytesMap::new();
            let map = try_fus! (json::to_string (&map));
            rpc_response (200, map)
        }
    });
    Box::new (f)
}

fn merge_map_impl (ctx: MmWeak, req: Request<Body>) -> HyRes {
    let f = req.into_body().concat2().then (move |body| -> HyRes {
        let body = try_fus! (body);
        let buf = body.to_vec();
        let (id, mapˢ) = try_fus! (netstring (&buf));
        let map: BytesMap = try_fus! (json::from_slice (mapˢ));

        let ctx = try_fus! (MmArc::from_weak (&ctx) .ok_or ("MM stopping"));
        let hfctx = try_fus! (HttpFallbackContext::from_ctx (&ctx));
        let mut maps = try_fus! (hfctx.maps.lock());
        if let Some (mapʰ) = maps.get_mut (id) {
            mapʰ.merge (map);
            let mapʳ = try_fus! (json::to_string (&mapʰ));
            rpc_response (200, mapʳ)
        } else {
            maps.insert (id.into(), map);
            rpc_response (200, mapˢ.to_vec())
        }
    });
    Box::new (f)
}

/// Creates a Hyper Future that would run the HTTP fallback server.
pub fn new_http_fallback (ctx: MmWeak, addr: SocketAddr) -> Result<Box<Future<Item=(), Error=()>+Send>, String> {
    let listener = try_s! (TcpListener::bind2 (&addr));

    struct RpcService {ctx: MmWeak}
    impl Service for RpcService {
        type ReqBody = Body; type ResBody = Body; type Error = String; type Future = HyRes;
        fn call (&mut self, req: Request<Body>) -> HyRes {
            let path = req.uri().path();
            if path == "/fallback/fetch_map" {
                fetch_map_impl (self.ctx.clone(), req)
            } else if path == "/fallback/merge_map" {
                merge_map_impl (self.ctx.clone(), req)
            } else if path == "/test_ip" {  // Helps `fn test_ip` to check the IP availability.
                rpc_response (200, "k")
            } else {
                rpc_response (404, "unknown path")
            }
        }
    }
    let server = listener.incoming().for_each (move |(socket, _my_sock)| {
        let ctx = ctx.clone();
        CORE.spawn (move |_| HTTP
                .serve_connection (socket, RpcService {ctx})
                .map(|_| ())
                .map_err (|err| log! ({"{}", err})));
        Ok(())
    }) .map_err (|err| log! ({"accept error: {}", err}));

    Ok (Box::new (server))
}

/// CRDT clocks are tracked separately for each actor ID.  
/// If two different clients with the same actor ID are trying simultaneously to change a map
/// then one of the change sets might be ignored due to the clock being in the past.
pub type UniqueActorId = u64;

/// CRDT map from bytes to bytes.  
/// NB: The keys and values must be strings in order for the JSON seriazliation to work.
pub type BytesMap = Map<String, Orswot<String, UniqueActorId>, UniqueActorId>;

fn fallback_url (addr: &SocketAddr, method: &str) -> String {
    fomat! (
        "http" if addr.port() == 443 {'s'} "://"
        (addr.ip())
        if addr.port() != 80 && addr.port() != 443 {':' (addr.port())}
        "/fallback/" (method)
    )
}

/// Fetches a CRDT map stored on the HTTP fallback server.
/// 
/// * `addr` - The address of the HTTP fallback server.
///            The port should be 80 or 443 as this should help the server to function
///            even with the most restrictive internet operators.
pub fn fetch_map (addr: &SocketAddr, id: Vec<u8>) -> Box<Future<Item=BytesMap, Error=String> + Send> {
    let url = fallback_url (addr, "fetch_map");
    let request = try_fus! (Request::builder()
        .method("POST")
        .uri (url)
        .body (Body::from (id)));
    let f = slurp_req (request);
    let f = f.and_then (|(status, _headers, body)| -> Result<BytesMap, String> {
        if status.as_u16() != 200 {return ERR! ("fetch_map not 200")}
        let map: BytesMap = try_s! (json::from_slice (&body));
        Ok (map)
    });
    Box::new (f)
}

/// Merges a CRDT map with the version stored on the HTTP fallback server.
/// 
/// * `addr` - The address of the HTTP fallback server.
///            The port should be 80 or 443 as this should help the server to function
///            even with the most restrictive internet operators.
/// 
/// Returns a fresh version of the map which is provided by the server after the merge.
pub fn merge_map (addr: &SocketAddr, id: Vec<u8>, map: BytesMap)
-> Box<Future<Item=BytesMap, Error=String> + Send> {
    let url = fallback_url (addr, "merge_map");
    let mut map = try_fus! (json::to_vec (&map));

    let mut buf = Vec::with_capacity (id.len() + map.len() + 9);
    try_fus! (write! (&mut buf, "{}:{},", id.len(), unsafe {from_utf8_unchecked (&id)}));
    buf.append (&mut map);

    let request = try_fus! (Request::builder()
        .method("POST")
        .uri (url)
        .body (Body::from (buf)));
    let f = slurp_req (request);
    let f = f.and_then (|(status, _headers, body)| -> Result<BytesMap, String> {
        if status.as_u16() != 200 {return ERR! ("merge_map not 200")}
        let map: BytesMap = try_s! (json::from_slice (&body));
        Ok (map)
    });
    Box::new (f)
}

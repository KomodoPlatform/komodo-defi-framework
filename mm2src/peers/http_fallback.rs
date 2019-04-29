use futures::{self, Future};
use hyper::{Request, Body};
use hyper::rt::{Stream};
use hyper::service::Service;
use std::net::{IpAddr, SocketAddr};
use tokio_core::net::TcpListener;

use crate::common::{rpc_response, HyRes, CORE, HTTP};

/// Creates a Hyper Future that would run the HTTP fallback server.
pub fn new_http_fallback (ip: IpAddr, port: u16) -> Result<Box<Future<Item=(), Error=()>+Send>, String> {
    let bindaddr = SocketAddr::new (ip, port);
    let listener = try_s! (TcpListener::bind2 (&bindaddr));

    struct RpcService;
    impl Service for RpcService {
        type ReqBody = Body; type ResBody = Body; type Error = String; type Future = HyRes;
        fn call (&mut self, _request: Request<Body>) -> HyRes {
            rpc_response (200, "k")
        }
    }
    let server = listener.incoming().for_each (move |(socket, _my_sock)| {
        CORE.spawn (move |_| HTTP
                .serve_connection (socket, RpcService)
                .map(|_| ())
                .map_err (|err| log! ({"test_bind] HTTP error: {}", err})));
        Ok(())
    }) .map_err (|err| log! ({"test_bind] accept error: {}", err}));

    Ok (Box::new (server))
}

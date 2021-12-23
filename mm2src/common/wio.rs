//! `wio` stands for "web I/O", it contains the parts which aren't directly available with WASM.

use futures::compat::Future01CompatExt;
use futures::executor::ThreadPool;
use futures01::sync::oneshot::{self, Receiver};
use futures01::{Async, Future, Poll};
use futures_cpupool::CpuPool;
use gstuff::{duration_to_float, now_float};
use hyper::client::HttpConnector;
use hyper::Client;
use hyper_rustls::HttpsConnector;
use std::fmt;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::runtime::Runtime;

fn start_core_thread() -> Mm2Runtime { Mm2Runtime(Runtime::new().unwrap()) }

pub struct Mm2Runtime(pub Runtime);

lazy_static! {
    /// Shared asynchronous reactor.
    pub static ref CORE: Mm2Runtime = start_core_thread();
    /// Shared CPU pool to run intensive/sleeping requests on a separate thread.
    ///
    /// Deprecated, prefer the futures 0.3 `POOL` instead.
    pub static ref CPUPOOL: CpuPool = CpuPool::new(8);
    /// Shared CPU pool to run intensive/sleeping requests on s separate thread.
    pub static ref POOL: Mutex<ThreadPool> = Mutex::new(ThreadPool::builder()
        .pool_size(8)
        .name_prefix("POOL")
        .create().expect("!ThreadPool"));
}

impl<Fut: std::future::Future<Output = ()> + Send + 'static> hyper::rt::Executor<Fut> for &Mm2Runtime {
    fn execute(&self, fut: Fut) { self.0.spawn(fut); }
}

/// With a shared reactor drives the future `f` to completion.
///
/// NB: This function is only useful if you need to get the results of the execution.
/// If the results are not necessary then a future can be scheduled directly on the reactor:
///
///     CORE.spawn (|_| f);
pub fn drive<F, R, E>(f: F) -> Receiver<Result<R, E>>
where
    F: Future<Item = R, Error = E> + Send + 'static,
    R: Send + 'static,
    E: Send + 'static,
{
    let (sx, rx) = oneshot::channel();
    CORE.0.spawn(
        f.then(move |fr: Result<R, E>| -> Result<(), ()> {
            let _ = sx.send(fr);
            Ok(())
        })
        .compat(),
    );
    rx
}

pub fn drive03<F, O>(f: F) -> futures::channel::oneshot::Receiver<O>
where
    F: std::future::Future<Output = O> + Send + 'static,
    O: Send + 'static,
{
    let (sx, rx) = futures::channel::oneshot::channel();
    CORE.0.spawn(async move {
        let res = f.await;
        if sx.send(res).is_err() {
            log!("drive03 receiver is dropped");
        };
    });
    rx
}

/// With a shared reactor drives the future `f` to completion.
///
/// Similar to `fn drive`, but returns a stringified error,
/// allowing us to collapse the `Receiver` and return the `R` directly.
pub fn drive_s<F, R, E>(f: F) -> impl Future<Item = R, Error = String>
where
    F: Future<Item = R, Error = E> + Send + 'static,
    R: Send + 'static,
    E: fmt::Display + Send + 'static,
{
    drive(f).then(move |r| -> Result<R, String> {
        let r = try_s!(r); // Peel the `Receiver`.
        let r = try_s!(r); // `E` to `String`.
        Ok(r)
    })
}

/// Finishes with the "timeout" error if the underlying future isn't ready withing the given timeframe.
///
/// NB: Tokio timers (in `tokio::timer`) only seem to work under the Tokio runtime,
/// which is unfortunate as we want the different futures executed on the different reactors
/// depending on how much they're I/O-bound, CPU-bound or blocking.
/// Unlike the Tokio timers this `Timeout` implementation works with any reactor.
/// Another option to consider is https://github.com/alexcrichton/futures-timer.
/// P.S. The older `0.1` version of the `tokio::timer` might work NP, it works in other parts of our code.
///      The new version, on the other hand, requires the Tokio runtime (https://tokio.rs/blog/2018-03-timers/).
/// P.S. We could try using the `futures-timer` crate instead, but note that it is currently under-maintained,
///      https://github.com/rustasync/futures-timer/issues/9#issuecomment-400802515.
pub struct Timeout<R> {
    fut: Box<dyn Future<Item = R, Error = String>>,
    started: f64,
    timeout: f64,
    monitor: Option<JoinHandle<()>>,
}
impl<R> Future for Timeout<R> {
    type Item = R;
    type Error = String;
    fn poll(&mut self) -> Poll<R, String> {
        match self.fut.poll() {
            Err(err) => Err(err),
            Ok(Async::Ready(r)) => Ok(Async::Ready(r)),
            Ok(Async::NotReady) => {
                let now = now_float();
                if now >= self.started + self.timeout {
                    Err(format!("timeout ({:.1} > {:.1})", now - self.started, self.timeout))
                } else {
                    // Start waking up this future until it has a chance to timeout.
                    // For now it's just a basic separate thread. Will probably optimize later.
                    if self.monitor.is_none() {
                        let task = futures01::task::current();
                        let deadline = self.started + self.timeout;
                        self.monitor = Some(
                            std::thread::Builder::new()
                                .name("timeout monitor".into())
                                .spawn(move || loop {
                                    std::thread::sleep(Duration::from_secs(1));
                                    task.notify();
                                    if now_float() > deadline + 2. {
                                        break;
                                    }
                                })
                                .unwrap(),
                        );
                    }
                    Ok(Async::NotReady)
                }
            },
        }
    }
}
impl<R> Timeout<R> {
    pub fn new(fut: Box<dyn Future<Item = R, Error = String>>, timeout: Duration) -> Timeout<R> {
        Timeout {
            fut,
            started: now_float(),
            timeout: duration_to_float(timeout),
            monitor: None,
        }
    }
}

unsafe impl<R> Send for Timeout<R> where Box<dyn Future<Item = R, Error = String>>: Send {}

/// Initialize the crate.
pub fn init() {
    // Pre-allocate the stack trace buffer in order to avoid allocating it from a signal handler.
    super::black_box(&*super::trace_buf());
    super::black_box(&*super::trace_name_buf());
}

lazy_static! {
    /// NB: With a shared client there is a possibility that keep-alive connections will be reused.
    pub static ref HYPER: Client<HttpsConnector<HttpConnector>> = {
        // Please note there was a problem on iOS if [`HttpsConnector::with_native_roots`] is used instead.
        let https = HttpsConnector::with_webpki_roots();
        Client::builder()
            .executor(&*CORE)
            // Hyper had a lot of Keep-Alive bugs over the years and I suspect
            // that with the shared client we might be getting errno 10054
            // due to a closed Keep-Alive connection mismanagement.
            // (To solve this problem Hyper should proactively close the Keep-Alive
            // connections after a configurable amount of time has passed since
            // their creation, thus saving us from trying to use the connections
            // closed on the other side. I wonder if we can implement this strategy
            // ourselves with a custom connector or something).
            // Performance of Keep-Alive in the Hyper client is questionable as well,
            // should measure it on a case-by-case basis when we need it.
            .pool_max_idle_per_host(0)
            .build(https)
    };
}

use crate::executor::abortable_system::{InnerShared, SystemInner};
use crate::executor::AbortableSystem;
use futures::channel::oneshot;
use futures::FutureExt;
use std::future::Future;

/// This is an `AbortableSystem` that initiates listeners for graceful shutdown
/// once the `GracefulShutdownRegistry` instance is dropped.
///
/// `GracefulShutdownRegistry` can be used in conjunction with the `spawn` method.
/// In some cases, the use of `GracefulShutdownRegistry` and `spawn` is justified.
/// For example, [`hyper::Server::with_graceful_shutdown`].
#[derive(Default)]
pub struct GracefulShutdownRegistry {
    inner: InnerShared<ShutdownInner>,
}

impl GracefulShutdownRegistry {
    /// Registers a graceful shutdown listener and returns a future
    /// that acts as a signal for graceful shutdown.
    pub fn register_listener(&self) -> impl Future<Output = ()> + Send + Sync + 'static {
        let (tx, rx) = oneshot::channel();
        self.inner.lock().insert_handle(tx);
        rx.then(|_| futures::future::ready(()))
    }
}

impl From<InnerShared<ShutdownInner>> for GracefulShutdownRegistry {
    fn from(inner: InnerShared<ShutdownInner>) -> Self { GracefulShutdownRegistry { inner } }
}

impl AbortableSystem for GracefulShutdownRegistry {
    type Inner = ShutdownInner;

    fn abort_all(&self) { self.inner.lock().abort_all() }

    fn __push_subsystem_abort_tx(&self, subsystem_abort_tx: oneshot::Sender<()>) {
        self.inner.lock().insert_handle(subsystem_abort_tx)
    }
}

#[derive(Default)]
pub struct ShutdownInner {
    abort_handlers: Vec<oneshot::Sender<()>>,
}

impl ShutdownInner {
    fn insert_handle(&mut self, handle: oneshot::Sender<()>) { self.abort_handlers.push(handle); }
}

impl SystemInner for ShutdownInner {
    fn abort_all(&mut self) { self.abort_handlers.clear(); }
}

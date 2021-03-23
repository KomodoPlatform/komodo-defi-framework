use futures::Future;
use lazy_static::lazy_static;
use std::pin::Pin;

lazy_static! {
    pub static ref SWARM_RUNTIME: SwarmRuntime = SwarmRuntime::new();
}

pub struct SwarmRuntime {
    #[cfg(not(target_arch = "wasm32"))]
    inner: tokio::runtime::Runtime,
}

pub trait SwarmRuntimeOps {
    fn new() -> Self;

    fn spawn<F>(&self, future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;
}

#[cfg(target_arch = "wasm32")]
impl SwarmRuntimeOps for SwarmRuntime {
    fn new() -> Self { SwarmRuntime {} }

    fn spawn<F>(&self, _future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        todo!()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SwarmRuntimeOps for SwarmRuntime {
    fn new() -> Self {
        SwarmRuntime {
            inner: tokio::runtime::Runtime::new().unwrap(),
        }
    }

    fn spawn<F>(&self, future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.inner.spawn(future);
    }
}

impl libp2p::core::Executor for &SwarmRuntime {
    fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) { self.spawn(future) }
}

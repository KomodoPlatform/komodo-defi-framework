use futures::future::{abortable, AbortHandle};
use futures::{Future as Future03, FutureExt};

#[cfg(not(target_arch = "wasm32"))] mod native_executor;
#[cfg(target_arch = "wasm32")] mod wasm_executor;

#[cfg(not(target_arch = "wasm32"))]
pub use native_executor::{spawn, spawn_after, spawn_boxed, Timer};
#[cfg(target_arch = "wasm32")]
pub use wasm_executor::{spawn, spawn_boxed, spawn_local, Timer};

/// The AbortHandle that aborts on drop
pub struct AbortOnDropHandle(Option<AbortHandle>);

impl From<AbortHandle> for AbortOnDropHandle {
    fn from(handle: AbortHandle) -> Self { AbortOnDropHandle(Some(handle)) }
}

impl AbortOnDropHandle {
    pub fn into_handle(mut self) -> AbortHandle { self.0.take().expect("`AbortHandle` Must be initialized") }
}

impl Drop for AbortOnDropHandle {
    #[inline(always)]
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

#[must_use]
pub fn spawn_abortable(fut: impl Future03<Output = ()> + Send + 'static) -> AbortOnDropHandle {
    let (abortable, handle) = abortable(fut);
    spawn(abortable.then(|_| async {}));
    AbortOnDropHandle::from(handle)
}

#[must_use]
pub fn spawn_abortable_with_msg(fut: impl Future03<Output = ()> + Send + 'static, msg: String) -> AbortOnDropHandle {
    let (abortable, handle) = abortable(fut);
    spawn(async move {
        if let Err(_aborted) = abortable.await {
            log::info!("{}", msg);
        }
    });
    AbortOnDropHandle::from(handle)
}

use futures::future::abortable;
use futures::{Future as Future03, FutureExt};

#[cfg(not(target_arch = "wasm32"))] mod native_executor;
#[cfg(not(target_arch = "wasm32"))]
pub use native_executor::{spawn, spawn_after, spawn_boxed, Timer};

mod spawner;
pub use spawner::{AbortableSpawner, AbortableSpawnerShared};

mod abort_on_drop;
pub use abort_on_drop::AbortOnDropHandle;

#[cfg(target_arch = "wasm32")] mod wasm_executor;
#[cfg(target_arch = "wasm32")]
pub use wasm_executor::{spawn, spawn_boxed, spawn_local, Timer};

#[must_use]
pub fn spawn_abortable(fut: impl Future03<Output = ()> + Send + 'static) -> AbortOnDropHandle {
    let (abortable, handle) = abortable(fut);
    spawn(abortable.then(|_| async {}));
    AbortOnDropHandle::from(handle)
}

#[must_use]
pub fn spawn_with_msg_on_abort(
    fut: impl Future03<Output = ()> + Send + 'static,
    level: log::Level,
    msg: String,
) -> AbortOnDropHandle {
    let (abortable, handle) = abortable(fut);
    spawn(async move {
        if let Err(_aborted) = abortable.await {
            log::log!(level, "{}", msg);
        }
    });
    AbortOnDropHandle::from(handle)
}

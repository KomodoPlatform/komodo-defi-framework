use futures::future::abortable;
use futures::{Future as Future03, FutureExt};

#[cfg(not(target_arch = "wasm32"))] mod native_executor;
#[cfg(not(target_arch = "wasm32"))]
pub use native_executor::{spawn, Timer};

mod abortable_spawner;
pub use abortable_spawner::{AbortableSpawner, AbortableSpawnerWeak, SpawnAbortable};

mod spawner;
pub use spawner::{BoxFutureSpawner, SpawnFuture};

mod abort_on_drop;
pub use abort_on_drop::AbortOnDropHandle;

#[cfg(target_arch = "wasm32")] mod wasm_executor;
#[cfg(target_arch = "wasm32")]
pub use wasm_executor::{spawn, spawn_local, Timer};

#[derive(Default)]
pub struct SpawnSettings {
    on_finish: Option<SpawnMsg>,
    on_abort: Option<SpawnMsg>,
}

impl SpawnSettings {
    pub fn info_on_any_stop(msg: String) -> SpawnSettings {
        let msg = SpawnMsg {
            level: log::Level::Info,
            msg,
        };
        SpawnSettings {
            on_finish: Some(msg.clone()),
            on_abort: Some(msg),
        }
    }

    pub fn info_on_finish(msg: String) -> SpawnSettings {
        let msg = SpawnMsg {
            level: log::Level::Info,
            msg,
        };
        SpawnSettings {
            on_finish: Some(msg),
            on_abort: None,
        }
    }

    pub fn info_on_abort(msg: String) -> SpawnSettings {
        let msg = SpawnMsg {
            level: log::Level::Info,
            msg,
        };
        SpawnSettings {
            on_finish: None,
            on_abort: Some(msg),
        }
    }
}

#[derive(Clone)]
struct SpawnMsg {
    level: log::Level,
    msg: String,
}

#[must_use]
pub fn spawn_abortable(fut: impl Future03<Output = ()> + Send + 'static) -> AbortOnDropHandle {
    let (abortable, handle) = abortable(fut);
    unsafe { spawn(abortable.then(|_| async {})) }
    AbortOnDropHandle::from(handle)
}

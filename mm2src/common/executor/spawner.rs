use crate::executor::{spawn, spawn_abortable, spawn_with_msg_on_abort, AbortOnDropHandle, Timer};
use futures::channel::oneshot;
use futures::Future as Future03;
use parking_lot::Mutex as PaMutex;
use std::sync::Arc;

const DEFAULT_CRITICAL_TIMEOUT_S: f64 = 1.;

pub type AbortableSpawnerShared = Arc<AbortableSpawner>;

/// Future spawner that ensures that the spawned futures will be aborted immediately
/// or after a [`AbortableSpawner::critical_timeout_s`] timeout
/// once an `AbortableSpawner` instance is dropped.
pub struct AbortableSpawner {
    abort_handlers: PaMutex<Vec<AbortOnDropHandle>>,
    critical_handlers: PaMutex<Vec<oneshot::Sender<()>>>,
    critical_timeout_s: f64,
}

impl Default for AbortableSpawner {
    fn default() -> Self { AbortableSpawner::new() }
}

impl AbortableSpawner {
    pub fn new() -> AbortableSpawner { AbortableSpawner::with_critical_timeout(DEFAULT_CRITICAL_TIMEOUT_S) }

    pub fn with_critical_timeout(critical_timeout_s: f64) -> AbortableSpawner {
        AbortableSpawner {
            abort_handlers: PaMutex::new(Vec::new()),
            critical_handlers: PaMutex::new(Vec::new()),
            critical_timeout_s,
        }
    }

    pub fn into_shared(self) -> AbortableSpawnerShared { Arc::new(self) }

    /// Spawns the `fut` future.
    /// The future will be stopped immediately if `AbortableSpawner` is dropped.
    pub fn spawn<F>(&self, fut: F)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        let abort_handle = spawn_abortable(fut);
        self.abort_handlers.lock().push(abort_handle);
    }

    /// Spawns the `fut` future with the `msg` message that will be printed once the future is stopped.
    /// The future will be stopped immediately if `AbortableSpawner` is dropped.
    pub fn spawn_with_msg_on_abort<F>(&self, fut: F, level: log::Level, msg: String)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        let abort_handle = spawn_with_msg_on_abort(fut, level, msg);
        self.abort_handlers.lock().push(abort_handle);
    }

    /// Register `abort_handle` of a spawned future.
    pub fn register_spawned(&self, abort_handle: AbortOnDropHandle) { self.abort_handlers.lock().push(abort_handle); }

    /// Spawns the `fut` future for which it's critical to complete the execution,
    /// or at least try to complete.
    /// The future will be stopped after the specified [`AbortableSpawner::critical_timeout_s`] timeout.
    pub fn spawn_critical<F>(&self, fut: F)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let critical_timeout_s = self.critical_timeout_s;

        let abort_handle = spawn_abortable(fut);
        let timeout_fut = async move {
            // First, wait for the corresponding [`AbortableSpawner::critical_handlers`] sender (aka `tx`) is dropped.
            let _ = rx.await;

            // Then give the `fut` future to try to complete in [`AbortableSpawner::critical_timeout_s`] seconds.
            Timer::sleep(critical_timeout_s).await;

            // Abort the given `fut` future.
            drop(abort_handle);
        };

        // Spawn the timeout future globally, since we're sure that it will be aborted if `tx` is dropped.
        spawn(timeout_fut);
        self.critical_handlers.lock().push(tx);
    }

    /// Aborts all spawned [`AbortableSpawner::abort_handlers`] futures,
    /// and initiates aborting of critical [`AbortableSpawner::critical_handlers`] futures
    /// after the specified [`AbortableSpawner::critical_timeout_s`].
    pub fn abort_all(&self) {
        self.abort_handlers.lock().clear();
        self.critical_handlers.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    #[test]
    fn test_spawn_critical() {
        static mut F1_FINISHED: bool = false;
        static mut F2_FINISHED: bool = false;

        let spawner = AbortableSpawner::with_critical_timeout(0.3);

        let fut1 = async move {
            Timer::sleep(0.5).await;
            unsafe { F1_FINISHED = true };
        };
        spawner.spawn_critical(fut1);

        let fut2 = async move {
            Timer::sleep(0.2).await;
            unsafe { F2_FINISHED = true };
        };
        spawner.spawn_critical(fut2);

        drop(spawner);

        block_on(Timer::sleep(1.));
        // `fut1` must not complete.
        assert!(unsafe { !F1_FINISHED });
        // `fut` must complete.
        assert!(unsafe { F2_FINISHED });
    }
}

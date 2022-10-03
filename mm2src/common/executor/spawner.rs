use crate::executor::{spawn, AbortOnDropHandle, SpawnSettings, Timer};
use futures::channel::oneshot;
use futures::future::{abortable, select, Either};
use futures::{Future as Future03, FutureExt};
use parking_lot::Mutex as PaMutex;
use std::sync::Arc;

const DEFAULT_CRITICAL_TIMEOUT_S: f64 = 1.;
const CAPACITY: usize = 1024;

pub type AbortableSpawnerShared = Arc<AbortableSpawner>;

type FutureId = usize;
type SpawnedFuturesShared<Handle> = Arc<PaMutex<SpawnedFutures<Handle>>>;

pub trait BoxFutureSpawner {
    fn spawn_boxed(&self, f: Box<dyn Future03<Output = ()> + Send + Unpin + 'static>);
}

impl<S: FutureSpawner> BoxFutureSpawner for S {
    fn spawn_boxed(&self, f: Box<dyn Future03<Output = ()> + Send + Unpin + 'static>) { self.spawn(f) }
}

pub trait FutureSpawner {
    /// Spawns the given `f` future.
    fn spawn<F>(&self, f: F)
    where
        F: Future03<Output = ()> + Send + 'static;
}

/// Future spawner that ensures that the spawned futures will be aborted immediately
/// or after a [`AbortableSpawner::critical_timeout_s`] timeout
/// once an `AbortableSpawner` instance is dropped.
pub struct AbortableSpawner {
    abort_handlers: SpawnedFuturesShared<AbortOnDropHandle>,
    critical_handlers: SpawnedFuturesShared<oneshot::Sender<()>>,
    critical_timeout_s: f64,
}

impl AbortableSpawner {
    pub fn new() -> AbortableSpawner { AbortableSpawner::with_critical_timeout(DEFAULT_CRITICAL_TIMEOUT_S) }

    pub fn with_critical_timeout(critical_timeout_s: f64) -> AbortableSpawner {
        AbortableSpawner {
            abort_handlers: Arc::new(PaMutex::new(SpawnedFutures::new())),
            critical_handlers: Arc::new(PaMutex::new(SpawnedFutures::new())),
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
        self.spawn_with_settings(fut, SpawnSettings::default())
    }

    /// Spawns the `fut` future with the specified `settings`.
    /// The future will be stopped immediately if `AbortableSpawner` is dropped.
    pub fn spawn_with_settings<F>(&self, fut: F, settings: SpawnSettings)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        let (abortable, handle) = abortable(fut);
        let future_id = self.abort_handlers.lock().insert_handle(handle.into());

        let weak_handlers = Arc::downgrade(&self.abort_handlers);

        let fut = async move {
            match abortable.await {
                // The future has finished normally.
                Ok(_) => {
                    if let Some(on_finish) = settings.on_finish {
                        log::log!(on_finish.level, "{}", on_finish.msg);
                    }

                    if let Some(handlers) = weak_handlers.upgrade() {
                        handlers.lock().remove_finished(future_id);
                    }
                },
                // The future has been aborted.
                // Corresponding future handle seems to be dropped at the `SpawnedFutures`,
                // so we don't need to [`SpawnedFutures::remove_finished`].
                Err(_) => {
                    if let Some(on_abort) = settings.on_abort {
                        log::log!(on_abort.level, "{}", on_abort.msg);
                    }
                },
            }
        };
        unsafe { spawn(fut) };
    }

    /// Spawns the `fut` future for which it's critical to complete the execution,
    /// or at least try to complete.
    /// The future will be stopped after the specified [`AbortableSpawner::critical_timeout_s`] timeout.
    pub fn spawn_critical<F>(&self, fut: F)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        self.spawn_critical_with_settings(fut, SpawnSettings::default())
    }

    /// Spawns the `fut` future for which it's critical to complete the execution,
    /// or at least try to complete.
    /// The future will be stopped after the specified [`AbortableSpawner::critical_timeout_s`] timeout.
    pub fn spawn_critical_with_settings<F>(&self, fut: F, settings: SpawnSettings)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        let (abortable_fut, abort_handle) = abortable(fut);

        let (tx, rx) = oneshot::channel();
        let future_id = self.critical_handlers.lock().insert_handle(tx);

        let critical_timeout_s = self.critical_timeout_s;
        let weak_handlers = Arc::downgrade(&self.critical_handlers);

        let final_future = async move {
            let wait_till_abort = async move {
                // First, wait for the `tx` sender (i.e. corresponding [`AbortableSpawner::critical_handlers`] item) is dropped.
                rx.await.ok();

                // Then give the `fut` future to try to complete in `critical_timeout_s` seconds.
                Timer::sleep(critical_timeout_s).await;
            };

            match select(abortable_fut.boxed(), wait_till_abort.boxed()).await {
                // The future has finished normally.
                Either::Left(_) => {
                    if let Some(on_finish) = settings.on_finish {
                        log::log!(on_finish.level, "{}", on_finish.msg);
                    }

                    // We need to remove the future ID if the handler still exists.
                    if let Some(handlers) = weak_handlers.upgrade() {
                        handlers.lock().remove_finished(future_id);
                    }
                },
                // `tx` has been removed from [`AbortableSpawner::critical_handlers`], *and* the `critical_timeout_s` timeout has expired.
                Either::Right(_) => {
                    if let Some(on_abort) = settings.on_abort {
                        log::log!(on_abort.level, "{}", on_abort.msg);
                    }

                    // Abort the input `fut`.
                    abort_handle.abort();
                },
            }
        };

        unsafe { spawn(final_future) };
    }

    /// Aborts all spawned [`AbortableSpawner::abort_handlers`] futures,
    /// and initiates aborting of critical [`AbortableSpawner::critical_handlers`] futures
    /// after the specified [`AbortableSpawner::critical_timeout_s`].
    pub fn abort_all(&self) {
        self.abort_handlers.lock().clear();
        self.critical_handlers.lock().clear();
    }
}

impl Default for AbortableSpawner {
    fn default() -> Self { AbortableSpawner::new() }
}

impl FutureSpawner for AbortableSpawner {
    fn spawn<F>(&self, f: F)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        self.spawn(f)
    }
}

/// `SpawnedFutures` is the container of the spawned future handles `FutureHandle`.
/// It holds the future handles, gives every future its *unique* `FutureId` identifier
/// (unique between spawned and alive futures).
/// One a future is finished, its `FutureId` can be reassign to another future.
/// This is necessary so that this container does not grow indefinitely.
/// Such `FutureId` identifier is used to remove `FutureHandle` associated with a finished future.
struct SpawnedFutures<FutureHandle> {
    abort_handlers: Vec<FutureHandle>,
    finished_futures: Vec<FutureId>,
}

impl<FutureHandle> Default for SpawnedFutures<FutureHandle> {
    fn default() -> Self { SpawnedFutures::new() }
}

impl<FutureHandle> SpawnedFutures<FutureHandle> {
    fn new() -> Self {
        SpawnedFutures {
            abort_handlers: Vec::with_capacity(CAPACITY),
            finished_futures: Vec::with_capacity(CAPACITY),
        }
    }

    /// Inserts the given `handle`.
    fn insert_handle(&mut self, handle: FutureHandle) -> FutureId {
        match self.finished_futures.pop() {
            Some(finished_id) => {
                self.abort_handlers[finished_id] = handle;
                // The freed future ID.
                finished_id
            },
            None => {
                self.abort_handlers.push(handle);
                // The last item ID.
                self.abort_handlers.len() - 1
            },
        }
    }

    /// [`SpawnedFuturesContainer::remove_finished`] is used internally only.
    ///
    /// # Note
    ///
    /// We don't need to remove an associated `FutureHandle`,
    /// but later we can easily reset the item at `abort_handlers[future_id]` with a new `FutureHandle`.
    fn remove_finished(&mut self, future_id: FutureId) { self.finished_futures.push(future_id); }

    fn clear(&mut self) {
        self.abort_handlers.clear();
        self.finished_futures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    macro_rules! test_spawn_removes_when_finished_impl {
        ($handlers:ident, $fun:ident) => {
            let spawner = AbortableSpawner::with_critical_timeout(0.3);

            spawner.$fun(async {});
            block_on(Timer::sleep(0.1));

            {
                let mng = spawner.$handlers.lock();
                assert_eq!(mng.abort_handlers.len(), 1);
                // The future should have finished already.
                assert_eq!(mng.finished_futures.len(), 1);
            }

            let fut1 = async { Timer::sleep(0.3).await };
            let fut2 = async { Timer::sleep(0.7).await };
            spawner.$fun(fut1);
            spawner.$fun(fut2);

            {
                let mng = spawner.$handlers.lock();
                // `abort_handlers` should be extended once
                // because `finished_futures` contained only one freed `FutureId`.
                assert_eq!(mng.abort_handlers.len(), 2);
                // `FutureId` should be used from `finished_futures` container.
                assert!(mng.finished_futures.is_empty());
            }

            block_on(Timer::sleep(0.5));

            {
                let mng = spawner.$handlers.lock();
                assert_eq!(mng.abort_handlers.len(), 2);
                assert_eq!(mng.finished_futures.len(), 1);
            }

            block_on(Timer::sleep(0.4));

            {
                let mng = spawner.$handlers.lock();
                assert_eq!(mng.abort_handlers.len(), 2);
                assert_eq!(mng.finished_futures.len(), 2);
            }
        };
    }

    #[test]
    fn test_spawn_critical_removes_when_finished() {
        test_spawn_removes_when_finished_impl!(critical_handlers, spawn_critical);
    }

    #[test]
    fn test_spawn_removes_when_finished() {
        test_spawn_removes_when_finished_impl!(abort_handlers, spawn);
    }

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

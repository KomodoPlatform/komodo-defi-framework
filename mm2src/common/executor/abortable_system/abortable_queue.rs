use crate::executor::abortable_system::{AbortableSystem, InnerShared, InnerWeak, SystemInner};
use crate::executor::spawner::{SpawnAbortable, SpawnFuture};
use crate::executor::{spawn, AbortSettings, Timer};
use futures::channel::oneshot;
use futures::future::{abortable, select, Either};
use futures::FutureExt;
use std::future::Future as Future03;
use std::sync::Arc;

const CAPACITY: usize = 1024;

type FutureId = usize;

/// This is an `AbortableSystem` that ensures that the spawned futures will be aborted
/// once the `AbortableQueue` instance is dropped.
///
/// `AbortableQueue` is responsible for storing future handles in `QueueInner`
/// and deleting them as soon as they complete.
#[derive(Debug, Default)]
pub struct AbortableQueue {
    inner: InnerShared<QueueInner>,
}

impl AbortableQueue {
    /// Returns `WeakSpawner` that will not prevent the spawned futures from being aborted.
    /// This is the only way to create a `'static` instance pointing to the same `QueueInner`
    /// that can be passed into spawned futures, since `AbortableQueue` doesn't implement `Clone`.
    pub fn weak_spawner(&self) -> WeakSpawner {
        WeakSpawner {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

impl From<InnerShared<QueueInner>> for AbortableQueue {
    fn from(inner: InnerShared<QueueInner>) -> Self { AbortableQueue { inner } }
}

impl AbortableSystem for AbortableQueue {
    type Inner = QueueInner;

    /// Aborts all spawned futures and initiates aborting of critical futures
    /// after the specified [`AbortSettings::critical_timeout_s`].
    fn abort_all(&self) { self.inner.lock().abort_all(); }

    fn __push_subsystem_abort_tx(&self, subsystem_abort_tx: oneshot::Sender<()>) {
        self.inner.lock().insert_handle(subsystem_abort_tx);
    }
}

/// `WeakSpawner` doesn't prevent the spawned futures from being aborted.
/// An instance of `WeakSpawner` can be safely passed into spawned futures.
///
/// # Important
///
/// If corresponding `AbortableQueue` instance is dropped, [`WeakSpawner::spawn`] won't
/// actually spawn the future as it's more likely that the program, or part of the program,
/// ends its work, and there is no need to execute tasks that are no longer relevant.
#[derive(Clone)]
pub struct WeakSpawner {
    inner: InnerWeak<QueueInner>,
}

impl SpawnFuture for WeakSpawner {
    fn spawn<F>(&self, f: F)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        self.spawn_with_settings(f, AbortSettings::default())
    }
}

impl SpawnAbortable for WeakSpawner {
    /// Spawns the `fut` future with the specified abort `settings`.
    /// The future won't be executed if `AbortableQueue` is dropped.
    fn spawn_with_settings<F>(&self, fut: F, settings: AbortSettings)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        let (abort_tx, abort_rx) = oneshot::channel();
        let future_id = match self.inner.upgrade() {
            Some(inner_arc) => inner_arc.lock().insert_handle(abort_tx),
            None => return,
        };

        let inner_weak = self.inner.clone();
        let (abortable_fut, abort_handle) = abortable(fut);

        let final_fut = async move {
            let critical_timeout_s = settings.critical_timeout_s;

            let wait_till_abort = async move {
                // First, wait for the `abort_tx` sender (i.e. corresponding [`QueueInner::futures`] item) is dropped.
                abort_rx.await.ok();

                // If the `critical_timeout_s` is set, give the `fut` future to try
                // to complete in `critical_timeout_s` seconds.
                if let Some(critical_timeout_s) = critical_timeout_s {
                    Timer::sleep(critical_timeout_s).await;
                }
            };

            match select(abortable_fut.boxed(), wait_till_abort.boxed()).await {
                // The future has finished normally.
                Either::Left(_) => {
                    if let Some(on_finish) = settings.on_finish {
                        log::log!(on_finish.level, "{}", on_finish.msg);
                    }

                    // We need to remove the future ID if the handler still exists.
                    if let Some(inner) = inner_weak.upgrade() {
                        inner.lock().on_finished(future_id);
                    }
                },
                // `abort_tx` has been removed from `QueueInner::futures`,
                // *and* the `critical_timeout_s` timeout has expired (if was specified).
                Either::Right(_) => {
                    if let Some(on_abort) = settings.on_abort {
                        log::log!(on_abort.level, "{}", on_abort.msg);
                    }

                    // Abort the input `fut`.
                    abort_handle.abort();
                },
            }
        };

        spawn(final_fut);
    }
}

/// `QueueInner` is the container of the spawned future handles [`oneshot::Sender<()>`].
/// It holds the future handles, gives every future its *unique* `FutureId` identifier
/// (unique between spawned and alive futures).
/// Once a future is finished, its `FutureId` can be reassign to another future.
/// This is necessary so that this container does not grow indefinitely.
#[derive(Debug)]
pub struct QueueInner {
    abort_handlers: Vec<oneshot::Sender<()>>,
    finished_futures: Vec<FutureId>,
}

impl Default for QueueInner {
    fn default() -> Self {
        QueueInner {
            abort_handlers: Vec::with_capacity(CAPACITY),
            finished_futures: Vec::with_capacity(CAPACITY),
        }
    }
}

impl QueueInner {
    /// Inserts the given future `handle`.
    fn insert_handle(&mut self, handle: oneshot::Sender<()>) -> FutureId {
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

    /// Handles the fact that the future associated with the `future_id` has been finished.
    ///
    /// # Note
    ///
    /// We don't need to remove an associated [`oneshot::Sender<()>`],
    /// but later we can easily reset the item at `abort_handlers[future_id]` with a new [`oneshot::Sender<()>`].
    fn on_finished(&mut self, future_id: FutureId) { self.finished_futures.push(future_id); }
}

impl SystemInner for QueueInner {
    fn abort_all(&mut self) {
        self.abort_handlers.clear();
        self.finished_futures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    fn test_future_finished_impl(settings: AbortSettings) {
        let abortable_system = AbortableQueue::default();
        let spawner = abortable_system.weak_spawner();

        spawner.spawn_with_settings(async {}, settings.clone());
        block_on(Timer::sleep(0.1));

        {
            let inner = abortable_system.inner.lock();
            assert_eq!(inner.abort_handlers.len(), 1);
            // The future should have finished already.
            assert_eq!(inner.finished_futures.len(), 1);
        }

        let fut1 = async { Timer::sleep(0.3).await };
        let fut2 = async { Timer::sleep(0.7).await };
        spawner.spawn_with_settings(fut1, settings.clone());
        spawner.spawn_with_settings(fut2, settings.clone());

        {
            let inner = abortable_system.inner.lock();
            // `abort_handlers` should be extended once
            // because `finished_futures` contained only one freed `FutureId`.
            assert_eq!(inner.abort_handlers.len(), 2);
            // `FutureId` should be used from `finished_futures` container.
            assert!(inner.finished_futures.is_empty());
        }

        block_on(Timer::sleep(0.5));

        {
            let inner = abortable_system.inner.lock();
            assert_eq!(inner.abort_handlers.len(), 2);
            assert_eq!(inner.finished_futures.len(), 1);
        }

        block_on(Timer::sleep(0.4));

        {
            let inner = abortable_system.inner.lock();
            assert_eq!(inner.abort_handlers.len(), 2);
            assert_eq!(inner.finished_futures.len(), 2);
        }
    }

    #[test]
    fn test_critical_future_finished() {
        let settings = AbortSettings::default().critical_timout_s(1.);
        test_future_finished_impl(settings);
    }

    #[test]
    fn test_future_finished() {
        let settings = AbortSettings::default();
        test_future_finished_impl(settings);
    }

    #[test]
    fn test_spawn_critical() {
        static mut F1_FINISHED: bool = false;
        static mut F2_FINISHED: bool = false;

        let abortable_system = AbortableQueue::default();
        let spawner = abortable_system.weak_spawner();

        let settings = AbortSettings::default().critical_timout_s(0.4);

        let fut1 = async move {
            Timer::sleep(0.6).await;
            unsafe { F1_FINISHED = true };
        };
        spawner.spawn_with_settings(fut1, settings.clone());

        let fut2 = async move {
            Timer::sleep(0.2).await;
            unsafe { F2_FINISHED = true };
        };
        spawner.spawn_with_settings(fut2, settings.clone());

        abortable_system.abort_all();

        block_on(Timer::sleep(1.2));
        // `fut1` must not complete.
        assert!(unsafe { !F1_FINISHED });
        // `fut` must complete.
        assert!(unsafe { F2_FINISHED });
    }
}

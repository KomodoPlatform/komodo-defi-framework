use crate::executor::abortable_system::{AbortableSystem, InnerShared, SystemInner};
use crate::executor::{spawn_abortable, AbortOnDropHandle};
use futures::channel::oneshot;
use futures::future::Future as Future03;
use parking_lot::{Mutex as PaMutex, MutexGuard as PaMutexGuard};
use std::borrow::Borrow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

/// An alias.
pub trait FutureIdTrait: 'static + Eq + Hash + Send {}

impl<T: 'static + Eq + Hash + Send> FutureIdTrait for T {}

/// This is a simple `AbortableSystem` that ensures that the spawned futures will be aborted
/// once the `AbortableMap` instance is dropped.
///
/// `AbortableSet` is responsible for storing future handles in `SpawnedFuturesMap` *only*,
/// and *not* responsible for deleting them when they complete.
///
/// `AbortableSet` allows to spawn futures by specified `FutureId`.
#[derive(Default)]
pub struct AbortableSimpleMap<FutureId: FutureIdTrait> {
    inner: Arc<PaMutex<SimpleMapInner<FutureId>>>,
    subsystems: PaMutex<Vec<oneshot::Sender<()>>>,
}

impl<FutureId: FutureIdTrait> AbortableSimpleMap<FutureId> {
    /// Locks the inner `SimpleMapInner` that can be used to spawn/abort/check if contains future
    /// by its `FutureId` identifier.
    pub fn lock(&self) -> PaMutexGuard<'_, SimpleMapInner<FutureId>> { self.inner.lock() }
}

impl<FutureId: FutureIdTrait> AbortableSystem for AbortableSimpleMap<FutureId> {
    type Inner = SimpleMapInner<FutureId>;

    fn abort_all(&self) {
        self.inner.lock().abort_all();
        self.subsystems.lock().clear();
    }

    fn __push_subsystem_abort_tx(&self, subsystem_abort_tx: oneshot::Sender<()>) {
        self.subsystems.lock().push(subsystem_abort_tx);
    }
}

impl<FutureId: FutureIdTrait> From<InnerShared<SimpleMapInner<FutureId>>> for AbortableSimpleMap<FutureId> {
    fn from(inner: InnerShared<SimpleMapInner<FutureId>>) -> Self {
        AbortableSimpleMap {
            inner,
            subsystems: PaMutex::new(Vec::new()),
        }
    }
}

pub struct SimpleMapInner<FutureId: FutureIdTrait> {
    futures: HashMap<FutureId, AbortOnDropHandle>,
}

impl<FutureId: FutureIdTrait> Default for SimpleMapInner<FutureId> {
    fn default() -> Self {
        SimpleMapInner {
            futures: HashMap::default(),
        }
    }
}

impl<FutureId: FutureIdTrait> SystemInner for SimpleMapInner<FutureId> {
    fn abort_all(&mut self) { self.futures.clear(); }
}

impl<FutureId: FutureIdTrait> SimpleMapInner<FutureId> {
    /// Spawns the `fut` future by its `future_id`,
    /// or do nothing if there is a spawned future with the same `future_id` already.
    ///
    /// Returns whether the future has been spawned.
    pub fn spawn_or_ignore<F>(&mut self, future_id: FutureId, fut: F) -> bool
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        match self.futures.entry(future_id) {
            Entry::Occupied(_) => false,
            Entry::Vacant(entry) => {
                let abort_handle = spawn_abortable(fut);
                entry.insert(abort_handle);
                true
            },
        }
    }

    /// Whether a future with the given `future_id` has been spawned already.
    pub fn contains<Q>(&self, future_id: &Q) -> bool
    where
        FutureId: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.futures.contains_key(future_id)
    }

    /// Aborts a spawned future by the given `future_id` if it's still alive.
    pub fn abort_future<Q>(&mut self, future_id: &Q) -> bool
    where
        FutureId: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.futures.remove(future_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use crate::executor::Timer;

    #[test]
    fn test_abort_all() {
        static mut F1_FINISHED: bool = false;
        static mut F2_FINISHED: bool = false;

        let abortable_system = AbortableSimpleMap::default();
        let mut guard = abortable_system.lock();

        guard.spawn_or_ignore("F1".to_string(), async move {
            Timer::sleep(0.1).await;
            unsafe { F1_FINISHED = true };
        });
        assert!(guard.contains("F1"));
        assert!(!guard.contains("F2"));
        guard.spawn_or_ignore("F2".to_string(), async move {
            Timer::sleep(0.5).await;
            unsafe { F2_FINISHED = true };
        });

        drop(guard);
        block_on(Timer::sleep(0.3));
        abortable_system.abort_all();
        block_on(Timer::sleep(0.4));

        unsafe {
            assert!(F1_FINISHED);
            assert!(!F2_FINISHED);
        }
    }

    #[test]
    fn test_abort_future() {
        static mut F1_FINISHED: bool = false;

        let abortable_system = AbortableSimpleMap::default();
        let mut guard = abortable_system.lock();

        guard.spawn_or_ignore("F1".to_string(), async move {
            Timer::sleep(0.2).await;
            unsafe { F1_FINISHED = true };
        });

        drop(guard);
        block_on(Timer::sleep(0.05));

        let mut guard = abortable_system.lock();
        guard.abort_future("F1");
        assert!(!guard.contains("F1"));

        block_on(Timer::sleep(0.3));

        unsafe {
            assert!(!F1_FINISHED);
        }
    }

    #[test]
    fn test_spawn_twice() {
        static mut F1_FINISHED: bool = false;
        static mut F1_COPY_FINISHED: bool = false;

        let abortable_system = AbortableSimpleMap::default();
        let mut guard = abortable_system.lock();

        guard.spawn_or_ignore("F1".to_string(), async move {
            unsafe { F1_FINISHED = true };
        });
        guard.spawn_or_ignore("F1".to_string(), async move {
            unsafe { F1_COPY_FINISHED = true };
        });

        drop(guard);
        block_on(Timer::sleep(0.1));

        unsafe {
            assert!(F1_FINISHED);
            assert!(!F1_COPY_FINISHED);
        }
    }
}

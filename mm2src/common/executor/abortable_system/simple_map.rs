use std::borrow::Borrow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use futures::channel::oneshot;
use futures::future::Future as Future03;
use parking_lot::{Mutex as PaMutex, MutexGuard as PaMutexGuard};

use crate::executor::abortable_system::{AbortableSystem, AbortedError, InnerShared, SystemInner};
use crate::executor::{spawn_abortable, AbortOnDropHandle};

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
    inner: Arc<PaMutex<SimpleMapInnerState<FutureId>>>,
}

impl<FutureId: FutureIdTrait> AbortableSimpleMap<FutureId> {
    /// Locks the inner `SimpleMapInner` that can be used to spawn/abort/check if contains future
    /// by its `FutureId` identifier.
    pub fn lock(&self) -> PaMutexGuard<'_, SimpleMapInnerState<FutureId>> { self.inner.lock() }
}

impl<FutureId: FutureIdTrait> AbortableSystem for AbortableSimpleMap<FutureId> {
    type Inner = SimpleMapInnerState<FutureId>;

    fn abort_all(&self) -> Result<(), AbortedError> { self.inner.lock().abort_all() }

    fn __push_subsystem_abort_tx(&self, subsystem_abort_tx: oneshot::Sender<()>) -> Result<(), AbortedError> {
        self.inner.lock().insert_subsystem(subsystem_abort_tx)
    }
}

impl<FutureId: FutureIdTrait> From<InnerShared<SimpleMapInnerState<FutureId>>> for AbortableSimpleMap<FutureId> {
    fn from(inner: InnerShared<SimpleMapInnerState<FutureId>>) -> Self { AbortableSimpleMap { inner } }
}

pub enum SimpleMapInnerState<FutureId: FutureIdTrait> {
    Ready {
        futures: HashMap<FutureId, AbortOnDropHandle>,
        subsystems: Vec<oneshot::Sender<()>>,
    },
    Aborted,
}

impl<FutureId: FutureIdTrait> SimpleMapInnerState<FutureId> {
    fn futures_mut(&mut self) -> Result<&mut HashMap<FutureId, AbortOnDropHandle>, AbortedError> {
        match self {
            SimpleMapInnerState::Ready { futures, .. } => Ok(futures),
            SimpleMapInnerState::Aborted => Err(AbortedError),
        }
    }
}

impl<FutureId: FutureIdTrait> Default for SimpleMapInnerState<FutureId> {
    fn default() -> Self {
        SimpleMapInnerState::Ready {
            futures: HashMap::new(),
            subsystems: Vec::new(),
        }
    }
}

impl<FutureId: FutureIdTrait> SystemInner for SimpleMapInnerState<FutureId> {
    fn abort_all(&mut self) -> Result<(), AbortedError> {
        if matches!(self, SimpleMapInnerState::Aborted) {
            return Err(AbortedError);
        }

        *self = SimpleMapInnerState::Aborted;
        Ok(())
    }
}

impl<FutureId: FutureIdTrait> SimpleMapInnerState<FutureId> {
    /// Spawns the `fut` future by its `future_id`,
    /// or do nothing if there is a spawned future with the same `future_id` already.
    ///
    /// Returns whether the future has been spawned.
    pub fn spawn_or_ignore<F>(&mut self, future_id: FutureId, fut: F) -> Result<bool, AbortedError>
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        let futures = self.futures_mut()?;
        match futures.entry(future_id) {
            Entry::Occupied(_) => Ok(false),
            Entry::Vacant(entry) => {
                let abort_handle = spawn_abortable(fut);
                entry.insert(abort_handle);
                Ok(true)
            },
        }
    }

    /// Whether a future with the given `future_id` has been spawned already.
    pub fn contains<Q>(&self, future_id: &Q) -> Result<bool, AbortedError>
    where
        FutureId: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        match self {
            SimpleMapInnerState::Ready { futures, .. } => Ok(futures.contains_key(future_id)),
            SimpleMapInnerState::Aborted => Err(AbortedError),
        }
    }

    /// Aborts a spawned future by the given `future_id` if it's still alive.
    pub fn abort_future<Q>(&mut self, future_id: &Q) -> Result<bool, AbortedError>
    where
        FutureId: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        Ok(self.futures_mut()?.remove(future_id).is_some())
    }

    fn insert_subsystem(&mut self, subsystem_abort_tx: oneshot::Sender<()>) -> Result<(), AbortedError> {
        match self {
            SimpleMapInnerState::Ready { subsystems, .. } => {
                subsystems.push(subsystem_abort_tx);
                Ok(())
            },
            SimpleMapInnerState::Aborted => Err(AbortedError),
        }
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

        guard
            .spawn_or_ignore("F1".to_string(), async move {
                Timer::sleep(0.1).await;
                unsafe { F1_FINISHED = true };
            })
            .unwrap();
        assert_eq!(guard.contains("F1"), Ok(true));
        assert_eq!(guard.contains("F2"), Ok(false));
        guard
            .spawn_or_ignore("F2".to_string(), async move {
                Timer::sleep(0.5).await;
                unsafe { F2_FINISHED = true };
            })
            .unwrap();

        drop(guard);
        block_on(Timer::sleep(0.3));
        abortable_system.abort_all().unwrap();
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

        guard
            .spawn_or_ignore("F1".to_string(), async move {
                Timer::sleep(0.2).await;
                unsafe { F1_FINISHED = true };
            })
            .unwrap();

        drop(guard);
        block_on(Timer::sleep(0.05));

        let mut guard = abortable_system.lock();
        guard.abort_future("F1").unwrap();
        assert_eq!(guard.contains("F1"), Ok(false));

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

        let fut_1 = async move {
            unsafe { F1_FINISHED = true };
        };
        guard.spawn_or_ignore("F1".to_string(), fut_1).unwrap();

        let fut_2 = async move {
            unsafe { F1_COPY_FINISHED = true };
        };
        guard.spawn_or_ignore("F1".to_string(), fut_2).unwrap();

        drop(guard);
        block_on(Timer::sleep(0.1));

        unsafe {
            assert!(F1_FINISHED);
            assert!(!F1_COPY_FINISHED);
        }
    }
}

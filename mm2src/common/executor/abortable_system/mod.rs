use crate::executor::spawn;
use futures::channel::oneshot;
use parking_lot::Mutex as PaMutex;
use std::sync::{Arc, Weak};

pub mod abortable_queue;
pub mod graceful_shutdown;
pub mod simple_map;

pub type InnerShared<Inner> = Arc<PaMutex<Inner>>;
pub type InnerWeak<Inner> = Weak<PaMutex<Inner>>;

pub trait AbortableSystem: From<InnerShared<Self::Inner>> {
    type Inner: SystemInner;

    /// Aborts all spawned futures and subsystems if they present.
    fn abort_all(&self);

    /// Creates a new subsystem `S` linked to `Self` the way that
    /// if `Self` is aborted, the futures spawned by the subsystem will be aborted as well.
    /// For more info, look at the [`tests::test_abort_subsystem`].
    ///
    ///
    /// But in the same time the subsystem can be aborted independently from `Self` system.
    /// For more info, look at the [`tests::test_abort_supersystem`].
    fn create_subsystem<S>(&self) -> S
    where
        S: AbortableSystem,
    {
        let (abort_tx, abort_rx) = oneshot::channel();
        self.__push_subsystem_abort_tx(abort_tx);

        let inner_shared = Arc::new(PaMutex::new(S::Inner::default()));
        let inner_weak = Arc::downgrade(&inner_shared);

        let abort_fut = async move {
            // Once the `abort_rx` is invoked, we need to abort its all futures.
            abort_rx.await.ok();

            if let Some(inner_arc) = inner_weak.upgrade() {
                inner_arc.lock().abort_all();
            }
        };

        spawn(abort_fut);
        S::from(inner_shared)
    }

    fn __push_subsystem_abort_tx(&self, subsystem_abort_tx: oneshot::Sender<()>);
}

pub trait SystemInner: Default + Send + 'static {
    /// Aborts all spawned futures and subsystems if they present.
    fn abort_all(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use crate::executor::{SpawnFuture, Timer};
    use abortable_queue::AbortableQueue;

    #[test]
    fn test_abort_subsystem() {
        static mut SUPER_FINISHED: bool = false;
        static mut SUB_FINISHED: bool = false;

        let super_system = AbortableQueue::default();
        super_system.weak_spawner().spawn(async move {
            Timer::sleep(0.5).await;
            unsafe { SUPER_FINISHED = true };
        });

        let sub_system: AbortableQueue = super_system.create_subsystem();
        sub_system.weak_spawner().spawn(async move {
            Timer::sleep(0.5).await;
            unsafe { SUB_FINISHED = true };
        });

        block_on(Timer::sleep(0.1));
        drop(sub_system);
        block_on(Timer::sleep(0.8));

        // Only the super system should finish as the sub system has been aborted.
        unsafe {
            assert!(SUPER_FINISHED);
            assert!(!SUB_FINISHED);
        }
    }

    #[test]
    fn test_abort_supersystem() {
        static mut SUPER_FINISHED: bool = false;
        static mut SUB_FINISHED: bool = false;

        let super_system = AbortableQueue::default();
        super_system.weak_spawner().spawn(async move {
            Timer::sleep(0.5).await;
            unsafe { SUPER_FINISHED = true };
        });

        let sub_system: AbortableQueue = super_system.create_subsystem();
        sub_system.weak_spawner().spawn(async move {
            Timer::sleep(0.5).await;
            unsafe { SUB_FINISHED = true };
        });

        block_on(Timer::sleep(0.1));
        drop(super_system);
        block_on(Timer::sleep(0.8));

        // Nothing should finish as the super system has been aborted.
        unsafe {
            assert!(!SUPER_FINISHED);
            assert!(!SUB_FINISHED);
        }
    }
}

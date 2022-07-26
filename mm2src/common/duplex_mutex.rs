use super::executor::Timer;
use super::now_ms;
use parking_lot_core::SpinWait;
use std::cell::UnsafeCell;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[must_use = "if unused the DuplexMutexGuard will immediately unlock"]
pub struct DuplexMutexGuard<'a, T> {
    mutex: &'a DuplexMutex<T>,
}

unsafe impl<T: Send> Send for DuplexMutexGuard<'_, T> {}
unsafe impl<T: Sync> Sync for DuplexMutexGuard<'_, T> {}

impl<T> Deref for DuplexMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { &*self.mutex.pimpl.data.get() } }
}

impl<T> DerefMut for DuplexMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T { unsafe { &mut *self.mutex.pimpl.data.get() } }
}

impl<T> Drop for DuplexMutexGuard<'_, T> {
    fn drop(&mut self) { self.mutex.pimpl.unlock().unwrap(); }
}

impl<T: fmt::Debug> fmt::Debug for DuplexMutexGuard<'_, T> {
    fn fmt(&self, ft: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Debug::fmt(&**self, ft) }
}

impl<T: fmt::Display> fmt::Display for DuplexMutexGuard<'_, T> {
    fn fmt(&self, ft: &mut fmt::Formatter<'_>) -> fmt::Result { (**self).fmt(ft) }
}

pub struct Impl<T> {
    locked: AtomicUsize,
    data: UnsafeCell<T>,
}

// We're only using `Impl::data` behind an `Arc` and a lock.
unsafe impl<T> Send for Impl<T> where T: Send {}
unsafe impl<T> Sync for Impl<T> {}

impl<T> Impl<T> {
    fn spinlock(&self, timeout_ms: i64) -> Result<(), String> {
        if self
            .locked
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return Ok(());
        }
        let start = now_ms() as i64;
        let mut spin_wait = SpinWait::new();
        loop {
            let fast = spin_wait.spin();
            if self
                .locked
                .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(());
            }
            if !fast {
                spin_wait.reset();
                let delta = now_ms() as i64 - start;
                if delta > timeout_ms {
                    return ERR!("spinlock timeout, {}ms", delta);
                }
            }
        }
    }

    fn unlock(&self) -> Result<(), String> {
        if self
            .locked
            .compare_exchange(1, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return Ok(());
        }
        ERR!("Not locked")
    }

    async fn sleeplock(&self, timeout_ms: i64) -> Result<(), String> {
        if self
            .locked
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return Ok(());
        }
        let start = now_ms() as i64;
        loop {
            Timer::sleep(0.02).await;
            if self
                .locked
                .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(());
            }
            let delta = now_ms() as i64 - start;
            if delta > timeout_ms {
                return ERR!("sleeplock timeout, {}ms", delta);
            }
        }
    }
}

/// A mutual exclusion primitive that can be used from both the synchronous and asynchronous contexts.  
///
/// There is a problem with existing primitives:
/// Synchronous Mutex can not be used reliably from WASM since threading there is new.
/// Asynchronous Mutex is only compatible with fully asynchronous code
/// because `block_on` will panic under a layered async->sync->block_on(lock),
/// but making everything asynchronous would lead to bloated machine code,
/// obscure errors, restraints and slowed compilation speed.
///
/// DuplexMutex bridges this gap by being useable in both contexts:  
/// In the synchronous context it will spin.  
/// In the asynchronous context it will wait with the `Timer`,
/// allowing the green thread holding the lock to resurface even in situations
/// when both the holder and the entrant are on the same system thread.
pub struct DuplexMutex<T> {
    pimpl: Arc<Impl<T>>,
}

unsafe impl<T> Send for DuplexMutex<T> where Arc<Impl<T>>: Send {}

impl<T> Clone for DuplexMutex<T> {
    fn clone(&self) -> DuplexMutex<T> {
        DuplexMutex {
            pimpl: self.pimpl.clone(),
        }
    }
}

impl<T> DuplexMutex<T> {
    pub fn new(v: T) -> DuplexMutex<T> {
        DuplexMutex {
            pimpl: Arc::new(Impl {
                locked: AtomicUsize::new(0),
                data: UnsafeCell::new(v),
            }),
        }
    }
}

impl<T> DuplexMutex<T> {
    /// Synchronous spinlock.  
    /// Should be used when the mutex contention happens from different system threads
    /// (like when the mutex guard does not cross green thread boundaries).  
    /// Using spinlock from two green threads running on the same system thread might result in a deadlock,
    /// but that might be mitigated by handling the timeout `Err`.
    pub fn spinlock(&self, timeout_ms: i64) -> Result<DuplexMutexGuard<'_, T>, String> {
        try_s!(self.pimpl.spinlock(timeout_ms));
        Ok(DuplexMutexGuard { mutex: self })
    }

    /// Asynchronous `Timer::sleep` lock.  
    /// Can be used with long-held locks and with locks held across green thread boundaries.
    pub async fn sleeplock(&self, timeout_ms: i64) -> Result<DuplexMutexGuard<'_, T>, String> {
        try_s!(self.pimpl.sleeplock(timeout_ms).await);
        Ok(DuplexMutexGuard { mutex: self })
    }
}

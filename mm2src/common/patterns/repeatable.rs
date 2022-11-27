//! TODO move to `common/custom_futures/repeatable.rs`.

use crate::executor::Timer;
use futures::FutureExt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

#[macro_export]
macro_rules! repeatable {
    // Please note that we shouldn't allow the user to declare the future as `async move`.
    // Because moving local variables may lead to incorrect usage.
    (async { $($t:tt)* }) => {
        $crate::repeatable::Repeatable::new(|| Box::pin(async { $($t)* }))
    };
    ($fut:expr) => {
        $crate::repeatable::Repeatable::new(|| $fut)
    };
}

#[macro_export]
macro_rules! ready {
    ($res:expr) => {{
        return $crate::repeatable::Action::Ready($res);
    }};
}

#[macro_export]
macro_rules! retry {
    () => {{
        return $crate::repeatable::Action::Retry;
    }};
}

type RepeatResult<T> = Result<T, AttemptsExceed>;

#[derive(Clone, Debug, PartialEq)]
pub struct AttemptsExceed {
    pub attempts: usize,
}

pub trait FactoryTrait<F>: Fn() -> F {}

impl<Factory, F> FactoryTrait<F> for Factory where Factory: Fn() -> F {}

pub trait RepeatableTrait<T>: Future<Output = Action<T>> + Unpin {}

impl<F, T> RepeatableTrait<T> for F where F: Future<Output = Action<T>> + Unpin {}

#[derive(Debug)]
pub enum Action<T> {
    Ready(T),
    Retry,
}

pub struct Repeatable<Factory, F, T> {
    factory: Factory,
    _phantom: PhantomData<(F, T)>,
}

impl<Factory, F, T> Repeatable<Factory, F, T> {
    pub fn new(factory: Factory) -> Self {
        Repeatable {
            factory,
            _phantom: PhantomData::default(),
        }
    }

    pub fn attempts(self, total_attempts: usize) -> RepeatAttempts<Factory, F, T> {
        // TODO avoid asserting probably.
        assert!(total_attempts > 0);

        RepeatAttempts {
            factory: self.factory,
            total_attempts,
            _phantom: PhantomData::default(),
        }
    }
}

/// The result of [`Repeatable::attempts`] - the next step at the future configuration.
pub struct RepeatAttempts<Factory, F, T> {
    factory: Factory,
    total_attempts: usize,
    _phantom: PhantomData<(F, T)>,
}

impl<Factory, F, T> RepeatAttempts<Factory, F, T>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T>,
{
    pub fn repeat_every(self, timeout_s: f64) -> RepeatAttemptsEvery<Factory, F, T> {
        let exec = (self.factory)();
        RepeatAttemptsEvery {
            factory: self.factory,
            exec_fut: Some(exec),
            timeout_fut: None,
            attempt: 0,
            total_attempts: self.total_attempts,
            repeat_every: timeout_s,
            _phantom: PhantomData::default(),
        }
    }
}

/// The result of [`Repeatable::repeat_every`] - the next step at the future configuration.
pub struct RepeatAttemptsEvery<Factory, F, T> {
    factory: Factory,
    /// Currently executable future. Aka an active attempt.
    exec_fut: Option<F>,
    /// A timeout future if we're currently waiting for a timeout.
    timeout_fut: Option<Timer>,
    attempt: usize,
    total_attempts: usize,
    repeat_every: f64,
    _phantom: PhantomData<(F, T)>,
}

impl<Factory, F: Unpin, T> Unpin for RepeatAttemptsEvery<Factory, F, T> {}

impl<Factory, F, T> Future for RepeatAttemptsEvery<Factory, F, T>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T>,
{
    type Output = RepeatResult<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            // TODO figure out why ? doesn't work on `Self::poll_timeout()?`.
            if Self::poll_timeout(&mut self, cx).is_pending() {
                return Poll::Pending;
            }

            let mut exec = self.exec_fut.take().unwrap();
            match exec.poll_unpin(cx) {
                Poll::Ready(Action::Ready(ready)) => return Poll::Ready(Ok(ready)),
                Poll::Ready(Action::Retry) => {
                    self.attempt += 1;
                    if self.attempt >= self.total_attempts {
                        return Poll::Ready(Err(AttemptsExceed { attempts: self.attempt }));
                    }
                    // Create a new future attempt.
                    self.exec_fut = Some((self.factory)());
                    self.timeout_fut = Some(Timer::sleep(self.repeat_every));
                    // We need to poll the timer at the next loop iteration to let the executor know about it.
                    continue;
                },
                // We should proceed with this `exec` future attempt.
                Poll::Pending => {
                    self.exec_fut = Some(exec);
                    return Poll::Pending;
                },
            }
        }
    }
}

impl<Factory, F, T> RepeatAttemptsEvery<Factory, F, T>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T>,
{
    /// Returns `Poll::Ready(())` if there is no need to wait for the timeout.
    fn poll_timeout(self: &mut Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut timeout = match self.timeout_fut.take() {
            Some(timeout) => timeout,
            None => return Poll::Ready(()),
        };

        match timeout.poll_unpin(cx) {
            Poll::Ready(_) => Poll::Ready(()),
            Poll::Pending => {
                self.timeout_fut = Some(timeout);
                Poll::Pending
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use futures::lock::Mutex as AsyncMutex;

    #[test]
    fn test_repeat_attempts_every() {
        let counter = AsyncMutex::new(0);

        let fut = repeatable!(async {
            let mut counter = counter.lock().await;
            *counter += 1;
            if *counter == 3 {
                ready!(*counter);
            } else {
                retry!();
            }
        })
        .attempts(3)
        .repeat_every(0.1);

        let actual = block_on(fut);
        // If the counter is 3, then there were exactly 3 attempts to finish the future.
        assert_eq!(actual, Ok(3));
    }

    #[test]
    fn test_repeat_attempts_every_exceed() {
        let counter = AsyncMutex::new(0);

        let fut = repeatable!(async {
            let mut counter = counter.lock().await;
            *counter += 1;
            if *counter == 3 {
                ready!(*counter);
            } else {
                retry!();
            }
        })
        .attempts(2)
        .repeat_every(0.1);

        let actual = block_on(fut);
        assert_eq!(actual, Err(AttemptsExceed { attempts: 2 }));

        // If the counter is 2, then there were exactly 2 attempts to finish the future.
        let actual_attempts = block_on(counter.lock());
        assert_eq!(*actual_attempts, 2);
    }
}

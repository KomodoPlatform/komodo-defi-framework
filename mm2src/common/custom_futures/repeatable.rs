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
        $crate::custom_futures::repeatable::Repeatable::new(|| Box::pin(async { $($t)* }))
    };
    ($fut:expr) => {
        $crate::custom_futures::repeatable::Repeatable::new(|| $fut)
    };
}

#[macro_export]
macro_rules! ready {
    ($res:expr) => {{
        return $crate::custom_futures::repeatable::Action::Ready($res);
    }};
}

#[macro_export]
macro_rules! retry {
    () => {{
        return $crate::custom_futures::repeatable::Action::Retry(());
    }};
    ($err:expr) => {{
        return $crate::custom_futures::repeatable::Action::Retry($err);
    }};
}

type RepeatResult<T, E> = Result<T, AttemptsExceed<E>>;

#[derive(Clone, Debug, PartialEq)]
pub struct AttemptsExceed<E> {
    pub attempts: usize,
    /// An error occurred during the last attempt.
    pub error: E,
}

pub trait FactoryTrait<F>: Fn() -> F {}

impl<Factory, F> FactoryTrait<F> for Factory where Factory: Fn() -> F {}

pub trait RepeatableTrait<T, E>: Future<Output = Action<T, E>> + Unpin {}

impl<F, T, E> RepeatableTrait<T, E> for F where F: Future<Output = Action<T, E>> + Unpin {}

pub(crate) trait InspectErrorTrait<E>: 'static + Fn(&E) {}

impl<F: 'static + Fn(&E), E> InspectErrorTrait<E> for F {}

#[derive(Debug)]
pub enum Action<T, E> {
    Ready(T),
    Retry(E),
}

pub trait RetryOnError<T, E> {
    fn retry_on_err(self) -> Action<T, E>;
}

impl<T, E> RetryOnError<T, E> for Result<T, E> {
    fn retry_on_err(self) -> Action<T, E> {
        match self {
            Ok(ready) => Action::Ready(ready),
            Err(e) => Action::Retry(e),
        }
    }
}

pub struct Repeatable<Factory, F, T, E> {
    factory: Factory,
    inspect_err: Option<Box<dyn InspectErrorTrait<E>>>,
    _phantom: PhantomData<(F, T, E)>,
}

impl<Factory, F, T, E> Repeatable<Factory, F, T, E> {
    pub fn new(factory: Factory) -> Self {
        Repeatable {
            factory,
            inspect_err: None,
            _phantom: PhantomData::default(),
        }
    }

    /// Specifies an inspect handler that does something with an error on each unsuccessful attempt.
    pub fn inspect_err<Inspect>(mut self, inspect: Inspect) -> Self
    where
        Inspect: 'static + Fn(&E),
    {
        self.inspect_err = Some(Box::new(inspect));
        self
    }

    pub fn attempts(self, total_attempts: usize) -> RepeatAttempts<Factory, F, T, E> {
        // TODO avoid asserting probably.
        assert!(total_attempts > 0);

        RepeatAttempts {
            factory: self.factory,
            total_attempts,
            inspect_err: self.inspect_err,
            _phantom: PhantomData::default(),
        }
    }
}

/// The result of [`Repeatable::attempts`] - the next step at the future configuration.
pub struct RepeatAttempts<Factory, F, T, E> {
    factory: Factory,
    total_attempts: usize,
    inspect_err: Option<Box<dyn InspectErrorTrait<E>>>,
    _phantom: PhantomData<(F, T, E)>,
}

impl<Factory, F, T, E> RepeatAttempts<Factory, F, T, E>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T, E>,
{
    /// Specifies an inspect handler that does something with an error on each unsuccessful attempt.
    pub fn inspect_err<Inspect>(mut self, inspect: Inspect) -> Self
    where
        Inspect: 'static + Fn(&E),
    {
        self.inspect_err = Some(Box::new(inspect));
        self
    }

    pub fn repeat_every(self, timeout_s: f64) -> RepeatAttemptsEvery<Factory, F, T, E> {
        let exec = (self.factory)();
        RepeatAttemptsEvery {
            factory: self.factory,
            exec_fut: Some(exec),
            timeout_fut: None,
            attempt: 0,
            total_attempts: self.total_attempts,
            repeat_every: timeout_s,
            inspect_err: self.inspect_err,
            _phantom: PhantomData::default(),
        }
    }
}

/// The result of [`Repeatable::repeat_every`] - the next step at the future configuration.
pub struct RepeatAttemptsEvery<Factory, F, T, E> {
    factory: Factory,
    /// Currently executable future. Aka an active attempt.
    exec_fut: Option<F>,
    /// A timeout future if we're currently waiting for a timeout.
    timeout_fut: Option<Timer>,
    attempt: usize,
    total_attempts: usize,
    repeat_every: f64,
    inspect_err: Option<Box<dyn InspectErrorTrait<E>>>,
    _phantom: PhantomData<(F, T, E)>,
}

impl<Factory, F: Unpin, T, E> Unpin for RepeatAttemptsEvery<Factory, F, T, E> {}

impl<Factory, F, T, E> Future for RepeatAttemptsEvery<Factory, F, T, E>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T, E>,
{
    type Output = RepeatResult<T, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            // TODO figure out why ? doesn't work on `Self::poll_timeout()?`.
            if Self::poll_timeout(&mut self, cx).is_pending() {
                return Poll::Pending;
            }

            let mut exec = self.exec_fut.take().unwrap();
            match exec.poll_unpin(cx) {
                Poll::Ready(Action::Ready(ready)) => return Poll::Ready(Ok(ready)),
                Poll::Ready(Action::Retry(error)) => {
                    if let Some(ref inspect) = self.inspect_err {
                        inspect(&error);
                    }

                    self.attempt += 1;
                    if self.attempt >= self.total_attempts {
                        return Poll::Ready(Err(AttemptsExceed {
                            attempts: self.attempt,
                            error,
                        }));
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

impl<Factory, F, T, E> RepeatAttemptsEvery<Factory, F, T, E> {
    /// Specifies an inspect handler that does something with an error on each unsuccessful attempt.
    pub fn inspect_err<Inspect>(mut self, inspect: Inspect) -> Self
    where
        Inspect: 'static + Fn(&E),
    {
        self.inspect_err = Some(Box::new(inspect));
        self
    }
}

impl<Factory, F, T, E> RepeatAttemptsEvery<Factory, F, T, E>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T, E>,
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
        assert_eq!(actual, Err(AttemptsExceed { attempts: 2, error: () }));

        // If the counter is 2, then there were exactly 2 attempts to finish the future.
        let actual_attempts = block_on(counter.lock());
        assert_eq!(*actual_attempts, 2);
    }

    #[test]
    fn test_retry_on_err() {
        let counter = AsyncMutex::new(0);

        async fn an_operation(counter: &AsyncMutex<i32>) -> Result<i32, &str> {
            let mut counter = counter.lock().await;
            *counter += 1;
            if *counter == 3 {
                Ok(*counter)
            } else {
                Err("Not ready")
            }
        }

        let fut = repeatable!(async { an_operation(&counter).await.retry_on_err() })
            .attempts(3)
            .repeat_every(0.1);

        let actual = block_on(fut);
        assert_eq!(actual, Ok(3));
    }

    #[test]
    fn test_inspect_err() {
        let inspect_counter = Arc::new(AtomicUsize::new(0));
        let inspect_counter_c = inspect_counter.clone();
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
        .inspect_err(move |_| {
            inspect_counter.fetch_add(1, Ordering::Relaxed);
        })
        .repeat_every(0.1);

        let actual = block_on(fut);
        // If the counter is 3, then there were exactly 3 attempts to finish the future.
        assert_eq!(actual, Ok(3));
        // There should be 2 errors.
        assert_eq!(inspect_counter_c.load(Ordering::Relaxed), 2);
    }
}

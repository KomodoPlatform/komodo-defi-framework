use crate::custom_futures::repeatable::{poll_timeout, Action, FactoryTrait, InspectErrorTrait, RepeatableTrait};
use crate::executor::Timer;
use futures::FutureExt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Clone, Debug, PartialEq)]
pub struct AttemptsExceed<E> {
    pub attempts: usize,
    /// An error occurred during the last attempt.
    pub error: E,
}

/// The result of [`Repeatable::attempts`] - the next step at the future configuration.
pub struct RepeatAttempts<Factory, F, T, E> {
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

impl<Factory, F, T, E> RepeatAttempts<Factory, F, T, E>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T, E>,
{
    pub(super) fn new(
        factory: Factory,
        repeat_every: f64,
        inspect_err: Option<Box<dyn InspectErrorTrait<E>>>,
        total_attempts: usize,
    ) -> Self {
        let exec = factory();

        RepeatAttempts {
            factory,
            exec_fut: Some(exec),
            timeout_fut: None,
            attempt: 0,
            total_attempts,
            repeat_every,
            inspect_err,
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
}

impl<Factory, F: Unpin, T, E> Unpin for RepeatAttempts<Factory, F, T, E> {}

impl<Factory, F, T, E> Future for RepeatAttempts<Factory, F, T, E>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T, E>,
{
    type Output = Result<T, AttemptsExceed<E>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            if poll_timeout(&mut self.timeout_fut, cx).is_pending() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_futures::repeatable::RetryOnError;
    use crate::{block_on, repeatable};
    use futures::lock::Mutex as AsyncMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    async fn an_operation(counter: &AsyncMutex<i32>) -> Result<i32, &str> {
        let mut counter = counter.lock().await;
        *counter += 1;
        if *counter == 3 {
            Ok(*counter)
        } else {
            Err("Not ready")
        }
    }

    #[test]
    fn test_attempts_success() {
        let counter = AsyncMutex::new(0);

        let fut = repeatable!(async { an_operation(&counter).await.retry_on_err() })
            .repeat_every(Duration::from_millis(100))
            .attempts(3);

        let actual = block_on(fut);
        // If the counter is 3, then there were exactly 3 attempts to finish the future.
        assert_eq!(actual, Ok(3));
    }

    #[test]
    fn test_attempts_exceed() {
        let counter = AsyncMutex::new(0);

        let fut = repeatable!(async { an_operation(&counter).await.retry_on_err() })
            .repeat_every(Duration::from_millis(100))
            .attempts(2);

        let actual = block_on(fut);
        assert_eq!(
            actual,
            Err(AttemptsExceed {
                attempts: 2,
                error: "Not ready"
            })
        );

        // If the counter is 2, then there were exactly 2 attempts to finish the future.
        let actual_attempts = block_on(counter.lock());
        assert_eq!(*actual_attempts, 2);
    }

    #[test]
    fn test_attempts_retry_on_err() {
        let counter = AsyncMutex::new(0);

        let fut = repeatable!(async { an_operation(&counter).await.retry_on_err() })
            .repeat_every(Duration::from_millis(100))
            .attempts(3);

        let actual = block_on(fut);
        assert_eq!(actual, Ok(3));
    }

    #[test]
    fn test_attempts_inspect_err() {
        let inspect_counter = Arc::new(AtomicUsize::new(0));
        let inspect_counter_c = inspect_counter.clone();
        let counter = AsyncMutex::new(0);

        let fut = repeatable!(async { an_operation(&counter).await.retry_on_err() })
            .repeat_every(Duration::from_millis(100))
            .inspect_err(move |_| {
                inspect_counter.fetch_add(1, Ordering::Relaxed);
            })
            .attempts(3);

        let actual = block_on(fut);
        // If the counter is 3, then there were exactly 3 attempts to finish the future.
        assert_eq!(actual, Ok(3));
        // There should be 2 errors.
        assert_eq!(inspect_counter_c.load(Ordering::Relaxed), 2);
    }
}

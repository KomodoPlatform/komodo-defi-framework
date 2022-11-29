use crate::custom_futures::repeatable::{poll_timeout, Action, FactoryTrait, InspectErrorTrait, RepeatableTrait};
use crate::executor::Timer;
use futures::FutureExt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use wasm_timer::Instant;

#[derive(Clone, Debug, PartialEq)]
pub struct TimeoutExpired<E> {
    pub until: Instant,
    /// An error occurred during the last attempt.
    pub error: E,
}

/// The result of [`Repeatable::attempts`] - the next step at the future configuration.
pub struct RepeatUntil<Factory, F, T, E> {
    factory: Factory,
    /// Currently executable future. Aka an active attempt.
    exec_fut: Option<F>,
    /// A timeout future if we're currently waiting for a timeout.
    timeout_fut: Option<Timer>,
    until: Instant,
    repeat_every: Duration,
    inspect_err: Option<Box<dyn InspectErrorTrait<E>>>,
    _phantom: PhantomData<(F, T, E)>,
}

impl<Factory, F, T, E> RepeatUntil<Factory, F, T, E>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T, E>,
{
    pub(super) fn new(
        factory: Factory,
        until: Instant,
        repeat_every: Duration,
        inspect_err: Option<Box<dyn InspectErrorTrait<E>>>,
    ) -> Self {
        let exec = factory();

        RepeatUntil {
            factory,
            exec_fut: Some(exec),
            timeout_fut: None,
            until,
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

impl<Factory, F: Unpin, T, E> Unpin for RepeatUntil<Factory, F, T, E> {}

impl<Factory, F, T, E> Future for RepeatUntil<Factory, F, T, E>
where
    Factory: FactoryTrait<F>,
    F: RepeatableTrait<T, E>,
{
    type Output = Result<T, TimeoutExpired<E>>;

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

                    let will_be_after_timeout = Instant::now() + self.repeat_every;
                    if will_be_after_timeout > self.until {
                        return Poll::Ready(Err(TimeoutExpired {
                            until: self.until,
                            error,
                        }));
                    }

                    // Create a new future attempt.
                    self.exec_fut = Some((self.factory)());
                    self.timeout_fut = Some(Timer::sleep(self.repeat_every.as_secs_f64()));
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
    use std::time::Duration;

    async fn an_operation(counter: &AsyncMutex<usize>, finish_if: usize) -> Result<usize, &str> {
        let mut counter = counter.lock().await;
        *counter += 1;
        if *counter == finish_if {
            Ok(*counter)
        } else {
            Err("Not ready")
        }
    }

    #[test]
    fn test_until_success() {
        const ATTEMPTS_TO_FINISH: usize = 5;
        const LOWEST_TIMEOUT: Duration = Duration::from_millis(350);
        const HIGHEST_TIMEOUT: Duration = Duration::from_millis(700);

        let counter = AsyncMutex::new(0);

        let fut = repeatable!(async { an_operation(&counter, ATTEMPTS_TO_FINISH).await.retry_on_err() })
            .repeat_every(Duration::from_millis(100))
            .until(Instant::now() + HIGHEST_TIMEOUT);

        let before = Instant::now();
        let actual = block_on(fut);
        let took = before.elapsed();

        // If the counter is 3, then there were exactly 3 attempts to finish the future.
        assert_eq!(actual, Ok(ATTEMPTS_TO_FINISH));

        assert!(
            LOWEST_TIMEOUT <= took && took <= HIGHEST_TIMEOUT,
            "Expected [{:?}, {:?}], but took {:?}",
            LOWEST_TIMEOUT,
            HIGHEST_TIMEOUT,
            took
        );
    }

    #[test]
    fn test_until_expired() {
        const ATTEMPTS_TO_FINISH: usize = 10;
        const LOWEST_TIMEOUT: Duration = Duration::from_millis(350);
        const HIGHEST_TIMEOUT: Duration = Duration::from_millis(700);

        let counter = AsyncMutex::new(0);

        let until = Instant::now() + HIGHEST_TIMEOUT;

        let fut = repeatable!(async { an_operation(&counter, ATTEMPTS_TO_FINISH).await.retry_on_err() })
            .repeat_every(Duration::from_millis(100))
            .until(until.clone());

        let before = Instant::now();
        let actual = block_on(fut);
        let took = before.elapsed();

        // If the counter is 3, then there were exactly 3 attempts to finish the future.
        let error = TimeoutExpired {
            until,
            error: "Not ready",
        };
        assert_eq!(actual, Err(error));

        assert!(
            LOWEST_TIMEOUT <= took && took <= HIGHEST_TIMEOUT,
            "Expected [{:?}, {:?}], but took {:?}",
            LOWEST_TIMEOUT,
            HIGHEST_TIMEOUT,
            took
        );
    }
}

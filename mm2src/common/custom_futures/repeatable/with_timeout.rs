use crate::custom_futures::repeatable::{poll_timeout, Action, FactoryTrait, InspectErrorTrait, RepeatableTrait};
use crate::executor::Timer;
use crate::now_ms;
use crate::number_type_casting::SafeTypeCastingNumbers;
use futures::FutureExt;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use wasm_timer::Instant;

#[derive(Clone, Debug, PartialEq)]
pub struct TimeoutExpired<E> {
    pub until: Until,
    /// An error occurred during the last attempt.
    pub error: E,
}

impl<E: fmt::Display> fmt::Display for TimeoutExpired<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Waited too long until {:?} for the future to succeed. Error: {}",
            self.until, self.error
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Until {
    Instant(Instant),
    TimestampMs(u64),
}

/// The next step at the future configuration `Repeatable` -> `RepeatEvery` -> `RepeatUntil`.
pub struct RepeatUntil<Factory, F, T, E> {
    factory: Factory,
    /// Currently executable future. Aka an active attempt.
    exec_fut: Option<F>,
    /// A timeout future if we're currently waiting for a timeout.
    timeout_fut: Option<Timer>,
    until: Until,
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
        until: Until,
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
        Inspect: 'static + Fn(&E) + Send,
    {
        self.inspect_err = Some(Box::new(inspect));
        self
    }

    /// Checks if the deadline is not going to be reached after the `repeat_every` timeout.
    fn check_can_retry_after_timeout(&self) -> bool {
        match self.until {
            Until::Instant(instant) => {
                let will_be_after_timeout = Instant::now() + self.repeat_every;
                will_be_after_timeout < instant
            },
            Until::TimestampMs(timestamp_ms) => {
                let timeout: u64 = self.repeat_every.as_millis().into_or_max();
                now_ms() + timeout < timestamp_ms
            },
        }
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

                    if !self.check_can_retry_after_timeout() {
                        return Poll::Ready(Err(TimeoutExpired {
                            until: self.until.clone(),
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
            until: Until::Instant(until),
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

    #[test]
    fn test_until_ms() {
        const ATTEMPTS_TO_FINISH: usize = 5;
        const LOWEST_TIMEOUT: u64 = 350;
        const HIGHEST_TIMEOUT: u64 = 700;

        let counter = AsyncMutex::new(0);

        let fut = repeatable!(async { an_operation(&counter, ATTEMPTS_TO_FINISH).await.retry_on_err() })
            .repeat_every(Duration::from_millis(100))
            .until_ms(now_ms() + HIGHEST_TIMEOUT);

        let before = Instant::now();
        let actual = block_on(fut);
        let took = before.elapsed();

        // If the counter is 3, then there were exactly 3 attempts to finish the future.
        assert_eq!(actual, Ok(ATTEMPTS_TO_FINISH));

        let lowest = Duration::from_millis(LOWEST_TIMEOUT);
        let highest = Duration::from_millis(HIGHEST_TIMEOUT);
        assert!(
            lowest <= took && took <= highest,
            "Expected [{:?}, {:?}], but took {:?}",
            lowest,
            highest,
            took
        );
    }
}

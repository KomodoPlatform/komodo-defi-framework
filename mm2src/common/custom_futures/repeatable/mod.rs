use crate::executor::Timer;
use futures::FutureExt;
use std::future::Future;
use std::marker::PhantomData;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

mod with_attempts;
mod with_timeout;

pub use with_attempts::{AttemptsExceed, RepeatAttempts};
pub use with_timeout::{RepeatUntil, TimeoutExpired};

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

    pub fn repeat_every(self, repeat_every: Duration) -> RepeatEvery<Factory, F, T, E> {
        RepeatEvery {
            factory: self.factory,
            repeat_every,
            inspect_err: self.inspect_err,
            _phantom: PhantomData::default(),
        }
    }
}

pub struct RepeatEvery<Factory, F, T, E> {
    factory: Factory,
    repeat_every: Duration,
    inspect_err: Option<Box<dyn InspectErrorTrait<E>>>,
    _phantom: PhantomData<(F, T, E)>,
}

impl<Factory, F, T, E> RepeatEvery<Factory, F, T, E> {
    /// Specifies an inspect handler that does something with an error on each unsuccessful attempt.
    pub fn inspect_err<Inspect>(mut self, inspect: Inspect) -> Self
    where
        Inspect: 'static + Fn(&E),
    {
        self.inspect_err = Some(Box::new(inspect));
        self
    }

    pub fn attempts(self, total_attempts: usize) -> RepeatAttempts<Factory, F, T, E>
    where
        Factory: FactoryTrait<F>,
        F: RepeatableTrait<T, E>,
    {
        RepeatAttempts::new(
            self.factory,
            self.repeat_every.as_secs_f64(),
            self.inspect_err,
            total_attempts,
        )
    }

    pub fn until(self, until: Instant) -> RepeatUntil<Factory, F, T, E>
    where
        Factory: FactoryTrait<F>,
        F: RepeatableTrait<T, E>,
    {
        RepeatUntil::new(self.factory, until, self.repeat_every, self.inspect_err)
    }
}

/// Returns `Poll::Ready(())` if there is no need to wait for the timeout.
fn poll_timeout(timeout_fut: &mut Option<Timer>, cx: &mut Context<'_>) -> Poll<()> {
    let mut timeout = match timeout_fut.take() {
        Some(timeout) => timeout,
        None => return Poll::Ready(()),
    };

    match timeout.poll_unpin(cx) {
        Poll::Ready(_) => Poll::Ready(()),
        Poll::Pending => {
            *timeout_fut = Some(timeout);
            Poll::Pending
        },
    }
}

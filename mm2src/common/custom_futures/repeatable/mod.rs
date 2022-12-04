//! A future that can be repeated if an error occurs or not all conditions are met.
//!
//! # Why `async move` shouldn't be allowed
//!
//! Let's consider the following example:
//!
//! ```rust
//! let mut counter = 0;
//! let res = repeatable!(async move {
//!   counter += 1;
//!   if counter > 1 { ready!() } else { retry!() }
//! }).repeat_every_secs(0.1).attempts(10).await;
//!
//! res.expect_err("'counter' will never be greater than 1");
//! ```
//!
//! This happens due to the fact that the `counter` variable is not shared between attempts,
//! and every time the future starts with `counter = 0`.

use crate::executor::Timer;
use crate::now_ms;
use futures::FutureExt;
use log::warn;
use std::future::Future;
use std::marker::PhantomData;
use std::task::{Context, Poll};
use std::time::Duration;
use wasm_timer::Instant;

mod with_attempts;
mod with_timeout;

pub use with_attempts::{AttemptsExceed, RepeatAttempts};
pub use with_timeout::{RepeatUntil, TimeoutExpired, Until};

/// Wraps the given future into `Repeatable` future.
/// The future should return [`Action<T, E>`] with any `T` and `E` types.
#[macro_export]
macro_rules! repeatable {
    (async { $($t:tt)* }) => {
        $crate::custom_futures::repeatable::Repeatable::new(|| Box::pin(async { $($t)* }))
    };
    ($fut:expr) => {
        $crate::custom_futures::repeatable::Repeatable::new(|| $fut)
    };
}

/// Wraps the given future into `Repeatable` future.
/// The future should return [`Result<T, E>`], where
/// * `Ok(T)` => `Action::Ready(T)`
/// * `Err(E)` => `Action::Retry(E)`
#[macro_export]
macro_rules! retry_on_err {
    (async { $($t:tt)* }) => {
        $crate::custom_futures::repeatable::Repeatable::new(|| {
            use $crate::custom_futures::repeatable::RetryOnError;
            use futures::FutureExt;

            let fut = async { $($t)* };
            Box::pin(fut.map(Result::retry_on_err))
        })
    };
    ($fut:expr) => {
        $crate::custom_futures::repeatable::Repeatable::new(|| {
            use $crate::custom_futures::repeatable::RetryOnError;
            use futures::FutureExt;

            $fut.map(Result::retry_on_err)
        })
    };
}

/// The macro expands as `return Action::Ready(T)`.
#[macro_export]
macro_rules! ready {
    () => {{
        return $crate::custom_futures::repeatable::Action::Ready(());
    }};
    ($res:expr) => {{
        return $crate::custom_futures::repeatable::Action::Ready($res);
    }};
}

/// The macro expands as `return Action::Retry(E)`.
#[macro_export]
macro_rules! retry {
    () => {{
        return $crate::custom_futures::repeatable::Action::Retry(());
    }};
    ($err:expr) => {{
        return $crate::custom_futures::repeatable::Action::Retry($err);
    }};
}

/// Unwraps a result or returns `Action::Retry(E)`.
#[macro_export]
macro_rules! try_or_retry {
    ($exp:expr) => {{
        match $exp {
            Ok(t) => t,
            Err(e) => $crate::retry!(e),
        }
    }};
}

/// Unwraps a result or returns `Action::Ready(E)`.
#[macro_export]
macro_rules! try_or_ready_err {
    ($exp:expr) => {{
        match $exp {
            Ok(t) => t,
            Err(e) => $crate::ready!(Err(e)),
        }
    }};
}

pub trait FactoryTrait<F>: Fn() -> F {}

impl<Factory, F> FactoryTrait<F> for Factory where Factory: Fn() -> F {}

pub trait RepeatableTrait<T, E>: Future<Output = Action<T, E>> + Unpin {}

impl<F, T, E> RepeatableTrait<T, E> for F where F: Future<Output = Action<T, E>> + Unpin {}

pub(crate) trait InspectErrorTrait<E>: 'static + Fn(&E) + Send {}

impl<F: 'static + Fn(&E) + Send, E> InspectErrorTrait<E> for F {}

/// The future is ether ready (with a `T` result), or not ready (failed with an intermediate `E` error).
#[derive(Debug)]
pub enum Action<T, E> {
    Ready(T),
    Retry(E),
}

pub trait RetryOnError<T, E> {
    fn retry_on_err(self) -> Action<T, E>;
}

impl<T, E> RetryOnError<T, E> for Result<T, E> {
    /// Converts `Result<T, E>` into `Action<T, E>`:
    /// * `Ok(T)` => `Action::Ready(T)`.
    /// * `Err(E)` => `Action::Retry(E)`.
    #[inline]
    fn retry_on_err(self) -> Action<T, E> {
        match self {
            Ok(ready) => Action::Ready(ready),
            Err(e) => Action::Retry(e),
        }
    }
}

/// The result of `repeatable` or `retry_on_err` macros - the first step at the future configuration.
pub struct Repeatable<Factory, F, T, E> {
    factory: Factory,
    inspect_err: Option<Box<dyn InspectErrorTrait<E>>>,
    _phantom: PhantomData<(F, T, E)>,
}

impl<Factory, F, T, E> Repeatable<Factory, F, T, E> {
    #[inline]
    pub fn new(factory: Factory) -> Self {
        Repeatable {
            factory,
            inspect_err: None,
            _phantom: PhantomData::default(),
        }
    }

    /// Specifies an inspect handler that does something with an error on each unsuccessful attempt.
    #[inline]
    pub fn inspect_err<Inspect>(mut self, inspect: Inspect) -> Self
    where
        Inspect: 'static + Fn(&E) + Send,
    {
        self.inspect_err = Some(Box::new(inspect));
        self
    }

    #[inline]
    pub fn repeat_every(self, repeat_every: Duration) -> RepeatEvery<Factory, F, T, E> {
        RepeatEvery {
            factory: self.factory,
            repeat_every,
            inspect_err: self.inspect_err,
            _phantom: PhantomData::default(),
        }
    }

    #[inline]
    pub fn repeat_every_ms(self, repeat_every: u64) -> RepeatEvery<Factory, F, T, E> {
        self.repeat_every(Duration::from_millis(repeat_every))
    }

    #[inline]
    pub fn repeat_every_secs(self, repeat_every: f64) -> RepeatEvery<Factory, F, T, E> {
        self.repeat_every(Duration::from_secs_f64(repeat_every))
    }
}

/// The next step at the future configuration `Repeatable` -> `RepeatEvery`.
pub struct RepeatEvery<Factory, F, T, E> {
    factory: Factory,
    repeat_every: Duration,
    inspect_err: Option<Box<dyn InspectErrorTrait<E>>>,
    _phantom: PhantomData<(F, T, E)>,
}

impl<Factory, F, T, E> RepeatEvery<Factory, F, T, E> {
    /// Specifies an inspect handler that does something with an error on each unsuccessful attempt.
    #[inline]
    pub fn inspect_err<Inspect>(mut self, inspect: Inspect) -> Self
    where
        Inspect: 'static + Fn(&E) + Send,
    {
        self.inspect_err = Some(Box::new(inspect));
        self
    }

    /// Specifies a total number of attempts to run the future.
    /// So there will be up to `total_attempts`.
    #[inline]
    pub fn attempts(self, total_attempts: usize) -> RepeatAttempts<Factory, F, T, E>
    where
        Factory: FactoryTrait<F>,
        F: RepeatableTrait<T, E>,
    {
        if total_attempts == 0 {
            warn!("There will be 1 attempt even though 'total_attempts' is 0");
        }

        RepeatAttempts::new(
            self.factory,
            self.repeat_every.as_secs_f64(),
            self.inspect_err,
            total_attempts,
        )
    }

    /// Specifies a deadline before that we may try to repeat the future.
    #[inline]
    pub fn until(self, until: Instant) -> RepeatUntil<Factory, F, T, E>
    where
        Factory: FactoryTrait<F>,
        F: RepeatableTrait<T, E>,
    {
        let now = Instant::now();
        if now > until {
            warn!("Deadline is reached already: now={now:?} until={until:?}")
        }

        RepeatUntil::new(self.factory, Until::Instant(until), self.repeat_every, self.inspect_err)
    }

    /// Specifies a deadline in milliseconds before that we may try to repeat the future.
    #[inline]
    pub fn until_ms(self, until_ms: u64) -> RepeatUntil<Factory, F, T, E>
    where
        Factory: FactoryTrait<F>,
        F: RepeatableTrait<T, E>,
    {
        let now = now_ms();
        if now >= until_ms {
            warn!("Deadline is reached already: now={now:?} until={until_ms:?}")
        }

        RepeatUntil::new(
            self.factory,
            Until::TimestampMs(until_ms),
            self.repeat_every,
            self.inspect_err,
        )
    }

    /// Specifies a timeout in milliseconds before that we may try to repeat the future.
    /// Note this method name should differ from [`FutureTimerExt::timeout_ms`].
    #[inline]
    pub fn with_timeout_ms(self, timeout_ms: u64) -> RepeatUntil<Factory, F, T, E>
    where
        Factory: FactoryTrait<F>,
        F: RepeatableTrait<T, E>,
    {
        self.until_ms(now_ms() + timeout_ms)
    }

    /// Specifies a timeout in seconds before that we may try to repeat the future.
    /// Note this method name should differ from [`FutureTimerExt::timeout_secs`].
    #[inline]
    pub fn with_timeout_secs(self, timeout_secs: f64) -> RepeatUntil<Factory, F, T, E>
    where
        Factory: FactoryTrait<F>,
        F: RepeatableTrait<T, E>,
    {
        let timeout_ms = (timeout_secs * 1000.) as u64;
        self.until_ms(now_ms() + timeout_ms)
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

use std::fmt;
use std::pin::Pin;
use std::task::Context;
use std::time::Duration;

use futures::task::Poll as Poll03;
use futures::Future as Future03;

use crate::executor::Timer;

#[derive(Debug)]
pub struct TimeoutError {
    pub duration: Duration,
}

impl fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}s timed out waiting for the future to complete",
            self.duration.as_secs_f64()
        )
    }
}

/// Unlike `futures_timer::FutureExt` and Tokio timers, this trait implementation works with any reactor and on WASM arch.
pub trait FutureTimerExt: Future03 + Sized {
    /// Finishes with `TimeoutError` if the underlying future isn't ready withing the given timeframe.
    fn timeout(self, duration: Duration) -> Timeout<Self> {
        Timeout {
            future: self,
            timer: Timer::sleep(duration.as_secs_f64()),
            duration,
        }
    }

    fn timeout_secs(self, secs: f64) -> Timeout<Self> {
        Timeout {
            future: self,
            timer: Timer::sleep(secs),
            duration: Duration::from_secs_f64(secs),
        }
    }
}

impl<F: Future03 + Sized> FutureTimerExt for F {}

/// Future returned by the `FutureTimerExt::timeout` method.
#[must_use = "futures do nothing unless polled"]
pub struct Timeout<F: Sized> {
    future: F,
    timer: Timer,
    duration: Duration,
}

impl<F> Future03 for Timeout<F>
where
    F: Future03 + Unpin,
{
    type Output = Result<F::Output, TimeoutError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll03<Self::Output> {
        match Future03::poll(Pin::new(&mut self.future), cx) {
            Poll03::Ready(out) => return Poll03::Ready(Ok(out)),
            Poll03::Pending => (),
        }
        match Future03::poll(Pin::new(&mut self.timer), cx) {
            Poll03::Ready(()) => Poll03::Ready(Err(TimeoutError {
                duration: self.duration,
            })),
            Poll03::Pending => Poll03::Pending,
        }
    }
}

unsafe impl<F> Send for Timeout<F> where F: Send {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout() {
        let _err =
            crate::block_on(Timer::sleep(0.4).timeout(Duration::from_secs_f64(0.1))).expect_err("Expected timeout");
        crate::block_on(Timer::sleep(0.1).timeout(Duration::from_secs_f64(0.2))).expect("Expected future");
    }
}

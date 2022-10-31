/// Custom future combinators/implementations - some of standard do not match our requirements.
use crate::executor::Timer;

use futures::task::Poll as Poll03;
use futures::Future as Future03;
use futures01::future::{self, loop_fn, Either as Either01, IntoFuture, Loop};
use futures01::stream::{Fuse, Stream};
use futures01::{Async, AsyncSink, Future, Poll, Sink};

use std::fmt;
use std::pin::Pin;
use std::task::Context;
use std::time::Duration;

/// The analogue of join_all combinator running futures `sequentially`.
/// `join_all` runs futures `concurrently` which cause issues with native coins daemons RPC.
/// We need to get raw transactions containing unspent outputs when we build new one in order
/// to get denominated integer amount of UTXO instead of f64 provided by `listunspent` call.
/// Sometimes we might need info about dozens (or even hundreds) transactions at time so we can overflow
/// RPC queue of daemon very fast like this: https://github.com/bitpay/bitcore-node/issues/463#issuecomment-228788871.
/// Thx to https://stackoverflow.com/a/51717254/8707622
pub fn join_all_sequential<I>(
    i: I,
) -> impl Future<Item = Vec<<I::Item as IntoFuture>::Item>, Error = <I::Item as IntoFuture>::Error>
where
    I: IntoIterator,
    I::Item: IntoFuture,
{
    let iter = i.into_iter();
    loop_fn((vec![], iter), |(mut output, mut iter)| {
        let fut = if let Some(next) = iter.next() {
            Either01::A(next.into_future().map(Some))
        } else {
            Either01::B(future::ok(None))
        };

        fut.and_then(move |val| {
            if let Some(val) = val {
                output.push(val);
                Ok(Loop::Continue((output, iter)))
            } else {
                Ok(Loop::Break(output))
            }
        })
    })
}

/// The analogue of select_ok combinator running futures `sequentially`.
/// The use case of such combinator is Electrum (and maybe not only Electrum) multiple servers support.
/// Electrum client uses shared HashMap to store responses and we can treat the first received response as
/// error while it's really successful. We might change the Electrum support design in the future to avoid
/// such race condition but `select_ok_sequential` might be still useful to reduce the networking overhead.
/// There is no reason actually to send same request to all servers concurrently when it's enough to use just 1.
/// But we do a kind of round-robin if first server fails to respond, etc, and we return error only if all servers attempts failed.
/// When a server responds successfully we return the response and the number of failed attempts in a tuple.
pub fn select_ok_sequential<I: IntoIterator>(
    i: I,
) -> impl Future<Item = (<I::Item as IntoFuture>::Item, usize), Error = Vec<<I::Item as IntoFuture>::Error>>
where
    I::Item: IntoFuture,
{
    let futures = i.into_iter();
    loop_fn((vec![], futures), |(mut errors, mut futures)| {
        let fut = if let Some(next) = futures.next() {
            Either01::A(next.into_future().map(Some))
        } else {
            Either01::B(future::ok(None))
        };

        fut.then(move |val| {
            let val = match val {
                Ok(val) => val,
                Err(e) => {
                    errors.push(e);
                    return Ok(Loop::Continue((errors, futures)));
                },
            };

            if let Some(val) = val {
                Ok(Loop::Break((val, errors.len())))
            } else {
                Err(errors)
            }
        })
    })
}

/// Future for the `Sink::send_all` combinator, which sends a stream of values
/// to a sink and then waits until the sink has fully flushed those values.
/// The difference from standard implementation is this SendAll returns the `Stream` part even in case of errors.
/// It's useful for Electrum connections (based on MPSC channels):
/// If we get connection error standard SendAll will consume the receiver but it can still
/// receive messages from sender bypassing them to new TcpStream created in loop_fn.
#[derive(Debug)]
#[must_use = "futures do nothing unless polled"]
pub struct SendAll<T, U: Stream> {
    sink: Option<T>,
    stream: Option<Fuse<U>>,
    buffered: Option<U::Item>,
}

impl<T, U> SendAll<T, U>
where
    T: Sink,
    U: Stream<Item = T::SinkItem>,
    T::SinkError: From<U::Error>,
{
    fn sink_mut(&mut self) -> &mut T {
        self.sink
            .as_mut()
            .take()
            .expect("Attempted to poll SendAll after completion")
    }

    pub fn new(sink: T, stream: U) -> SendAll<T, U> {
        SendAll {
            sink: Some(sink),
            stream: Some(stream.fuse()),
            buffered: None,
        }
    }

    fn stream_mut(&mut self) -> &mut Fuse<U> {
        self.stream
            .as_mut()
            .take()
            .expect("Attempted to poll SendAll after completion")
    }

    fn take_stream(&mut self) -> U {
        let fuse = self.stream.take().expect("Attempted to poll Forward after completion");
        fuse.into_inner()
    }

    fn take_result(&mut self) -> (T, U) {
        let sink = self.sink.take().expect("Attempted to poll Forward after completion");
        let fuse = self.stream.take().expect("Attempted to poll Forward after completion");
        (sink, fuse.into_inner())
    }

    fn try_start_send(&mut self, item: U::Item) -> Poll<(), T::SinkError> {
        debug_assert!(self.buffered.is_none());
        if let AsyncSink::NotReady(item) = self.sink_mut().start_send(item)? {
            self.buffered = Some(item);
            return Ok(Async::NotReady);
        }
        Ok(Async::Ready(()))
    }
}

macro_rules! try_ready_send_all {
    ($selff: ident, $e:expr) => {
        match $e {
            Ok(Async::Ready(t)) => t,
            Ok(Async::NotReady) => return Ok(Async::NotReady),
            Err(e) => return Err(($selff.take_stream(), From::from(e))),
        }
    };
}

impl<T, U> Future for SendAll<T, U>
where
    T: Sink,
    U: Stream<Item = T::SinkItem>,
    T::SinkError: From<U::Error>,
{
    type Item = (T, U);
    type Error = (U, T::SinkError);

    fn poll(&mut self) -> Poll<(T, U), (U, T::SinkError)> {
        // If we've got an item buffered already, we need to write it to the
        // sink before we can do anything else
        if let Some(item) = self.buffered.take() {
            try_ready_send_all!(self, self.try_start_send(item));
        }

        loop {
            match self
                .stream_mut()
                .poll()
                .map_err(|e| (self.take_stream(), From::from(e)))?
            {
                Async::Ready(Some(item)) => try_ready_send_all!(self, self.try_start_send(item)),
                Async::Ready(None) => {
                    try_ready_send_all!(self, self.sink_mut().close());
                    return Ok(Async::Ready(self.take_result()));
                },
                Async::NotReady => {
                    try_ready_send_all!(self, self.sink_mut().poll_complete());
                    return Ok(Async::NotReady);
                },
            }
        }
    }
}

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

#[test]
fn test_timeout() {
    let _err = crate::block_on(Timer::sleep(0.4).timeout(Duration::from_secs_f64(0.1))).expect_err("Expected timeout");
    let _ok = crate::block_on(Timer::sleep(0.1).timeout(Duration::from_secs_f64(0.2))).expect("Expected future");
}

/// Custom future combinators/implementations - some of standard do not match our requirements.
use futures01::future::{self, loop_fn, Either as Either01, IntoFuture, Loop};
use futures01::Future;

pub mod repeatable;
pub mod timeout;

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

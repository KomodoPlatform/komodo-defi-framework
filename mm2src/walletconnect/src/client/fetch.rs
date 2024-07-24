use super::outbound::{create_request, ResponseFuture};

use crate::client::Client;
use crate::error::ServiceErrorExt;

use futures_util::stream::{Stream, StreamExt};
use futures_util::FutureExt;
use relay_rpc::domain::Topic;
use relay_rpc::rpc::{BatchFetchMessages, ServiceRequest, SubscriptionData};
use std::task::Poll;

/// Stream that uses the `irn_batchFetch` RPC method to retrieve messages from
/// the Relay.
pub struct FetchMessageStream {
    client: Client,
    request: BatchFetchMessages,
    batch: Option<std::vec::IntoIter<SubscriptionData>>,
    batch_fut: Option<ResponseFuture<BatchFetchMessages>>,
    has_more: bool,
}

impl FetchMessageStream {
    pub fn new(client: Client, topics: impl Into<Vec<Topic>>) -> Self {
        let request = BatchFetchMessages { topics: topics.into() };

        Self {
            client,
            request,
            batch: None,
            batch_fut: None,
            has_more: true,
        }
    }

    /// Clears all internal state so that on the next stream poll it returns
    /// `None` and finishes data streaming.
    #[inline]
    pub fn clear(&mut self) {
        self.batch = None;
        self.batch_fut = None;
        self.has_more = false;
    }
}

impl Stream for FetchMessageStream {
    type Item = Result<SubscriptionData, ServiceErrorExt<<BatchFetchMessages as ServiceRequest>::Error>>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        loop {
            if let Some(batch) = self.batch.as_mut() {
                // drain item from the batch if we have one.
                if let Some(data) = batch.next() {
                    return Poll::Ready(Some(Ok(data)));
                };
                // batch is empty.
                self.batch = None;
            } else if let Some(fut) = self.batch_fut.as_mut() {
                match fut.poll_unpin(cx) {
                    Poll::Ready(Ok(data)) => {
                        // The next batch is ready. Update `has_more` flag and clear the batch future.
                        self.batch = Some(data.messages.into_iter());
                        self.has_more = data.has_more;
                        self.batch_fut = None;
                    },
                    Poll::Ready(Err(err)) => {
                        // Error receiving the next batch. This is unrecoverable, so clear the state and
                        // end the stream
                        self.clear();
                        return Poll::Ready(Some(Err(err)));
                    },
                    Poll::Pending => return Poll::Pending,
                }
            } else if self.has_more {
                let (req, fut) = create_request(self.request.clone());
                // call self.client.request(req);
                self.batch_fut = Some(fut);
            } else {
                // The stream can't produce any more items, since it doesn't have neither a
                // batch of data or a future for receiving the next batch, and `has_more` flag
                // is not set.
                return Poll::Ready(None);
            }
        }
    }
}

// cf. https://github.com/tower-rs/tower-hyper/blob/master/src/body.rs
// cf. https://github.com/hyperium/hyper/issues/1782#issuecomment-473129843
// cf. https://github.com/hyperium/http-body/issues/2

use futures01::Poll;
use http_body::Body as HttpBody;
use hyper::body::Payload;

pub use hyper::Body;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Lifts a body to support `Payload`
#[derive(Debug)]
pub struct LiftBody<T> {inner: T}

impl<T> LiftBody<T> {
    pub fn into_inner (self) -> T {self.inner}
}

impl<T: HttpBody> From<T> for LiftBody<T> {
    fn from (inner: T) -> Self {LiftBody {inner}}
}

impl<T> Payload for LiftBody<T>
where
    T: HttpBody + Send + 'static,
    T::Data: Send,
    T::Error: Into<Error>
{
    type Data = T::Data;
    type Error = T::Error;

    fn poll_data (&mut self) -> Poll<Option<Self::Data>, Self::Error> {
        self.inner.poll_data()
    }

    fn poll_trailers (&mut self) -> Poll<Option<hyper::HeaderMap>, Self::Error> {
        self.inner.poll_trailers()
    }

    fn is_end_stream (&self) -> bool {
        self.inner.is_end_stream()
    }
}

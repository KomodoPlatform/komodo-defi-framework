use web3::{api::Namespace,
           helpers::{self, CallFuture},
           types::{Address, U256},
           Transport};

/// `ParityNonce` namespace.
#[derive(Debug, Clone)]
pub(crate) struct ParityNonce<T> {
    transport: T,
}

impl<T: Transport> Namespace<T> for ParityNonce<T> {
    fn new(transport: T) -> Self
    where
        Self: Sized,
    {
        ParityNonce { transport }
    }

    fn transport(&self) -> &T { &self.transport }
}

impl<T: Transport> ParityNonce<T> {
    /// Parity next nonce.
    pub(crate) fn parity_next_nonce(&self, addr: Address) -> CallFuture<U256, T::Out> {
        let addr = helpers::serialize(&addr);
        CallFuture::new(self.transport.execute("parity_nextNonce", vec![addr]))
    }
}

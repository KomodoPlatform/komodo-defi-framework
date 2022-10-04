use futures::Future as Future03;

pub trait BoxFutureSpawner {
    fn spawn_boxed(&self, f: Box<dyn Future03<Output = ()> + Send + Unpin + 'static>);
}

impl<S: SpawnFuture> BoxFutureSpawner for S {
    fn spawn_boxed(&self, f: Box<dyn Future03<Output = ()> + Send + Unpin + 'static>) { self.spawn(f) }
}

pub trait SpawnFuture {
    /// Spawns the given `f` future.
    fn spawn<F>(&self, f: F)
    where
        F: Future03<Output = ()> + Send + 'static;
}

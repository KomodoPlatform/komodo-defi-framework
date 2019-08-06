// "It is the executor’s job to call `poll` on the task until `Ready(())` is returned."
// -- https://tokio.rs/docs/internals/runtime-model/

// Invoked from HTTP server the helpers will enjoy full native support for futures and threads.
// The portable code, on the other hand, will need this module
// in order to work with futures without the native threads and I/O.

// TODO: The portable executor should be available under `CORE.spawn`.

use futures03::{FutureExt};
use futures03::task::{waker_ref, ArcWake, Context, Poll};
use futures03::future::BoxFuture;
use std::future::Future;
use std::sync::{Arc, Mutex};

struct Task {future: Mutex<BoxFuture<'static, ()>>}

impl ArcWake for Task {
    fn wake_by_ref (arc_self: &Arc<Self>) {
        // Currently a NOP because `run` will run all the tasks anyway.  
        // Should later reimplement it to wake a specific future.  
        // cf. https://rust-lang.github.io/async-book/02_execution/04_executor.html
    }
}

lazy_static! {static ref TASKS: Mutex<Vec<Arc<Task>>> = Mutex::new (Vec::new());}

pub fn spawn (future: impl Future<Output = ()> + 'static + Send) {
    let future = future.boxed();
    let task = Arc::new (Task {future: Mutex::new (future)});
    unwrap! (TASKS.lock()) .push (task)
}

pub fn run() {
    let mut tasks = unwrap! (TASKS.lock());
    tasks.retain (|task| {
        let mut future = unwrap! (task.future.lock());
        let waker = waker_ref (&task);
        let context = &mut Context::from_waker (&*waker);
        if let Poll::Pending = future.as_mut().poll (context) {
            true  // Retain, we're not done yet.
        } else {
            log! ("executor] task finished!");
            false  // Evict, we're done here.
        }
    })
}

/// This native export allows the WASM host to run the executor via the WASM FFI.  
/// TODO: Start a thread from the `start_helpers` instead.
#[no_mangle]
pub unsafe extern fn run_executor() {run()}

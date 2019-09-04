// "It is the executor’s job to call `poll` on the task until `Ready(())` is returned."
// -- https://tokio.rs/docs/internals/runtime-model/

// Invoked from HTTP server the helpers will enjoy full native support for futures and threads.
// The portable code, on the other hand, will need this module
// in order to work with futures without the native threads and I/O.

use crate::now_float;
use atomic::Atomic;
use futures::FutureExt;
use futures::executor::enter;
use futures::future::BoxFuture;
use futures::task::{waker_ref, ArcWake, Context, Poll};
use std::future::Future;
use std::mem::swap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;

struct Task {
    future: Mutex<BoxFuture<'static, ()>>,
    /// We can skip running the task till its alarm clock goes off.
    alarm_clock: Atomic<f64>
}

impl ArcWake for Task {
    fn wake_by_ref (arc_self: &Arc<Self>) {
        arc_self.alarm_clock.store (0., Ordering::Relaxed)
}   }

lazy_static! {
    static ref TASKS: Mutex<Vec<Arc<Task>>> = Mutex::new (Vec::new());
    /// Spawned tasks go into this separate queue first
    /// in order to allow the running tasks to spawn new tasks and not upset the `TASKS` lock.
    static ref NEW_TASKS: Mutex<Vec<Arc<Task>>> = Mutex::new (Vec::new());
}

pub fn spawn (future: impl Future<Output = ()> + Send + 'static) {
    spawn_after (0., future)
}

/// Schedule the given `future` to be executed shortly after the given `utc` time is reached.
pub fn spawn_after (utc: f64, future: impl Future<Output = ()> + Send + 'static) {
    let future = future.boxed();
    let task = Arc::new (Task {future: Mutex::new (future), alarm_clock: Atomic::new (utc)});
    unwrap! (NEW_TASKS.lock()) .push (task)
}

pub fn run() {
    let mut new_tasks = Vec::new();
    swap (&mut new_tasks, &mut* unwrap! (NEW_TASKS.lock()));

    let mut tasks = unwrap! (TASKS.lock());
    for new_task in new_tasks {tasks.push (new_task)}
    let enter = enter().expect ("!enter");
    let now = now_float();
    tasks.retain (|task| {
        // As an optimization, and in order to maintain the proper task waking logic,
        // we're going to skip the tasks which aren't quite ready to run yet.
        let alarm_clock = task.alarm_clock.load (Ordering::Relaxed);
        if now < alarm_clock {return true}  // See you later.

        // Pre-schedule the task into waking up a bit later.
        // The underlying task future can speed things up by using the `Waker`.
        let later = now + 2.;  // Bump this up to test the `Waker` code.
        let _ = task.alarm_clock.compare_exchange (alarm_clock, later, Ordering::Relaxed, Ordering::Relaxed);

        let mut future = unwrap! (task.future.lock());
        let waker = waker_ref (&task);
        let context = &mut Context::from_waker (&*waker);
        if let Poll::Pending = future.as_mut().poll (context) {
            true  // Retain, we're not done yet.
        } else {
            false  // Evict, we're done here.
        }
    });
    drop (enter)
}

/// This native export allows the WASM host to run the executor via the WASM FFI.  
/// TODO: Start a thread from the `start_helpers` instead.
#[no_mangle]
pub unsafe extern fn run_executor() {run()}

/// A future that completes at a given time.  
pub struct Timer {till_utc: f64}

impl Timer {
    pub fn till (till_utc: f64) -> Timer {Timer {till_utc}}
    pub fn sleep (seconds: f64) -> Timer {Timer {till_utc: now_float() + seconds}}
    pub fn till_utc (&self) -> f64 {self.till_utc}
}

impl Future for Timer {
    type Output = ();
    fn poll (self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        if self.till_utc - now_float() <= 0. {return Poll::Ready(())}

        // NB: We should get a new `Waker` on every `poll` in case the future migrates between executors.
        // cf. https://rust-lang.github.io/async-book/02_execution/03_wakeups.html
        let waker = cx.waker().clone();
        spawn_after (self.till_utc, async {waker.wake()});

        Poll::Pending
    }
}

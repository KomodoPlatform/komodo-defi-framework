use futures::task::Context;
use futures::task::Poll as Poll03;
use futures::Future as Future03;
use gstuff::now_float;
use std::pin::Pin;
use std::thread;
use std::time::Duration;

pub fn spawn(future: impl Future03<Output = ()> + Send + 'static) { crate::wio::CORE.0.spawn(future); }

pub fn spawn_boxed(future: Box<dyn Future03<Output = ()> + Send + Unpin + 'static>) { spawn(future); }

/// Schedule the given `future` to be executed shortly after the given `utc` time is reached.
pub fn spawn_after(utc: f64, future: impl Future03<Output = ()> + Send + 'static) {
    use crossbeam::channel;
    use gstuff::Constructible;
    use std::collections::BTreeMap;
    use std::sync::Once;

    type SheduleChannelItem = (f64, Pin<Box<dyn Future03<Output = ()> + Send + 'static>>);
    static START: Once = Once::new();
    static SCHEDULE: Constructible<channel::Sender<SheduleChannelItem>> = Constructible::new();
    START.call_once(|| {
        thread::Builder::new()
            .name("spawn_after".into())
            .spawn(move || {
                let (tx, rx) = channel::bounded(0);
                SCHEDULE.pin(tx).expect("spawn_after] Can't pin the channel");
                type Task = Pin<Box<dyn Future03<Output = ()> + Send + 'static>>;
                let mut tasks: BTreeMap<Duration, Vec<Task>> = BTreeMap::new();
                let mut ready = Vec::with_capacity(4);
                loop {
                    let now = Duration::from_secs_f64(now_float());
                    let mut next_stop = Duration::from_secs_f64(0.1);
                    for (utc, _) in tasks.iter() {
                        if *utc <= now {
                            ready.push(*utc)
                        } else {
                            next_stop = *utc - now;
                            break;
                        }
                    }
                    for utc in ready.drain(..) {
                        let v = match tasks.remove(&utc) {
                            Some(v) => v,
                            None => continue,
                        };
                        //log! ("spawn_after] spawning " (v.len()) " tasks at " [utc]);
                        for f in v {
                            spawn(f)
                        }
                    }
                    let (utc, f) = match rx.recv_timeout(next_stop) {
                        Ok(t) => t,
                        Err(channel::RecvTimeoutError::Disconnected) => break,
                        Err(channel::RecvTimeoutError::Timeout) => continue,
                    };
                    tasks
                        .entry(Duration::from_secs_f64(utc))
                        .or_insert_with(Vec::new)
                        .push(f)
                }
            })
            .expect("Can't spawn a spawn_after thread");
    });
    loop {
        match SCHEDULE.as_option() {
            None => {
                thread::yield_now();
                continue;
            },
            Some(tx) => {
                tx.send((utc, Box::pin(future))).expect("Can't reach spawn_after");
                break;
            },
        }
    }
}

/// A future that completes at a given time.  
pub struct Timer {
    till_utc: f64,
}

impl Timer {
    pub fn till(till_utc: f64) -> Timer { Timer { till_utc } }
    pub fn sleep(seconds: f64) -> Timer {
        Timer {
            till_utc: now_float() + seconds,
        }
    }
    pub fn sleep_ms(ms: u32) -> Timer {
        let seconds = gstuff::duration_to_float(Duration::from_millis(ms as u64));
        Timer {
            till_utc: now_float() + seconds,
        }
    }
    pub fn till_utc(&self) -> f64 { self.till_utc }
}

impl Future03 for Timer {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll03<Self::Output> {
        let delta = self.till_utc - now_float();
        if delta <= 0. {
            return Poll03::Ready(());
        }
        // NB: We should get a new `Waker` on every `poll` in case the future migrates between executors.
        // cf. https://rust-lang.github.io/async-book/02_execution/03_wakeups.html
        let waker = cx.waker().clone();
        spawn_after(self.till_utc, async move { waker.wake() });
        Poll03::Pending
    }
}

#[test]
fn test_timer() {
    let started = now_float();
    let ti = Timer::sleep(0.2);
    let delta = now_float() - started;
    assert!(delta < 0.04, "{}", delta);
    super::block_on(ti);
    let delta = now_float() - started;
    println!("time delta is {}", delta);
    assert!(delta > 0.2);
    assert!(delta < 0.4)
}

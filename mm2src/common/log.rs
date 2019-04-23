//! Human-readable logging and statuses.

// TODO: As we discussed with Artem, skip a status update if it is equal to the previous update.
// TODO: Sort the tags while converting `&[&TagParam]` to `Vec<Tag>`.

use chrono::{Local, TimeZone, Utc};
use chrono::format::DelayedFormat;
use chrono::format::strftime::StrftimeItems;
use crossbeam::queue::SegQueue;
use gstuff::now_ms;
use libc::{c_char, c_int};
use regex::Regex;
use serde_json::{Value as Json};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::default::Default;
use std::env;
use std::fs;
use std::fmt;
use std::fmt::Write as WriteFmt;
use std::io::{Seek, SeekFrom, Write};
use std::mem::swap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

lazy_static! {
    /// True if we're likely to be under a capturing cargo test.
    static ref CAPTURING_TEST: bool = {
        // See if we're running under a test.
        // Only the "cargo test" uses the "deps" dir to run the MM2.
        // Plus `mm2-\w+` is a giveaway.
        let ex = unwrap! (Regex::new (r#"(?x) target [/\\] debug [/\\] deps [/\\] mm2-\w+ (.exe)? $"#));
        let mut args = env::args();
        let cmd = unwrap! (args.next());
        if ex.is_match (&cmd) {
            !args.any (|a| a == "--nocapture")
        } else {false}
    };
    static ref PRINTF_LOCK: Mutex<()> = Mutex::new(());
    /// If this C callback is present then all the logging output should happen through it
    /// (and leaving stdout untouched).
    pub static ref LOG_OUTPUT: Mutex<Option<extern fn (line: *const c_char)>> = Mutex::new (None);
}

#[cfg(windows)]
fn flush_stdout() {
    // I don't see the `stdout` in the Windows version of the `libc` crate, but `_flushall` comes to the rescue.
    // https://docs.microsoft.com/en-us/cpp/c-runtime-library/reference/flushall?view=vs-2017
    extern "C" {fn _flushall() -> c_int;}
    unsafe {_flushall()};
}

#[cfg(not(windows))]
fn flush_stdout() {}

/// Initialized and used when there's a need to chute the logging into a given thread.
struct Gravity {
    /// The center of gravity, the thread where the logging should reach the `println!` output.
    target_thread_id: thread::ThreadId,
    /// Log chunks received from satellite threads.
    landing: SegQueue<String>,
    /// Keeps a portiong of a recently flushed gravity log in RAM for inspection by the unit tests.
    tail: Mutex<VecDeque<String>>
}

impl Gravity {
    /// Files a log chunk to be logged from the center of gravity thread.
    fn chunk2log (&self, chunk: String) {
        self.landing.push (chunk);
        if thread::current().id() == self.target_thread_id {
            self.flush()
        }
    }
    /// Prints the collected log chunks.  
    /// `println!` is used for compatibility with unit test stdout capturing.
    fn flush (&self) {
        let mut tail = self.tail.lock();
        while let Some (chunk) = self.landing.try_pop() {
            println! ("{}", chunk);
            if let Ok (ref mut tail) = tail {
                if tail.len() == tail.capacity() {let _ = tail.pop_front();}
                tail.push_back (chunk)
    }   }   }
}

thread_local! {
    /// If set, pulls the `chunk2log` (aka `log!`) invocations into the gravity of another thread.
    static GRAVITY: RefCell<Option<Arc<Gravity>>> = RefCell::new (None)
}

#[doc(hidden)]
pub fn chunk2log (mut chunk: String) {
    extern {fn printf(_: *const c_char, ...) -> c_int;}

    if let Ok (log_output) = LOG_OUTPUT.lock() {
        if let Some (log_cb) = *log_output {
            chunk.push ('\0');
            log_cb (chunk.as_ptr() as *const c_char);
            return
        }
    }

    // NB: Using gravity even in the non-capturing tests in order to give the tests access to the gravity tail.
    let rc = GRAVITY.try_with (|gravity| {
        if let Some (ref gravity) = *gravity.borrow() {
            let mut chunkʹ = String::new();
            swap (&mut chunk, &mut chunkʹ);
            gravity.chunk2log (chunkʹ);
            true
        } else {
            false
        }
    });
    match rc {Ok (true) => return, _ => ()}

    // The C and the Rust standard outputs overlap on Windows,
    // so we use the C `printf` in order to avoid that.

    // On the other hand, on Darwin the `printf` is getting buffered or something, breaking some tests,
    // so we only want to use the `printf` on Windows and not on Darwin.

    // Also, `printf` isn't captured by the Rust tests,
    // so we should fall back to `println!` while running under a capturing test.

    if cfg! (not (windows)) || *CAPTURING_TEST {
        println! ("{}", chunk)
    } else {
        chunk.push ('\n');
        chunk.push ('\0');
        if let Ok (_lock) = PRINTF_LOCK.lock() {unsafe {
            printf (b"%s\0".as_ptr() as *const c_char, chunk.as_ptr() as *const c_char);
            // Stdout buffering would sometimes mess with the tests.
            // Particularly when running under the VSCode debugger on Windows, as the buffer size is bumped up then.
            // But also with normal runs sometimes, when examining the end of the log.
            // Explicitly flushing the stdout helps.
            flush_stdout();
        }}
    }
}

#[doc(hidden)]
pub fn short_log_time() -> DelayedFormat<StrftimeItems<'static>> {
    // NB: Given that the debugging logs are targeted at the developers and not the users
    // I think it's better to output the time in UTC here
    // in order for the developers to more easily match the events between the various parts of the peer-to-peer system.
    let time = Utc.timestamp_millis (now_ms() as i64);
    time.format ("%d %H:%M:%S")
}

/// Debug logging.
/// 
/// This logging SHOULD be human-readable but it is not intended for the end users specifically.
/// Rather, it's being used as debugging and testing tool.
/// 
/// (As such it doesn't have to be a text paragraph, the capital letters and end marks are not necessary).
/// 
/// For the user-targeted logging use the `LogState::log` instead.
/// 
/// On Windows the Rust standard output and the standard output of the MM1 C library are not compatible,
/// they will overlap and overwrite each other if used togather.
/// In order to avoid it, all logging MUST be done through this macros and NOT through `println!` or `eprintln!`.
#[macro_export]
macro_rules! log {
    ($($args: tt)+) => {{
        use std::fmt::Write;

        // We can optimize this with a stack-allocated SmallVec from https://github.com/arcnmx/stack-rs,
        // though it doesn't worth the trouble at the moment.
        let mut buf = String::new();
        unwrap! (wite! (&mut buf,
            ($crate::log::short_log_time()) ", "
            (::gstuff::filename (file!())) ':' (line!()) "] "
            $($args)+)
        );
        $crate::log::chunk2log (buf)
    }}
}

pub trait TagParam<'a> {
    fn key (&self) -> String;
    fn val (&self) -> Option<String>;
}

impl<'a> TagParam<'a> for &'a str {
    fn key (&self) -> String {String::from (&self[..])}
    fn val (&self) -> Option<String> {None}
}

impl<'a> TagParam<'a> for String {
    fn key (&self) -> String {self.clone()}
    fn val (&self) -> Option<String> {None}
}

impl<'a> TagParam<'a> for (&'a str, &'a str) {
    fn key (&self) -> String {String::from (self.0)}
    fn val (&self) -> Option<String> {Some (String::from (self.1))}
}

impl<'a> TagParam<'a> for (String, &'a str) {
    fn key (&self) -> String { self.0.clone() }
    fn val (&self) -> Option<String> {Some (String::from (self.1))}
}

impl<'a> TagParam<'a> for (&'a str, i32) {
    fn key (&self) -> String {String::from (self.0)}
    fn val (&self) -> Option<String> {Some (fomat! ((self.1)))}
}

#[derive(Clone, Eq, PartialEq)]
pub struct Tag {
    pub key: String,
    pub val: Option<String>
}

impl Tag {
    /// Returns the tag's value or the empty string if there is no value.
    pub fn val_s (&self) -> &str {
        match self.val {
            Some (ref s) => &s[..],
            None => ""
        }
    }
}

impl fmt::Debug for Tag {
    fn fmt (&self, ft: &mut fmt::Formatter) -> fmt::Result {
        ft.write_str (&self.key) ?;
        if let Some (ref val) = self.val {
            ft.write_str ("=") ?;
            ft.write_str (val) ?;
        }
        Ok(())
    }
}

/// The status entry kept in the dashboard.
#[derive(Clone)]
pub struct Status {
    pub tags: Vec<Tag>,
    pub line: String,
    // Might contain the previous versions of the status.
    pub trail: Vec<Status>
}

#[derive(Clone)]
pub struct LogEntry {
    pub time: u64,
    pub emotion: String,
    pub tags: Vec<Tag>,
    pub line: String,
    /// If the log entry represents a finished `Status` then `trail` might contain the previous versions of that `Status`.
    pub trail: Vec<Status>
}

impl Default for LogEntry {
    fn default() -> Self {
        LogEntry {
            time: now_ms(),
            emotion: Default::default(),
            tags: Default::default(),
            line: Default::default(),
            trail: Default::default(),
        }
    }
}

impl LogEntry {
    pub fn format (&self, buf: &mut String) -> Result<(), fmt::Error> {
        let time = Local.timestamp_millis (self.time as i64);

        wite! (buf,
            if self.emotion.is_empty() {'·'} else {(self.emotion)}
            ' '
            (time.format ("%Y-%m-%d %H:%M:%S %z"))
            ' '
            // TODO: JSON-escape the keys and values when necessary.
            '[' for t in &self.tags {(t.key) if let Some (ref v) = t.val {'=' (v)}} separated {' '} "] "
            (self.line)
            for tr in self.trail.iter().rev() {
                "\n    " (tr.line)
            }
        )
    }
}

/// Tracks the status of an ongoing operation, allowing us to inform the user of the status updates.
/// 
/// Dropping the handle tells us that the operation was "finished" and that we can dump the final status into the log.
pub struct StatusHandle<'a> {
    log: &'a LogState,
    status: Option<Arc<Mutex<Status>>>
}

impl<'a> StatusHandle<'a> {
    /// Creates the status or rewrites it.
    /// 
    /// The `tags` can be changed as well:
    /// with `StatusHandle` the status line is directly identified by the handle and doesn't use the tags to lookup the status line.
    pub fn status<'b> (&mut self, tags: &[&TagParam], line: &str) {
        let mut stack_status = Status {
            tags: tags.iter().map (|t| Tag {key: t.key(), val: t.val()}) .collect(),
            line: line.into(),
            trail: Vec::new()
        };
        if let Some (ref status) = self.status {
            {
                let mut shared_status = unwrap! (status.lock(), "Can't lock the status");
                swap (&mut stack_status, &mut shared_status);
                swap (&mut stack_status.trail, &mut shared_status.trail);  // Move the existing `trail` back to the `shared_status`.
                shared_status.trail.push (stack_status);
            }
            self.log.updated (status);
        } else {
            let status = Arc::new (Mutex::new (stack_status));
            self.status = Some (status.clone());
            self.log.started (status);
        }
    }

    /// Adds new text into the status line.  
    /// Does nothing if the status handle is empty (if the status wasn't created yet).
    pub fn append (&self, suffix: &str) {
        if let Some (ref status) = self.status {
            {
                let mut status = unwrap! (status.lock(), "Can't lock the status");
                status.line.push_str (suffix)
            }
            self.log.updated (status);
        }
    }

    /// Detach the handle from the status, allowing the status to remain in the dashboard when the handle is dropped.
    /// 
    /// The code should later manually finish the status (finding it with `LogState::find_status`).
    pub fn detach (&mut self) -> &mut Self {
        self.status = None;
        self
    }
}

impl<'a> Drop for StatusHandle<'a> {
    fn drop (&mut self) {
        if let Some (ref status) = self.status {
            self.log.finished (status)
        }
    }
}

/// Generates a MM dashboard file path from the MM log file path.
pub fn dashboard_path (log_path: &Path) -> Result<PathBuf, String> {
    let log_path = try_s! (log_path.to_str().ok_or ("Non-unicode log_path?"));
    Ok (format! ("{}.dashboard", log_path) .into())
}

/// The shared log state of a MarketMaker instance.  
/// Carried around by the MarketMaker state, `MmCtx`.  
/// Keeps track of the log file and the status dashboard.
pub struct LogState {
    dashboard: Mutex<Vec<Arc<Mutex<Status>>>>,
    /// Keeps recent log entries in memory in case we need them for debugging.  
    /// Should allow us to examine the log from withing the unit tests, core dumps and live debugging sessions.
    tail: Mutex<VecDeque<LogEntry>>,
    /// Initialized when we need the logging to happen through a certain thread
    /// (this thread becomes a center of gravity for the other registered threads).
    /// In the future we might also use `gravity` to log into a file.
    gravity: Mutex<Option<Arc<Gravity>>>,
    /// Log to stdout if `None`.
    _log_file: Option<Mutex<fs::File>>,
    /// Dashboard is dumped here, allowing us to easily observe it from a command-line or the tests.  
    /// No dumping if `None`.
    dashboard_file: Option<Mutex<fs::File>>
}

impl LogState {
    /// Log into memory, for unit testing.
    pub fn in_memory() -> LogState {
        LogState {
            dashboard: Mutex::new (Vec::new()),
            tail: Mutex::new (VecDeque::with_capacity (64)),
            gravity: Mutex::new (None),
            _log_file: None,
            dashboard_file: None
        }
    }

    /// Initialize according to the MM command-line configuration.
    pub fn mm (conf: &Json) -> LogState {
        let (log_file, dashboard_file) = match conf["log"] {
            Json::Null => (None, None),
            Json::String (ref path) => {
                let log_file = unwrap! (
                    fs::OpenOptions::new().append (true) .create (true) .open (path),
                    "Can't open log file {}", path
                );

                let dashboard_path = unwrap! (dashboard_path (Path::new (&path)));
                let dashboard_file = unwrap! (
                    fs::OpenOptions::new().write (true) .create (true) .open (&dashboard_path),
                    "Can't open dashboard file {:?}", dashboard_path
                );

                (Some (Mutex::new (log_file)), Some (Mutex::new (dashboard_file)))
            },
            ref x => panic! ("The 'log' is not a string: {:?}", x)
        };
        LogState {
            dashboard: Mutex::new (Vec::new()),
            tail: Mutex::new (VecDeque::with_capacity (64)),
            gravity: Mutex::new (None),
            _log_file: log_file,
            dashboard_file
        }
    }

    /// The operation is considered "in progress" while the `StatusHandle` exists.
    /// 
    /// When the `StatusHandle` is dropped the operation is considered "finished" (possibly with a failure)
    /// and the status summary is dumped into the log.
    pub fn status_handle (&self) -> StatusHandle {
        StatusHandle {
            log: self,
            status: None
        }
    }

    fn dump_dashboard (&self, dashboard: MutexGuard<Vec<Arc<Mutex<Status>>>>) {
        if dashboard.len() == 0 {return}
        let df = match self.dashboard_file {Some (ref df) => df, None => return};
        let mut buf = String::with_capacity (dashboard.len() * 256);
        let mut locked = Vec::new();
        for status in dashboard.iter() {
            if let Ok (status) = status.try_lock() {
                let _ = writeln! (&mut buf, "{:?} {}", status.tags, status.line);
            } else {
                locked.push (status.clone())
            }
        }
        drop (dashboard);  // Unlock the dashboard.
        for status in locked {
            if let Ok (status) = status.lock() {
                let _ = writeln! (&mut buf, "{:?} {}", status.tags, status.line);
            } else {
                log! ("dump_dashboard] Can't lock a status")
            }
        }

        let mut df = match df.lock() {Ok (lock) => lock, Err (err) => {log! ({"dump_dashboard] Can't lock the file: {}", err}); return}};
        if let Err (err) = df.seek (SeekFrom::Start (0)) {log! ({"dump_dashboard] Can't seek the file: {}", err}); return}
        if let Err (err) = df.write_all (buf.as_bytes()) {log! ({"dump_dashboard] Can't write the file: {}", err}); return}
        if let Err (err) = df.set_len (buf.len() as u64) {log! ({"dump_dashboard] Can't truncate the file: {}", err}); return}
    }

    /// Invoked when the `StatusHandle` gets the first status.
    fn started (&self, status: Arc<Mutex<Status>>) {
        match self.dashboard.lock() {
            Ok (mut dashboard) => {
                dashboard.push (status);
                self.dump_dashboard (dashboard)
            },
            Err (err) => log! ({"log] Can't lock the dashboard: {}", err})
        }
    }

    /// Invoked when the `StatusHandle` updates the status.
    fn updated (&self, _status: &Arc<Mutex<Status>>) {
        match self.dashboard.lock() {
            Ok (dashboard) => self.dump_dashboard (dashboard),
            Err (err) => log! ({"log] Can't lock the dashboard: {}", err})
        }
    }

    /// Invoked when the `StatusHandle` is dropped, marking the status as finished.
    fn finished (&self, status: &Arc<Mutex<Status>>) {
        match self.dashboard.lock() {
            Ok (mut dashboard) => {
                if let Some (idx) = dashboard.iter().position (|e| Arc::ptr_eq (e, status)) {
                    dashboard.swap_remove (idx);
                    self.dump_dashboard (dashboard)
                } else {
                    log! ("log] Warning, a finished StatusHandle was missing from the dashboard.");
                }
            },
            Err (err) => log! ({"log] Can't lock the dashboard: {}", err})
        }
        let mut status = match status.lock() {
            Ok (status) => status,
            Err (err) => {
                log! ({"log] Can't lock the status: {}", err});
                return
            }
        };
        let chunk = match self.tail.lock() {
            Ok (mut tail) => {
                if tail.len() == tail.capacity() {let _ = tail.pop_front();}
                let mut log = LogEntry::default();
                swap (&mut log.tags, &mut status.tags);
                swap (&mut log.line, &mut status.line);
                swap (&mut log.trail, &mut status.trail);
                let mut chunk = String::with_capacity (256);
                if let Err (err) = log.format (&mut chunk) {
                    log! ({"log] Error formatting log entry: {}", err});
                }
                tail.push_back (log);
                Some (chunk)
            },
            Err (err) => {
                log! ({"log] Can't lock the tail: {}", err});
                None
            }
        };
        if let Some (chunk) = chunk {self.chunk2log (chunk)}
    }

    /// Read-only access to the status dashboard.
    pub fn with_dashboard (&self, cb: &mut dyn FnMut (&[Arc<Mutex<Status>>])) {
        let dashboard = unwrap! (self.dashboard.lock(), "Can't lock the dashboard");
        cb (&dashboard[..])
    }

    pub fn with_tail (&self, cb: &mut dyn FnMut (&VecDeque<LogEntry>)) {
        let tail = unwrap! (self.tail.lock(), "Can't lock the tail");
        cb (&*tail)
    }

    pub fn with_gravity_tail (&self, cb: &mut dyn FnMut (&VecDeque<String>)) {
        let gravity = unwrap! (self.gravity.lock(), "Can't lock the gravity");
        if let Some (ref gravity) = *gravity {
            gravity.flush();
            let tail = unwrap! (gravity.tail.lock(), "Can't lock the tail");
            cb (&*tail)
    }   }

    /// Creates the status or rewrites it if the tags match.
    pub fn status<'b> (&self, tags: &[&TagParam], line: &str) -> StatusHandle {
        let mut status = self.claim_status (tags) .unwrap_or (self.status_handle());
        status.status (tags, line);
        status
    }

    /// Search dashboard for status matching the tags.
    /// 
    /// Note that returned status handle represent an ownership of the status and on the `drop` will mark the status as finished.
    pub fn claim_status (&self, tags: &[&TagParam]) -> Option<StatusHandle> {
        let mut found = Vec::new();
        let mut locked = Vec::new();
        let tags: Vec<Tag> = tags.iter().map (|t| Tag {key: t.key(), val: t.val()}) .collect();
        let dashboard = unwrap! (self.dashboard.lock(), "Can't lock the dashboard");
        for status_arc in &*dashboard {
            if let Ok (ref status) = status_arc.try_lock() {
                if status.tags == tags {found.push (StatusHandle {
                    log: self,
                    status: Some (status_arc.clone())
                })}
            } else {
                locked.push (status_arc.clone())
            }
        }
        drop (dashboard);  // Unlock the dashboard before lock-waiting on statuses, avoiding a chance of deadlock.
        for status_arc in locked {
            let matches = unwrap! (status_arc.lock(), "Can't lock a status") .tags == tags;
            if matches {found.push (StatusHandle {
                log: self,
                status: Some (status_arc)
            })}
        }
        if found.len() > 1 {log! ("log] Dashboard tags not unique!")}
        found.pop()
    }

    /// Returns `true` if there are recent log entries exactly matching the tags.
    pub fn tail_any (&self, tags: &[&TagParam]) -> bool {
        let tags: Vec<Tag> = tags.iter().map (|t| Tag {key: t.key(), val: t.val()}) .collect();
        let tail = match self.tail.lock() {Ok (l) => l, _ => return false};
        for en in tail.iter() {
            if en.tags == tags {
                return true
            }
        }
        return false
    }

    /// Creates a new human-readable log entry.
    /// 
    /// This is a bit different from the `println!` logging
    /// (https://www.reddit.com/r/rust/comments/9hpk65/which_tools_are_you_using_to_debug_rust_projects/e6dkciz/)
    /// as the information here is intended for the end users
    /// (and to be shared through the GUI),
    /// explaining what's going on with MM.
    /// 
    /// Since the focus here is on human-readability, the log entry SHOULD be treated
    /// as a text paragraph, namely starting with a capital letter and ending with an end mark.
    /// 
    /// * `emotion` - We might use a unicode smiley here
    ///   (https://unicode.org/emoji/charts/full-emoji-list.html)
    ///   to emotionally color the event (the good, the bad and the ugly)
    ///   or enrich it with infographics.
    /// * `tags` - Parsable part of the log,
    ///   representing subsystems and sharing concrete values.
    ///   GUI might use it to get some useful information from the log.
    /// * `line` - The human-readable description of the event,
    ///   we have no intention to make it parsable.
    pub fn log (&self, emotion: &str, tags: &[&TagParam], line: &str) {
        let entry = LogEntry {
            time: now_ms(),
            emotion: emotion.into(),
            tags: tags.iter().map (|t| Tag {key: t.key(), val: t.val()}) .collect(),
            line: line.into(),
            trail: Vec::new()
        };

        let mut chunk = String::with_capacity (256);
        if let Err (err) = entry.format (&mut chunk) {
            log! ({"log] Error formatting log entry: {}", err});
            return
        }

        match self.tail.lock() {
            Ok (mut tail) => {
                if tail.len() == tail.capacity() {let _ = tail.pop_front();}
                tail.push_back (entry)
            },
            Err (err) => log! ({"log] Can't lock the tail: {}", err})
        }

        self.chunk2log (chunk)
    }

    fn chunk2log (&self, chunk: String) {
        self::chunk2log (chunk)
        /*
        match self.log_file {
            Some (ref f) => match f.lock() {
                Ok (mut f) => {
                    if let Err (err) = f.write (chunk.as_bytes()) {
                        eprintln! ("log] Can't write to the log: {}", err);
                        println! ("{}", chunk);
                    }
                },
                Err (err) => {
                    eprintln! ("log] Can't lock the log: {}", err);
                    println! ("{}", chunk)
                }
            },
            None => println! ("{}", chunk)
        }
        */
    }

    /// Writes into the *raw* portion of the log, the one not shared with the UI.
    pub fn rawln (&self, mut line: String) {
        line.push ('\n');
        self.chunk2log (line);
    }

    /// Binds the logger to the current thread,
    /// creating a gravity anomaly that would pull log entries made on other threads into this thread.
    /// Useful for unit tests, since they can only capture the output made from the initial test thread
    /// (https://github.com/rust-lang/rust/issues/12309,
    ///  https://github.com/rust-lang/rust/issues/50297#issuecomment-388988381).
    pub fn thread_gravity_on (&self) -> Result<(), String> {
        let mut gravity = try_s! (self.gravity.lock());
        if let Some (ref gravity) = *gravity {
            if gravity.target_thread_id == thread::current().id() {
                Ok(())
            } else {
                ERR! ("Gravity already enabled and for a different thread")
            }
        } else {
            *gravity = Some (Arc::new (Gravity {
                target_thread_id: thread::current().id(),
                landing: SegQueue::new(),
                tail: Mutex::new (VecDeque::with_capacity (64))
            }));
            Ok(())
        }
    }

    /// Start intercepting the `log!` invocations happening on the current thread.
    pub fn register_my_thread (&self) -> Result<(), String> {
        let gravity = try_s! (self.gravity.lock());
        if let Some (ref gravity) = *gravity {
            try_s! (GRAVITY.try_with (|thread_local_gravity| {
                thread_local_gravity.replace (Some (gravity.clone()))
            }));
        } else {
            // If no gravity thread is registered then `register_my_thread` is currently a no-op.
            // In the future we might implement a version of `Gravity` that pulls log entries into a file
            // (but we might want to get rid of C logging first).
        }
        Ok(())
    }
}

impl Drop for LogState {
    fn drop (&mut self) {
        // Make sure to log the chunks received from the satellite threads.
        // NB: The `drop` might happen in a thread that is not the center of gravity,
        //     resulting in log chunks escaping the unit test capture.
        //     One way to fight this might be adding a flushing RAII struct into a unit test.
        // NB: The `drop` will not be happening if some of the satellite threads still hold to the context.
        let mut gravity_arc = None;  // Variable is used in order not to hold two locks.
        if let Ok (gravity) = self.gravity.lock() {
            if let Some (ref gravity) = *gravity {
                gravity_arc = Some (gravity.clone())
        }   }
        if let Some (gravity) = gravity_arc {
            gravity.flush()
        }

        let dashboard_copy = {
            let dashboard = match self.dashboard.lock() {
                Ok (d) => d,
                Err (err) => {log! ({"LogState::drop] Can't lock `dashboard`: {}", err}); return}
            };
            dashboard.clone()
        };
        if dashboard_copy.len() > 0 {
            log! ("--- LogState] Bye! Remaining status entries. ---");
            for status in &*dashboard_copy {self.finished (status)}
        } else {
            log! ("LogState] Bye!");
        }
    }
}

#[doc(hidden)]
pub mod tests {
    use super::LogState;

    pub fn test_status() {
        let log = LogState::in_memory();

        log.with_dashboard (&mut |dashboard| assert_eq! (dashboard.len(), 0));

        let mut handle = log.status_handle();
        for n in 1..=3 {
            handle.status (&[&"tag1", &"tag2"], &format! ("line {}", n));

            log.with_dashboard (&mut |dashboard| {
                assert_eq! (dashboard.len(), 1);
                let status = unwrap! (dashboard[0].lock());
                assert! (status.tags.iter().any (|tag| tag.key == "tag1"));
                assert! (status.tags.iter().any (|tag| tag.key == "tag2"));
                assert_eq! (status.tags.len(), 2);
                assert_eq! (status.line, format! ("line {}", n));
            });
        }
        drop (handle);

        log.with_dashboard (&mut |dashboard| assert_eq! (dashboard.len(), 0));  // The status was dumped into the log.
        log.with_tail (&mut |tail| {
            assert_eq! (tail.len(), 1);
            assert_eq! (tail[0].line, "line 3");
            assert! (tail[0].trail.iter().any (|status| status.line == "line 2"));
            assert! (tail[0].trail.iter().any (|status| status.line == "line 1"));

            assert! (tail[0].tags.iter().any (|tag| tag.key == "tag1"));
            assert! (tail[0].tags.iter().any (|tag| tag.key == "tag2"));
            assert_eq! (tail[0].tags.len(), 2);
        })
    }
}

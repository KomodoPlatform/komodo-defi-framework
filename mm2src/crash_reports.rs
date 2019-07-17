#![allow(unused_imports)]
use common::{self, set_panic_hook, stack_trace, stack_trace_frame};
use libc::c_int;
use std::env;
use std::io::stderr;
use std::io::Write;
use std::path::Path;
use std::process::abort;
use std::sync::Once;

#[cfg(windows)]
#[allow(dead_code)]
fn exception_name (exception_code: u32) -> &'static str {
    use winapi::um::minwinbase::{EXCEPTION_ACCESS_VIOLATION, EXCEPTION_ILLEGAL_INSTRUCTION};
    match exception_code {
        EXCEPTION_ACCESS_VIOLATION => "Access Violation",
        EXCEPTION_ILLEGAL_INSTRUCTION => "Illegal Instruction",
        0xE06D7363 => "VC++ Exception",  // https://blogs.msdn.microsoft.com/oldnewthing/20100730-00/?p=13273
        _ => ""
    }
}

/// Performs a crash report and aborts.
#[cfg(windows)]
#[no_mangle]
pub extern fn rust_seh_handler (exception_code: u32) {
    eprintln! ("SEH caught! ExceptionCode: {} ({}).", exception_code, exception_name (exception_code));
    stack_trace (&mut stack_trace_frame, &mut |trace| {
        let stderr = stderr();
        let mut stderr = stderr.lock();
        let _ = stderr.write_all (trace.as_bytes());
        let _ = stderr.flush();
    });
    abort()
}

#[cfg(test)]
#[inline(never)]
#[allow(dead_code)]
fn access_violation() {
    let ptr: *mut i32 = 0 as *mut i32;
    unsafe {*ptr = 123};
}

#[cfg(test)]
#[inline(never)]
#[allow(dead_code)]
extern fn call_access_violation() {access_violation()}

#[cfg(unix)]
#[allow(dead_code)]
extern fn signal_handler (sig: c_int) {
    {
        // NB: Manually writing to `stderr` is more reliable than using `eprintln!`, especially around `dup2`.
        let stderr = stderr();
        let mut stderr = stderr.lock();
        let _ = writeln! (&mut stderr, "Signal caught! sig {}", sig);
        let _ = stderr.flush();
    }

    stack_trace (&mut stack_trace_frame, &mut |trace| {
        let stderr = stderr();
        let mut stderr = stderr.lock();
        let _ = stderr.write_all (trace.as_bytes());
        // Explicitly flush the output stream. Under at least the Docker/Linux the tail is sometimes lost on `abort`.
        let _ = stderr.flush();
    });
    abort();
}

#[cfg(unix)]
fn init_signal_handling() {
    // TODO: Implement without `nix`.
/*

    use nix::sys::signal::{sigaction, SaFlags, Signal, SigAction, SigHandler, SigSet};

    lazy_static! {
        static ref ACTION: SigAction = SigAction::new (
            SigHandler::Handler (signal_handler),
            SaFlags::empty(),
            SigSet::empty()
        );
    }

    for signal in [Signal::SIGILL, Signal::SIGFPE, Signal::SIGSEGV, Signal::SIGBUS, Signal::SIGSYS].iter() {
        unsafe {unwrap! (sigaction (*signal, &*ACTION), "!sigaction");}
    }
*/
}

#[allow(dead_code)]
#[cfg(not(unix))]
fn init_signal_handling() {}

/// Check that access violation stack traces are being reported to stderr under macOS and Linux.
#[test]
fn test_crash_handling() {
/* TODO: Implement without `cmd!` (without `duct`).
    let executable = unwrap! (env::args().next());
    let executable = unwrap! (Path::new (&executable) .canonicalize());

    if env::var ("_MM2_TEST_CRASH_HANDLING_IS_CHILD") != Ok ("1".into()) {
        log! ("test_crash_handling] Spawning a child...");
        let output = unwrap! (cmd! (&executable, "test_crash_handling", "--nocapture")
            .env ("_MM2_TEST_CRASH_HANDLING_IS_CHILD", "1")
            .dir (unwrap! (executable.parent()))  // Might help finding libcurl.dll and pthreadVC2.dll.
            .stdout_capture().stderr_capture().unchecked().run());
        let stderr = String::from_utf8_lossy (&output.stderr);
        log! ({"Obtained stderr is: ---\n{}", stderr});

        if cfg!(windows) {
          assert! (stderr.contains ("SEH caught!"));
        } else {
          assert! (stderr.contains ("Signal caught!"));
        }

        assert! (stderr.contains ("] mm2::mm2::crash_reports::access_violation"));
        assert! (stderr.contains ("] mm2::mm2::crash_reports::call_access_violation"));
    } else {
        log! ("test_crash_handling] Hi from the child.");
        init_crash_reports();
        call_access_violation();
    }
*/
}

/// Setup the crash handlers.
#[allow(dead_code)]
pub fn init_crash_reports() {
    static ONCE: Once = Once::new();
    ONCE.call_once (|| {
        common::wio::init();

        set_panic_hook();

        // Try to invoke the `rust_seh_handler` whenever the C code crashes.
        if cfg! (windows) {
            extern "C" {fn init_veh();}
            unsafe {init_veh();}
        } else if cfg! (unix) {
            init_signal_handling()
        }

        // Log Rust panics.
        env::set_var ("RUST_BACKTRACE", "1")
        // ^^ NB: In the future this might also affect the normal errors, cf. https://github.com/rust-lang/rfcs/blob/master/text/2504-fix-error.md.
    })
}

// Make sure Rust panics still work in the presence of the VEH handler.
#[test]
#[should_panic]
fn test_panic() {
    init_crash_reports();
    panic! ("NP");
}

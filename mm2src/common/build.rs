// The script here will translate some of the C headers necessary for the gradual Rust port into the corresponding Rust files.
// Going to take the *whitelisting* approach, converting just the necessary definitions, in order to keep the builds fast.

// The script is experimentally formatted with `rustfmt`. Probably not going to use `rustfmt` for the rest of the project though.

// Bindgen requirements: https://rust-lang-nursery.github.io/rust-bindgen/requirements.html
//              Windows: https://github.com/rust-lang-nursery/rustup.rs/issues/1003#issuecomment-289825927
// On build.rs: https://doc.rust-lang.org/cargo/reference/build-scripts.html

#![allow(uncommon_codepoints)]
#![feature(non_ascii_idents)]
#![cfg_attr(not(feature = "native"), allow(dead_code))]

#[macro_use] extern crate fomat_macros;
#[macro_use] extern crate unwrap;

use chrono::DateTime;
use gstuff::{last_modified_sec, slurp};
use regex::Regex;
use std::env::{self};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::str::{from_utf8, from_utf8_unchecked};
use std::thread;

/// Ongoing (RLS) builds might interfere with a precise time comparison.
const SLIDE: f64 = 60.;

/// This function ensures that we have the “MM_VERSION” and “MM_DATETIME” variables during the build.
///
/// The build script will usually help us by putting the MarketMaker version into the “MM_VERSION” file
/// and the corresponding ISO 8601 time into the “MM_DATETIME” file
/// (environment variable isn't as useful because we can't `rerun-if-changed` on it).
///
/// For the nightly builds the version contains the short commit hash.
///
/// We're also trying to get the hash and the time from Git.
///
/// Git information isn't always available during the build (for instance, when a build server is used,
/// we might skip synchronizing the Git repository there),
/// but if it is, then we're going to check if the “MM_DATETIME” and the Git data match.
fn mm_version() -> String {
    // Try to load the variable from the file.
    let mm_versionᵖ = root().join("MM_VERSION");
    let mut buf;
    let version = if let Ok(mut mm_versionᶠ) = fs::File::open(&mm_versionᵖ) {
        buf = String::new();
        unwrap!(mm_versionᶠ.read_to_string(&mut buf), "Can't read from MM_VERSION");
        buf.trim().to_string()
    } else {
        // If the “MM_VERSION” file is absent then we should create it
        // in order for the Cargo dependency management to see it,
        // because Cargo will keep rebuilding the `common` crate otherwise.
        //
        // We should probably fetch the actual git version here,
        // with something like `git log '--pretty=format:%h' -n 1` for the nightlies,
        // and a release tag when building from some kind of a stable branch,
        // though we should keep the ability for the tooling to provide the “MM_VERSION”
        // externally, because moving the entire ".git" around is not always practical.

        let mut version = "UNKNOWN".to_string();
        let mut command = Command::new("git");
        command.arg("log").arg("--pretty=format:%h").arg("-n1");
        if let Ok(go) = command.output() {
            if go.status.success() {
                version = unwrap!(from_utf8(&go.stdout)).trim().to_string();
                if !unwrap!(Regex::new(r"^\w+$")).is_match(&version) {
                    panic!("{}", version)
                }
            }
        }

        if let Ok(mut mm_versionᶠ) = fs::File::create(&mm_versionᵖ) {
            unwrap!(mm_versionᶠ.write_all(version.as_bytes()));
        }
        version
    };
    println!("cargo:rustc-env=MM_VERSION={}", version);

    let mut dt_git = None;
    let mut command = Command::new("git");
    command.arg("log").arg("--pretty=format:%cI").arg("-n1"); // ISO 8601
    if let Ok(go) = command.output() {
        if go.status.success() {
            let got = unwrap!(from_utf8(&go.stdout)).trim();
            let _dt_check = unwrap!(DateTime::parse_from_rfc3339(got));
            dt_git = Some(got.to_string());
        }
    }

    let mm_datetimeᵖ = root().join("MM_DATETIME");
    let dt_file = unwrap!(String::from_utf8(slurp(&mm_datetimeᵖ)));
    let mut dt_file = dt_file.trim().to_string();
    if let Some(ref dt_git) = dt_git {
        if dt_git[..] != dt_file[..] {
            // Create or update the “MM_DATETIME” file in order to appease the Cargo dependency management.
            let mut mm_datetimeᶠ = unwrap!(fs::File::create(&mm_datetimeᵖ));
            unwrap!(mm_datetimeᶠ.write_all(dt_git.as_bytes()));
            dt_file = dt_git.clone();
        }
    }

    println!("cargo:rustc-env=MM_DATETIME={}", dt_file);

    version
}

/// Formats a vector of command-line arguments into a printable string, for the build log.
fn show_args<'a, I: IntoIterator<Item = &'a String>>(args: I) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    for arg in args {
        if arg.contains(' ') {
            let _ = write!(&mut buf, " \"{}\"", arg);
        } else {
            buf.push(' ');
            buf.push_str(arg)
        }
    }
    buf
}

fn forward(stdout: ChildStdout) {
    unwrap!(thread::Builder::new().name("forward".into()).spawn(move || {
        let mut buf = Vec::new();
        for ch in stdout.bytes() {
            let ch = match ch {
                Ok(k) => k,
                Err(_) => break,
            };
            if ch == b'\n' {
                eprintln!("{}", unsafe { from_utf8_unchecked(&buf) });
            } else {
                buf.push(ch)
            }
        }
        if !buf.is_empty() {
            eprintln!("{}", unsafe { from_utf8_unchecked(&buf) });
        }
    }));
}

/// Like the `duct` `cmd!` but also prints the command into the standard error stream.
macro_rules! ecmd {
    ( $program:expr ) => {{
        eprintln!("$ {}", $program);
        let mut command = Command::new ($program);
        command.stdout (Stdio::piped());  // Printed to `stderr` in `run!`
        command.stderr (Stdio::inherit());  // `stderr` is directly visible with "cargo build -vv".
        command
    }};
    ( @s $args: expr, $arg:expr ) => {$args.push (String::from ($arg));};
    ( @i $args: expr, $iterable:expr ) => {for v in $iterable {ecmd! (@s $args, v)}};
    ( @a $args: expr, i $arg:expr ) => {ecmd! (@i $args, $arg);};
    ( @a $args: expr, i $arg:expr, $( $tail:tt )* ) => {ecmd! (@i $args, $arg); ecmd! (@a $args, $($tail)*);};
    ( @a $args: expr, $arg:expr ) => {ecmd! (@s $args, $arg);};
    ( @a $args: expr, $arg:expr, $( $tail:tt )* ) => {ecmd! (@s $args, $arg); ecmd! (@a $args, $($tail)*);};
    ( $program:expr, $( $args:tt )* ) => {{
        let mut args: Vec<String> = Vec::new();
        ecmd! (@a &mut args, $($args)*);
        eprintln!("$ {}{}", $program, show_args (&args));
        let mut command = Command::new ($program);
        command.stdout (Stdio::inherit()) .stderr (Stdio::inherit());
        for arg in args {command.arg (arg);}
        command
    }};
}
macro_rules! run {
    ( $command: expr ) => {
        let mut pc = unwrap!($command.spawn());
        if let Some(stdout) = pc.stdout.take() {
            forward(stdout)
        }
        let status = unwrap!(pc.wait());
        if !status.success() {
            panic!("Command returned an error status: {}", status)
        }
    };
}

/// See if we have the required libraries.
#[cfg(windows)]
fn windows_requirements() {
    use std::ffi::OsString;
    use std::mem::MaybeUninit;
    use std::os::windows::ffi::OsStringExt;
    // https://msdn.microsoft.com/en-us/library/windows/desktop/ms724373(v=vs.85).aspx
    use winapi::um::sysinfoapi::GetSystemDirectoryW;

    let system = {
        let mut buf: [u16; 1024] = unsafe { MaybeUninit::uninit().assume_init() };
        let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), (buf.len() - 1) as u32) };
        if len <= 0 {
            panic!("!GetSystemDirectoryW")
        }
        let len = len as usize;
        let system = OsString::from_wide(&buf[0..len]);
        Path::new(&system).to_path_buf()
    };
    eprintln!("windows_requirements] System directory is {:?}.", system);

    // `msvcr100.dll` is required by `ftp://sourceware.org/pub/pthreads-win32/prebuilt-dll-2-9-1-release/dll/x64/pthreadVC2.dll`
    let msvcr100 = system.join("msvcr100.dll");
    if !msvcr100.exists() {
        panic!(
            "msvcr100.dll is missing. \
            You can install it from https://www.microsoft.com/en-us/download/details.aspx?id=14632."
        );
    }

    // I don't exactly know what DLLs this download installs. Probably "msvcp140...". Might prove useful later.
    //You can install it from https://aka.ms/vs/15/release/vc_redist.x64.exe,
    //see https://support.microsoft.com/en-us/help/2977003/the-latest-supported-visual-c-downloads
}

#[cfg(not(windows))]
fn windows_requirements() {}

/// SuperNET's root.
fn root() -> PathBuf {
    let common = Path::new(env!("CARGO_MANIFEST_DIR"));
    let super_net = common.join("../..");
    let super_net = match super_net.canonicalize() {
        Ok(p) => p,
        Err(err) => panic!("Can't canonicalize {:?}: {}", super_net, err),
    };
    // On Windows we're getting these "\\?\" paths from canonicalize but they aren't any good for CMake.
    if cfg!(windows) {
        let s = path2s(super_net);
        let stripped = match s.strip_prefix(r"\\?\") {
            Some(stripped) => stripped,
            None => &s,
        };
        Path::new(stripped).into()
    } else {
        super_net
    }
}

/// Absolute path taken from SuperNET's root + `path`.  
fn rabs(rrel: &str) -> PathBuf { root().join(rrel) }

fn path2s(path: PathBuf) -> String { unwrap!(path.to_str(), "Non-stringy path {:?}", path).into() }

/// Loads the `path`, runs `update` on it and saves back the result if it differs.
fn _in_place(path: &dyn AsRef<Path>, update: &mut dyn FnMut(Vec<u8>) -> Vec<u8>) {
    let path: &Path = path.as_ref();
    if !path.is_file() {
        return;
    }
    let dir = unwrap!(path.parent());
    let name = unwrap!(unwrap!(path.file_name()).to_str());
    let bulk = slurp(&path);
    if bulk.is_empty() {
        return;
    }
    let updated = update(bulk.clone());
    if bulk != updated {
        let tmp = dir.join(fomat! ((name) ".tmp"));
        {
            let mut file = unwrap!(fs::File::create(&tmp));
            unwrap!(file.write_all(&updated));
        }
        unwrap!(fs::rename(tmp, path))
    }
}

/// Build helper C code.
///
/// I think "git clone ... && cargo build" should be enough to start hacking on the Rust code.
///
/// For now we're building the Structured Exception Handling code here,
/// but in the future we might subsume the rest of the C build under build.rs.
fn build_c_code() {
    // Link in the Windows-specific crash handling code.

    if cfg!(windows) {
        let lm_seh = unwrap!(last_modified_sec(&"seh.c"), "Can't stat seh.c");
        let out_dir = unwrap!(env::var("OUT_DIR"), "!OUT_DIR");
        let lib_path = Path::new(&out_dir).join("libseh.a");
        let lm_lib = last_modified_sec(&lib_path).unwrap_or(0.);
        if lm_seh >= lm_lib - SLIDE {
            cc::Build::new().file("seh.c").warnings(true).compile("seh");
        }
        println!("cargo:rustc-link-lib=static=seh");
        println!("cargo:rustc-link-search=native={}", out_dir);
    }

    if cfg!(windows) {
        // https://sourceware.org/pthreads-win32/
        // ftp://sourceware.org/pub/pthreads-win32/prebuilt-dll-2-9-1-release/

        let pthread_dll = root().join("x64/pthreadVC2.dll");
        if !pthread_dll.is_file() {
            run!(ecmd!("cmd", "/c", "marketmaker_build_depends.cmd").current_dir(&root()));
            assert!(pthread_dll.is_file(), "Missing {:?}", pthread_dll);
        }

        println!("cargo:rustc-link-lib=pthreadVC2");
        unwrap!(
            fs::copy(&pthread_dll, root().join("target/debug/pthreadVC2.dll")),
            "Can't copy {:?}",
            pthread_dll
        );
    } else {
        println!("cargo:rustc-link-lib=crypto");
    }
}

fn main() {
    // NB: `rerun-if-changed` will ALWAYS invoke the build.rs if the target does not exists.
    // cf. https://github.com/rust-lang/cargo/issues/4514#issuecomment-330976605
    //     https://github.com/rust-lang/cargo/issues/4213#issuecomment-310697337
    // `RUST_LOG=cargo::core::compiler::fingerprint cargo build` shows the fingerprit files used.

    println!("cargo:rerun-if-changed={}", path2s(rabs("MM_VERSION")));
    println!("cargo:rerun-if-changed={}", path2s(rabs("MM_DATETIME")));
    if cfg!(not(feature = "native")) {
        return;
    }
    mm_version();
    windows_requirements();
    build_c_code();
}

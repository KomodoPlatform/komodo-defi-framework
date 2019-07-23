// The script here will translate some of the C headers necessary for the gradual Rust port into the corresponding Rust files.
// Going to take the *whitelisting* approach, converting just the necessary definitions, in order to keep the builds fast.

// The script is experimentally formatted with `rustfmt`. Probably not going to use `rustfmt` for the rest of the project though.

// Bindgen requirements: https://rust-lang.github.io/rust-bindgen/requirements.html
//              Windows: https://github.com/rust-lang-nursery/rustup.rs/issues/1003#issuecomment-289825927
// On build.rs: https://doc.rust-lang.org/cargo/reference/build-scripts.html

#![feature(non_ascii_idents)]
#![cfg_attr(not(feature = "native"), allow(dead_code))]

#[macro_use]
extern crate fomat_macros;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate unwrap;

use bzip2::read::BzDecoder;
use futures::{Future, Stream};
use futures_cpupool::CpuPool;
use glob::{glob, Paths, PatternError};
use gstuff::{last_modified_sec, now_float, slurp};
use hyper_rustls::HttpsConnector;
use libflate::gzip::Decoder;
use std::env::{self, var};
use std::fmt::{self, Write as FmtWrite};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::str::from_utf8_unchecked;
use std::sync::Arc;
use std::thread;
use tar::Archive;

/// Ongoing (RLS) builds might interfere with a precise time comparison.
const SLIDE: f64 = 60.;

/// Quote spaces, in case the project is on a path with them.
fn escape_some (path: &str) -> String {
    // AG: I've tried using the shell_escape crate for this,
    // but it adds the single quotes, breaking the variable and argument assignments on Windows.

    let mut esc = String::with_capacity (path.len());
    for ch in path.chars() {
        if ch == ' ' {esc.push ('\\')}
        esc.push (ch)
    }
    esc
}

/// Returns `true` if the `target` is not as fresh as the `prerequisites`.
fn make(target: &AsRef<Path>, prerequisites: &[PathBuf]) -> bool {
    let target_lm = last_modified_sec(target).expect("!last_modified") as u64;
    if target_lm == 0 {
        return true;
    }
    let mut prerequisites_lm = 0;
    for path in prerequisites {
        prerequisites_lm = std::cmp::max(
            prerequisites_lm,
            last_modified_sec(&path).expect("!last_modified") as u64,
        )
    }
    target_lm < prerequisites_lm
}

fn bindgen<
    'a,
    TP: AsRef<Path>,
    FI: Iterator<Item = &'a &'a str>,
    TI: Iterator<Item = &'a &'a str>,
    DI: Iterator<Item = &'a &'a str>,
>(
    from: Vec<String>,
    to: TP,
    functions: FI,
    types: TI,
    defines: DI,
) {
    if cfg!(not(feature = "native")) {
        return;
    }

    // We'd like to regenerate the bindings whenever the build.rs changes, in case we changed bindgen configuration here.
    let lm_build_rs = unwrap!(last_modified_sec(&"build.rs"), "Can't stat build.rs");

    let to = to.as_ref();

    let mut lm_from = 0f64;
    for header_path in &from {
        lm_from = match last_modified_sec(&header_path) {
            Ok(sec) => lm_from.max(sec),
            Err(err) => panic!("Can't stat the header {:?}: {}", from, err),
        };
    }
    let lm_to = last_modified_sec(&to).unwrap_or(0.);
    if lm_from >= lm_to - SLIDE || lm_build_rs >= lm_to - SLIDE {
        let bindings = {
            // https://docs.rs/bindgen/0.37.*/bindgen/struct.Builder.html
            let mut builder = bindgen::builder();
            for header_path in from {
                builder = builder.header(header_path)
            }
            builder = builder.ctypes_prefix("::libc");
            builder = builder.whitelist_recursively(true);
            builder = builder.layout_tests(false);
            builder = builder.derive_default(true);
            // Currently works for functions but not for variables such as `extern uint32_t DOCKERFLAG`.
            builder = builder.generate_comments(true);
            if cfg!(windows) {
                // Normally we should be checking for `_WIN32`, but `nn_config.h` checks for `WIN32`.
                // (Note that it's okay to have WIN32 defined for 64-bit builds,
                // cf https://github.com/rust-lang-nursery/rust-bindgen/issues/1062#issuecomment-334804738).
                builder = builder.clang_arg("-D WIN32");
            }
            for name in functions {
                builder = builder.whitelist_function(name)
            }
            for name in types {
                builder = builder.whitelist_type(name)
            }
            // Looks like C defines should be whitelisted both on the function and the variable levels.
            for name in defines {
                builder = builder.whitelist_function(name);
                builder = builder.whitelist_var(name)
            }
            let target = Target::load();
            if let Target::iOS(ref targetᴱ) = target {
                if targetᴱ == "aarch64-apple-ios" {
                    // https://github.com/rust-lang/rust-bindgen/issues/1211
                    builder = builder.clang_arg("--target=arm64-apple-ios");
                }
                let cops = unwrap!(target.ios_clang_ops());
                builder = builder.clang_arg(fomat!("--sysroot="(cops.sysroot)));
                builder = builder.clang_arg("-arch").clang_arg(cops.arch);
            }
            match builder.generate() {
                Ok(bindings) => bindings,
                Err(()) => panic!("Error generating the bindings for {:?}", to),
            }
        };

        if let Err(err) = bindings.write_to_file(to) {
            panic!("Error writing to {:?}: {}", to, err)
        }
    }
}

fn generate_bindings() {
    let target = Target::load();
    if target.is_android_cross() {
        if !Path::new("/usr/lib/llvm-3.9/lib/libclang.so").exists() {
            // clang is missing from japaric/armv7-linux-androideabi by default.
            // cf. https://github.com/rust-embedded/cross/issues/174
            // We should explain installing and committing clang into japaric/armv7-linux-androideabi when it does.
            panic!(
                "libclang-3.9-dev needs to be installed in order for the cross-compilation to work"
            );
        }
    }

    let c_headers = out_dir().join("c_headers");
    let _ = fs::create_dir(&c_headers);
    assert!(c_headers.is_dir());

    bindgen(
        vec![
            "../../iguana/exchanges/LP_include.h".into(),
        ],
        c_headers.join("LP_include.rs"),
        [
            // functions
            "OS_ensure_directory"
        ]
        .iter(),
        // types
        [
        ]
        .iter(),
        [
            // defines
            "G",
            "GLOBAL_DBDIR",
        ]
        .iter(),
    );
}

/// The build script will usually help us by putting the MarketMaker version into the "MM_VERSION" file
/// (environment variable isn't as useful because we can't `rerun-if-changed` on it).
/// We're using the "UNKNOWN" version designator if the file isn't there or is empty.
/// For the nightly builds the version contains the short commit hash.
/// This function ensures that we have the "MM_VERSION" variable during the build.
fn mm_version() -> String {
    // Try to load the variable from the file.
    let mm_versionᵖ = root().join("MM_VERSION");
    let mut buf;
    let version = if let Ok(mut mm_versionᶠ) = fs::File::open(&mm_versionᵖ) {
        buf = String::new();
        unwrap!(
            mm_versionᶠ.read_to_string(&mut buf),
            "Can't read from MM_VERSION"
        );
        buf.trim()
    } else {
        // If the MM_VERSION file is absent then we should create it
        // in order for the Cargo dependency management to see it,
        // because Cargo will keep rebuilding the `common` crate otherwise.
        //
        // We should probably fetch the actual git version here,
        // with something like `git log '--pretty=format:%h' -n 1` for the nightlies,
        // and a release tag when building from some kind of a stable branch,
        // though we should keep the ability for the tooling to provide the MM_VERSION
        // externally, because moving the entire ".git" around is not always practical.
        if let Ok(mut mm_versionᶠ) = fs::File::create(&mm_versionᵖ) {
            unwrap!(mm_versionᶠ.write_all(&b"UNKNOWN"[..]));
        }
        "UNKNOWN"
    };
    println!("cargo:rustc-env=MM_VERSION={}", version);
    version.into()
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
    unwrap!(thread::Builder::new()
        .name("forward".into())
        .spawn(move || {
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
        eprintln! ("$ {}", $program);
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
    use std::mem::uninitialized;
    use std::os::windows::ffi::OsStringExt;
    // https://msdn.microsoft.com/en-us/library/windows/desktop/ms724373(v=vs.85).aspx
    use winapi::um::sysinfoapi::GetSystemDirectoryW;

    let system = {
        let mut buf: [u16; 1024] = unsafe { uninitialized() };
        let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), (buf.len() - 1) as u32) };
        if len <= 0 {
            panic!("!GetSystemDirectoryW")
        }
        let len = len as usize;
        let system = OsString::from_wide(&buf[0..len]);
        Path::new(&system).to_path_buf()
    };
    eprintln!("windows_requirements] System directory is {:?}.", system);

    if !Path::new(r"c:\Program Files\LLVM\bin\libclang.dll").is_file() {
        // If `clang -v` works then maybe libclang is installed at a different location.
        let clang_v = ecmd!("clang", "-v").output();
        let clang_v = if let Ok(output) = clang_v {
            unsafe { String::from_utf8_unchecked(output.stdout) }
        } else {
            String::new()
        };
        if !clang_v.contains("clang version") {
            panic!(
                "\n\
                 windows_requirements]\n\
                 Per https://rust-lang.github.io/rust-bindgen/requirements.html\n\
                 please download and install a 'Windows (64-bit)' pre-build binary of LLVM\n\
                 from http://releases.llvm.org/download.html\n\
                 "
            );
        }
    }

    // `msvcr100.dll` is required by `ftp://sourceware.org/pub/pthreads-win32/prebuilt-dll-2-9-1-release/dll/x64/pthreadVC2.dll`
    let msvcr100 = system.join("msvcr100.dll");
    if !msvcr100.exists() {
        panic! ("msvcr100.dll is missing. \
            You can install it from https://www.microsoft.com/en-us/download/details.aspx?id=14632.");
    }

    // I don't exactly know what DLLs this download installs. Probably "msvcp140...". Might prove useful later.
    //You can install it from https://aka.ms/vs/15/release/vc_redist.x64.exe,
    //see https://support.microsoft.com/en-us/help/2977003/the-latest-supported-visual-c-downloads
}

#[cfg(not(windows))]
fn windows_requirements() {}

/// SuperNET's root.
/// Calculated from `CARGO_MANIFEST_DIR`.
/// NB: `cross` mounts it at "/project" and mounts it read-only!
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
        Path::new(if s.starts_with(r"\\?\") {
            &s[4..]
        } else {
            &s[..]
        })
        .into()
    } else {
        super_net
    }
}

/// A folder cargo creates for our build.rs specifically.
fn out_dir() -> PathBuf {
    // cf. https://github.com/rust-lang/cargo/issues/3368#issuecomment-265900350
    let out_dir = unwrap!(var("OUT_DIR"));
    let out_dir = Path::new(&out_dir);
    if !out_dir.is_dir() {
        panic!("OUT_DIR !is_dir")
    }
    out_dir.to_path_buf()
}

/// Absolute path taken from SuperNET's root + `path`.  
fn rabs(rrel: &str) -> PathBuf {
    root().join(rrel)
}

fn path2s<P>(path: P) -> String
where
    P: AsRef<Path>,
{
    let path: &Path = path.as_ref();
    unwrap!(path.to_str(), "Non-stringy path {:?}", path).into()
}

/// Downloads a file, placing it into the given path
/// and sharing the download status on the standard error stream.
///
/// Panics on errors.
///
/// The idea is to replace wget and cURL build dependencies, particularly on Windows.
/// Being able to see the status of the download in the terminal
/// seems more important here than the Future-based parallelism.
fn hget(url: &str, to: PathBuf) {
    // NB: Not using reqwest because I don't see a "hyper-rustls" option in
    // https://github.com/seanmonstar/reqwest/commit/82bc1be89e576b34f09f0f016b0ff38a22820ac5
    use hyper::client::HttpConnector;
    use hyper::header::{CONTENT_LENGTH, LOCATION};
    use hyper::{Body, Client, Request, StatusCode};

    eprintln!("hget] Downloading {} ...", url);

    let https = HttpsConnector::new(1);
    let pool = CpuPool::new(1);
    let client = Arc::new(Client::builder().executor(pool.clone()).build(https));

    fn rec(
        client: Arc<Client<HttpsConnector<HttpConnector>>>,
        request: Request<Body>,
        to: PathBuf,
    ) -> Box<dyn Future<Item = (), Error = ()> + Send> {
        Box::new(client.request(request) .then(move |res| -> Box<dyn Future<Item=(), Error=()> + Send> {
            let res = unwrap!(res);
            let status = res.status();
            if status == StatusCode::FOUND {
                let location = unwrap!(res.headers()[LOCATION].to_str());

                epintln!("hget] Redirected to "
                    if location.len() < 99 {  // 99 here is a numerically convenient screen width.
                        (location) " …"
                    } else {
                        (&location[0..33]) '…' (&location[location.len()-44..location.len()]) " …"
                    }
                );

                let request = unwrap!(Request::builder().uri(location) .body(Body::empty()));
                rec(client, request, to)
            } else if status == StatusCode::OK {
                let mut file = unwrap!(fs::File::create(&to), "hget] Can't create {:?}", to);
                // "cargo build -vv" shares the stderr with the user but buffers it on a line by line basis,
                // meaning that without some dirty terminal tricks we won't be able to share
                // a download status one-liner.
                // The alternative, then, is to share the status updates based on time:
                // If the download was working for five-ten seconds we want to share the status
                // with the user in order not to keep her in the dark.
                let mut received = 0;
                let mut last_status_update = now_float();
                let len: Option<usize> = res.headers().get(CONTENT_LENGTH) .map(|hv| unwrap!(unwrap!(hv.to_str()).parse()));
                Box::new(res.into_body().for_each(move |chunk| {
                    received += chunk.len();
                    if now_float() - last_status_update > 3. {
                        last_status_update = now_float();
                        epintln!(
                            {"hget] Fetched {:.0} KiB", received as f64 / 1024.}
                            if let Some(len) = len {{" out of {:.0}", len as f64 / 1024.}}
                            " …"
                        );
                    }
                    unwrap!(file.write_all(&chunk));
                    Ok(())
                }).then(move |r| -> Result<(), ()> {unwrap!(r); Ok(())}))
            } else {
                panic!("hget] Unknown status: {:?} (headers: {:?}", status, res.headers())
            }
        }))
    }

    let request = unwrap!(Request::builder().uri(url).body(Body::empty()));
    unwrap!(pool.spawn(rec(client, request, to)).wait())
}

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

#[derive(Debug)]
struct IosClangOps {
    /// iPhone SDK (iPhoneOS for arm64, iPhoneSimulator for x86_64)
    sysroot: &'static str,
    /// "arm64", "x86_64".
    arch: &'static str,
    /// Identifies the corresponding clang options defined in "user-config.jam".
    b2_toolset: &'static str,
    /// The minimal iOS version: 10 for 32-bit targets, 11 for 64-bit targets.
    ios_min: f64,
}

#[allow(non_camel_case_types)]
#[derive(PartialEq, Eq, Debug)]
enum Target {
    Unix,
    Mac,
    Windows,
    /// https://github.com/rust-embedded/cross
    AndroidCross,
    /// https://github.com/TimNN/cargo-lipo
    iOS(String),
}
impl Target {
    fn load() -> Target {
        let targetᴱ = unwrap!(var("TARGET"));
        match &targetᴱ[..] {
            "x86_64-unknown-linux-gnu" => Target::Unix,
            // Used when compiling MM from under Raspberry Pi.
            "armv7-unknown-linux-gnueabihf" => Target::Unix,
            "x86_64-apple-darwin" => Target::Mac,
            "x86_64-pc-windows-msvc" => Target::Windows,
            "wasm32-unknown-emscripten" => Target::Unix, // Pretend.
            "armv7-linux-androideabi" => {
                if Path::new("/android-ndk").exists() {
                    Target::AndroidCross
                } else {
                    panic!(
                        "/android-ndk not found. Please use the `cross` as described at \
                         https://github.com/artemii235/SuperNET/blob/mm2-cross/docs/ANDROID.md"
                    )
                }
            }
            "aarch64-apple-ios" => Target::iOS(targetᴱ),
            "x86_64-apple-ios" => Target::iOS(targetᴱ),
            "armv7-apple-ios" => Target::iOS(targetᴱ),
            "armv7s-apple-ios" => Target::iOS(targetᴱ),
            t => panic!("Target not (yet) supported: {}", t),
        }
    }
    /// True if building for ARM under https://github.com/rust-embedded/cross
    /// or a similar setup based on the "japaric/armv7-linux-androideabi" Docker image.
    fn is_android_cross(&self) -> bool {
        *self == Target::AndroidCross
    }
    fn is_ios(&self) -> bool {
        match self {
            &Target::iOS(_) => true,
            _ => false,
        }
    }
    fn is_mac(&self) -> bool {
        *self == Target::Mac
    }
    /// The "-arch" parameter passed to Xcode clang++ when cross-building for iOS.
    fn ios_clang_ops(&self) -> Option<IosClangOps> {
        match self {
            &Target::iOS(ref target) => match &target[..] {
                "aarch64-apple-ios" => Some(IosClangOps {
                    // cf. `xcrun --sdk iphoneos --show-sdk-path`
                    sysroot: "/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk",
                    arch: "arm64",
                    b2_toolset: "darwin-iphone",
                    ios_min: 11.0,
                }),
                "x86_64-apple-ios" => Some(IosClangOps {
                    sysroot: "/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk",
                    arch: "x86_64",
                    b2_toolset: "darwin-iphonesim",
                    ios_min: 11.0,
                }),
                // armv7, 32-bit
                "armv7-apple-ios" => Some(IosClangOps {
                    sysroot: "/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk",
                    arch: "armv7",
                    b2_toolset: "darwin-iphone10v7",
                    ios_min: 10.0,
                }),
                //"armv7s-apple-ios" => "armv7s", 32-bit
                _ => None,
            },
            _ => None,
        }
    }
    fn cc(&self, plus_plus: bool) -> cc::Build {
        let mut cc = cc::Build::new();
        if self.is_android_cross() {
            cc.compiler(if plus_plus {
                // NB: GCC is a link to Clang in the NDK.
                "/android-ndk/bin/clang++"
            } else {
                "/android-ndk/bin/clang"
            });
            cc.archiver("/android-ndk/bin/arm-linux-androideabi-ar");
        } else if self.is_ios() {
            let cops = unwrap!(self.ios_clang_ops());
            // cf. `xcode-select -print-path`
            cc.compiler(if plus_plus {
                "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang++"
            } else {
                "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang"
            });
            cc.flag(&fomat!("--sysroot="(cops.sysroot)));
            cc.flag("-stdlib=libc++");
            cc.flag(&fomat!("-miphoneos-version-min="(cops.ios_min)));
            cc.flag(&fomat!("-mios-simulator-version-min="(cops.ios_min)));
            cc.flag(&fomat!("-DIPHONEOS_DEPLOYMENT_TARGET="(cops.ios_min)));
            cc.flag("-arch").flag(cops.arch);
        }
        cc
    }
}
impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &Target::iOS(ref target) => f.write_str(&target[..]),
            _ => wite!(f, [self]),
        }
    }
}

fn fetch_boost(_target: &Target) -> PathBuf {
    let out_dir = out_dir();
    let prefix = out_dir.join("boost");
    let boost_system = prefix.join("lib/libboost_system.a");
    if boost_system.exists() {
        return prefix;
    }

    let boost = out_dir.join("boost_1_68_0");
    epintln!("Boost at "[boost]);
    if !boost.exists() {
        // [Download and] unpack Boost.
        let tbz = out_dir.join("boost_1_68_0.tar.bz2");
        if !tbz.exists() {
            let tmp = tbz.with_extension("bz2-tmp");
            let _ = fs::remove_file(&tmp);
            hget(
                "https://dl.bintray.com/boostorg/release/1.68.0/source/boost_1_68_0.tar.bz2",
                tmp.clone(),
            );
            unwrap!(fs::rename(tmp, tbz));
        }

        // Boost is huge, a full installation will impact the build time
        // and might hit the CI space limits.
        // To avoid this we unpack only a small subset.

        // Example using bcp to help with finding a part of the subset:
        // sh bootstrap.sh
        // ./b2 release address-model=64 link=static cxxflags=-fPIC cxxstd=11 define=BOOST_ERROR_CODE_HEADER_ONLY stage --with-date_time --with-system
        // ./b2 release address-model=64 link=static cxxflags=-fPIC cxxstd=11 define=BOOST_ERROR_CODE_HEADER_ONLY tools/bcp
        // dist/bin/bcp --scan --list ../libtorrent-rasterbar-1.2.0/src/*.cpp

        let f = unwrap!(fs::File::open(out_dir.join("boost_1_68_0.tar.bz2")));
        let bz2 = BzDecoder::new(f);
        let mut a = Archive::new(bz2);
        for en in unwrap!(a.entries()) {
            let mut en = unwrap!(en);
            let path = unwrap!(en.path());
            let pathˢ = unwrap!(path.to_str());
            assert!(pathˢ.starts_with("boost_1_68_0/"));
            let pathˢ = &pathˢ[13..];
            let unpack = pathˢ == "bootstrap.sh"
                || pathˢ == "bootstrap.bat"
                || pathˢ == "boost-build.jam"
                || pathˢ == "boostcpp.jam"
                || pathˢ == "boost/assert.hpp"
                || pathˢ == "boost/aligned_storage.hpp"
                || pathˢ == "boost/array.hpp"
                || pathˢ.starts_with("boost/asio/")
                || pathˢ.starts_with("boost/blank")
                || pathˢ.starts_with("boost/bind")
                || pathˢ == "boost/call_traits.hpp"
                || pathˢ.starts_with("boost/callable_traits/")
                || pathˢ == "boost/cerrno.hpp"
                || pathˢ == "boost/config.hpp"
                || pathˢ == "boost/concept_check.hpp"
                || pathˢ == "boost/crc.hpp"
                || pathˢ.starts_with("boost/container")
                || pathˢ.starts_with("boost/container_hash/")
                || pathˢ.starts_with("boost/concept/")
                || pathˢ.starts_with("boost/config/")
                || pathˢ.starts_with("boost/core/")
                || pathˢ.starts_with("boost/chrono")
                || pathˢ == "boost/cstdint.hpp"
                || pathˢ == "boost/current_function.hpp"
                || pathˢ == "boost/checked_delete.hpp"
                || pathˢ.starts_with("boost/date_time/")
                || pathˢ.starts_with("boost/detail/")
                || pathˢ.starts_with("boost/exception/")
                || pathˢ.starts_with("boost/fusion/")
                || pathˢ.starts_with("boost/function")
                || pathˢ.starts_with("boost/functional")
                || pathˢ == "boost/get_pointer.hpp"
                || pathˢ.starts_with("boost/iterator/")
                || pathˢ.starts_with("boost/intrusive")
                || pathˢ.starts_with("boost/integer")
                || pathˢ.starts_with("boost/io")
                || pathˢ.starts_with("boost/lexical_cast")
                || pathˢ == "boost/limits.hpp"
                || pathˢ.starts_with("boost/mpl/")
                || pathˢ.starts_with("boost/math")
                || pathˢ.starts_with("boost/move")
                || pathˢ.starts_with("boost/multiprecision")
                || pathˢ == "boost/mem_fn.hpp"
                || pathˢ == "boost/next_prior.hpp"
                || pathˢ == "boost/noncopyable.hpp"
                || pathˢ.starts_with("boost/none")
                || pathˢ.starts_with("boost/numeric/")
                || pathˢ == "boost/operators.hpp"
                || pathˢ.starts_with("boost/optional")
                || pathˢ.starts_with("boost/predef")
                || pathˢ.starts_with("boost/preprocessor/")
                || pathˢ.starts_with("boost/pool/")
                || pathˢ == "boost/ref.hpp"
                || pathˢ.starts_with("boost/range/")
                || pathˢ.starts_with("boost/ratio")
                || pathˢ.starts_with("boost/system/")
                || pathˢ.starts_with("boost/smart_ptr/")
                || pathˢ == "boost/scope_exit.hpp"
                || pathˢ == "boost/static_assert.hpp"
                || pathˢ == "boost/shared_ptr.hpp"
                || pathˢ == "boost/shared_array.hpp"
                || pathˢ == "boost/swap.hpp"
                || pathˢ.starts_with("boost/type_traits")
                || pathˢ.starts_with("boost/type_index")
                || pathˢ.starts_with("boost/typeof")
                || pathˢ.starts_with("boost/tuple/")
                || pathˢ.starts_with("boost/thread")
                || pathˢ.starts_with("boost/token")
                || pathˢ == "boost/throw_exception.hpp"
                || pathˢ == "boost/type.hpp"
                || pathˢ.starts_with("boost/utility/")
                || pathˢ == "boost/utility.hpp"
                || pathˢ.starts_with("boost/variant")
                || pathˢ == "boost/version.hpp"
                || pathˢ.starts_with("boost/winapi/")
                || pathˢ.starts_with("libs/config/")
                || pathˢ.starts_with("libs/chrono/")
                || pathˢ.starts_with("libs/date_time/")
                || pathˢ.starts_with("libs/system/")
                || pathˢ.starts_with("tools/build/")
                || pathˢ == "Jamroot";
            if !unpack {
                continue;
            }
            unwrap!(en.unpack_in(&out_dir));
        }

        assert!(boost.exists());
    }

    let b2 = boost.join(if cfg!(windows) { "b2.exe" } else { "b2" });
    if !b2.exists() {
        if cfg!(windows) {
            run!(ecmd!("cmd", "/c", "bootstrap.bat").current_dir(&boost));
        } else {
            run!(ecmd!("/bin/sh", "bootstrap.sh").current_dir(&boost));
        }
        assert!(b2.exists());
    }
    boost
}

/// In-place modification of file contents. Like "sed -i" (or "perl -i") but with a Rust code instead of a sed pattern.
/// Uses `String` instead of `Vec<u8>` simply because `str` has more helpers out of the box (`std::str::pattern` works with `str`).
/// Returns `false` if the file is absent or empty. Panics on errors.
fn with_file(path: &AsRef<Path>, visitor: &Fn(&mut String)) -> bool {
    let bulk = slurp(path);
    if bulk.is_empty() {
        return false;
    }
    let mut bulk = unwrap!(String::from_utf8(bulk));
    let copy = bulk.clone();
    visitor(&mut bulk);
    if copy == bulk {
        return true; // Not modified.
    }
    let tmp = fomat! ((unwrap! (path.as_ref().to_str())) ".tmp");
    let mut out = unwrap!(fs::File::create(&tmp));
    unwrap!(out.write_all(bulk.as_bytes()));
    drop(out);
    let permissions = unwrap!(fs::metadata(path)).permissions();
    unwrap!(fs::set_permissions(&tmp, permissions));
    unwrap!(fs::rename(tmp, path));
    eprintln!("Patched {:?}", path.as_ref());
    true
}

/// Patch libtorrent to work with the older version of clang we have in the `cross`.
fn patch_libtorrent(rasterbar: &Path) {
    let patched_flag = rasterbar.join("patched.flag");
    if !patched_flag.exists() {
        assert!(with_file(
            &rasterbar.join("include/libtorrent/config.hpp"),
            &|hh| if !hh.contains("boost/exception/to_string.hpp") {
                *hh = hh.replace(
                    "#include <boost/config.hpp>",
                    "#include <boost/config.hpp>\n\
                     #include <boost/exception/to_string.hpp>",
                );
                assert!(hh.contains("boost/exception/to_string.hpp"));
            }
        ));

        assert!(with_file(
            &rasterbar.join("include/libtorrent/file_storage.hpp"),
            &|hh| *hh = hh.replace(
                "file_entry& operator=(file_entry&&) & noexcept = default;",
                "file_entry& operator=(file_entry&&) & = default;"
            ),
        ));

        assert!(with_file(
            &rasterbar.join("include/libtorrent/broadcast_socket.hpp"),
            &|hh| *hh = hh.replace(
                "std::array<char, 1500> buffer{};",
                "std::array<char, 1500> buffer;"
            )
        ));

        let paths = &[
            "include/libtorrent/units.hpp",
            "src/alert.cpp",
            "include/libtorrent/bencode.hpp",
            "src/bdecode.cpp",
            "include/libtorrent/session.hpp",
            "src/path.cpp",
            "src/file_storage.cpp",
            "src/http_parser.cpp",
            "src/http_tracker_connection.cpp",
            "src/i2p_stream.cpp",
            "src/identify_client.cpp",
            "src/lazy_bdecode.cpp",
            "src/lazy_bdecode.cpp",
            "src/lsd.cpp",
            "src/lsd.cpp",
            "src/natpmp.cpp",
            "src/natpmp.cpp",
            "src/socket_io.cpp",
            "src/torrent.cpp",
            "src/torrent_info.cpp",
            "src/upnp.cpp",
            "src/upnp.cpp",
            "src/stack_allocator.cpp",
            "src/kademlia/msg.cpp",
            "src/kademlia/item.cpp",
        ];
        for path in paths {
            assert!(
                with_file(&rasterbar.join(path), &|cc| *cc = cc
                    .replace("std::to_string", "boost::to_string")
                    .replace("std::snprintf", "snprintf")
                    .replace("std::vsnprintf", "vsnprintf")
                    .replace("std::strtoll", "strtoll")),
                "Not found: {}",
                path
            )
        }

        unwrap!(fs::File::create(patched_flag));
    }
}

/// Downloads and builds libtorrent.
fn build_libtorrent(boost: &Path, target: &Target) -> (PathBuf, PathBuf) {
    let out_dir = out_dir();
    // NB: On Windows the path length is limited
    // and we should use a short folder name for a better chance of fitting it.
    let rasterbar = out_dir.join("lt");
    epintln!("libtorrent at "[rasterbar]);

    if !rasterbar.exists() {
        let tgz = out_dir.join("lt.tgz");
        if !tgz.exists() {
            // Released tgz version fails to link for iOS due to https://github.com/arvidn/libtorrent/pull/3629,
            // should get a fresh Git version instead. Except we don't have "git" when building for Android,
            // so we're using the [tarball API](https://stackoverflow.com/a/8378458/257568) instead.
            hget(
                "https://codeload.github.com/arvidn/libtorrent/legacy.tar.gz/RC_1_2",
                tgz.clone(),
            );
            assert!(tgz.exists());
        }
        let mut f = unwrap!(fs::File::open(&tgz));
        let gz = unwrap!(Decoder::new(&mut f));
        let mut a = Archive::new(gz);
        let libtorrent = out_dir.join("ltt");
        for en in unwrap!(a.entries()) {
            let mut en = unwrap!(en);
            let pathⁱ = unwrap!(en.path());
            let pathⱼ = pathⁱ
                .components()
                .skip(1)
                .map(|c| {
                    if let Component::Normal(c) = c {
                        c
                    } else {
                        panic!("Bad path: {:?}, {:?}", pathⁱ, c)
                    }
                })
                .fold(PathBuf::new(), |p, c| p.join(c));
            unwrap!(en.unpack(libtorrent.join(pathⱼ)));
        }
        assert!(libtorrent.is_dir());
        unwrap!(fs::rename(libtorrent, &rasterbar));
    }

    let include = rasterbar.join("include");
    assert!(include.is_dir());

    fn find_libtorrent_a(rasterbar: &Path, target: &Target) -> Option<PathBuf> {
        // The library path is different for every platform and toolset version.
        // The alternative to *finding* the library is adding " install --prefix=../lti"
        // to the BJam build, but that would waste space and time.
        let mut search_from = rasterbar.join("bin");
        if let Target::iOS(ref _targetᴱ) = target {
            let cops = unwrap!(target.ios_clang_ops());
            search_from = search_from.join(cops.b2_toolset)
        }
        let search_for = if cfg!(windows) {
            "libtorrent.lib"
        } else {
            "libtorrent.a"
        };
        let mut lib_paths: Vec<_> = unwrap!(glob(unwrap!(search_from
            .join("**")
            .join(search_for)
            .to_str())))
        .collect();
        if lib_paths.is_empty() {
            None
        } else if lib_paths.len() > 1 {
            panic!(
                "Multiple versions of {} found in {:?}",
                search_for, search_from
            )
        } else {
            let a = unwrap!(lib_paths.remove(0));
            assert!(a.is_file());
            assert!(a.starts_with(&rasterbar));
            Some(a)
        }
    }

    if let Some(existing_a) = find_libtorrent_a(&rasterbar, &target) {
        return (existing_a, include);
    }

    // This version of the build doesn't compile Boost separately
    // but rather allows the libtorrent to compile it
    // "you probably want to just build libtorrent and have it build boost
    //  (otherwise you'll end up building the boost dependencies twice)"
    //  - https://github.com/arvidn/libtorrent/issues/26#issuecomment-121478708

    let boostˢ = unwrap!(boost.to_str());
    let boostᵉ = escape_some(boostˢ.into());
    // NB: The common compiler flags go to the "cxxflags=" here
    // and the platform-specific flags go to the jam files or to conditionals below.
    let mut b2 = fomat!(
        "b2 -j4 -d+2 release"
        " link=static deprecated-functions=off debug-symbols=off"
        " dht=on encryption=on crypto=built-in iconv=off i2p=off"
        " cxxflags=-DBOOST_ERROR_CODE_HEADER_ONLY=1"
        " cxxflags=-std=c++11"
        " cxxflags=-fPIC"
        " include="(boostᵉ)
    );

    fn jam(boost: &Path, from: &str) {
        let user_config_jamˢ = slurp(&root().join(from));
        let user_config_jamᵖ = boost.join("tools/build/src/user-config.jam");
        epintln!("build_libtorrent] Creating "[user_config_jamᵖ]"…");
        let mut user_config_jamᶠ = unwrap!(fs::File::create(&user_config_jamᵖ));
        unwrap!(user_config_jamᶠ.write_all(&user_config_jamˢ));
    }
    if target.is_ios() {
        // NB: The "darwin" toolset is defined in "tools/build/src/tools/darwin.jam":
        jam(boost, "mm2src/common/ios/user-config.jam");
        let cops = unwrap!(target.ios_clang_ops());
        unwrap!(wite!(&mut b2, " toolset="(cops.b2_toolset)));
    } else if target.is_android_cross() {
        jam(boost, "mm2src/common/android/user-config.jam");
        unwrap!(wite!(&mut b2, " toolset=gcc"));
        patch_libtorrent(&rasterbar);
    } else if cfg!(windows) {
        unwrap!(wite!(&mut b2, " toolset=msvc-14.1 address-model=64"));
    }

    let boost_build_path = boost.join("tools").join("build");
    let boost_build_pathˢ = unwrap!(boost_build_path.to_str());
    let export = if cfg!(windows) { "SET" } else { "export" };
    epintln!("build_libtorrent]\n"
      "  $ "(export)" PATH="(boostᵉ) if cfg!(windows) {";%PATH%"} else {":$PATH"} "\n"
      "  $ "(export)" BOOST_BUILD_PATH="(escape_some(boost_build_pathˢ)) "\n"
      "  $ "(b2));
    if cfg!(windows) {
        run!(ecmd!("cmd", "/c", b2)
            .env("PATH", format!("{};{}", boostˢ, unwrap!(var("PATH"))))
            .env("BOOST_BUILD_PATH", boost_build_path)
            .env_remove("BOOST_ROOT") // cf. https://stackoverflow.com/a/55141466/257568
            .current_dir(&rasterbar));
    } else {
        run!(ecmd!("/bin/sh", "-c", b2)
            .env("PATH", format!("{}:{}", boostˢ, unwrap!(var("PATH"))))
            .env("BOOST_BUILD_PATH", boost_build_path)
            .env_remove("BOOST_ROOT") // cf. https://stackoverflow.com/a/55141466/257568
            .current_dir(&rasterbar));
    }

    let a = unwrap!(find_libtorrent_a(&rasterbar, &target));
    (a, include)
}

#[cfg(not(feature = "native"))]
fn libtorrent() {}

#[cfg(feature = "native")]
fn libtorrent() {
    // NB: Distributions should have a copy of https://github.com/arvidn/libtorrent/blob/master/LICENSE.

    let target = Target::load();
    let boost = fetch_boost(&target);
    let (lt_a, lt_include) = build_libtorrent(&boost, &target);
    println!("cargo:rustc-link-lib=static={}", {
        let name = unwrap!(unwrap!(lt_a.file_stem()).to_str());
        if cfg!(windows) {
            name
        } else {
            &name[3..]
        }
    });
    println!(
        "cargo:rustc-link-search=native={}",
        unwrap!(unwrap!(lt_a.parent()).to_str())
    );

    if cfg!(windows) {
        println!("cargo:rustc-link-lib=iphlpapi"); // NotifyAddrChange.
    }

    epintln!("Building dht.cc …");
    let lm_dht = unwrap!(last_modified_sec(&"dht.cc"), "Can't stat dht.cc");
    let out_dir = unwrap!(var("OUT_DIR"), "!OUT_DIR");
    let lib_path = Path::new(&out_dir).join("libdht.a");
    let lm_lib = last_modified_sec(&lib_path).unwrap_or(0.);
    if lm_dht >= lm_lib - SLIDE {
        let mut cc = target.cc(true);

        // Mismatch between the libtorrent and the dht.cc flags
        // might produce weird "undefined reference" link errors.
        // Building libtorrent with "-d+2" passed to "b2" should show the actual defines.

        cc.flag("-DBOOST_ALL_NO_LIB");
        cc.flag("-DBOOST_ASIO_ENABLE_CANCELIO");
        cc.flag("-DBOOST_ASIO_HAS_STD_CHRONO");
        cc.flag("-DBOOST_MULTI_INDEX_DISABLE_SERIALIZATION");
        cc.flag("-DBOOST_NO_DEPRECATED");
        cc.flag("-DBOOST_SYSTEM_NO_DEPRECATED");
        cc.flag("-DNDEBUG");
        cc.flag("-DTORRENT_BUILDING_LIBRARY");
        cc.flag("-DTORRENT_NO_DEPRECATE");
        cc.flag("-DTORRENT_USE_I2P=0");
        cc.flag("-DTORRENT_USE_ICONV=0");
        if cfg!(windows) {
            cc.flag("-DWIN32");
            cc.flag("-DWIN32_LEAN_AND_MEAN");
            // https://stackoverflow.com/questions/7582394/strdup-or-strdup
            cc.flag("-D_CRT_SECURE_NO_DEPRECATE");
            cc.flag("-D_FILE_OFFSET_BITS=64");
            cc.flag("-D_SCL_SECURE_NO_DEPRECATE");
            cc.flag("-D_WIN32");
            // https://docs.microsoft.com/en-us/cpp/porting/modifying-winver-and-win32-winnt?view=vs-2017
            cc.flag("-D_WIN32_WINNT=0x0600");
            cc.flag("-D__USE_W32_SOCKETS");
            // cf. https://stackoverflow.com/questions/4573536/ehsc-vc-eha-synchronous-vs-asynchronous-exception-handling
            cc.flag("/EHsc");
        } else {
            cc.flag("-fexceptions");
            cc.flag("-D_FILE_OFFSET_BITS=64");
            cc.flag("-D_WIN32_WINNT=0x0600");
            cc.flag("-std=c++11");
            cc.flag("-ftemplate-depth=512");
            cc.flag("-finline-functions");
            cc.flag("-fvisibility=hidden");
            cc.flag("-fvisibility-inlines-hidden");
        }

        // Fixes the «Undefined symbols… "boost::system::detail::generic_category_ncx()"».
        cc.flag("-DBOOST_ERROR_CODE_HEADER_ONLY=1");

        cc.file("dht.cc")
            .warnings(true)
            .opt_level(2)
            .pic(true)
            .include(lt_include)
            .include(boost)
            .compile("dht");
    }
    println!("cargo:rustc-link-lib=static=dht");
    println!("cargo:rustc-link-search=native={}", out_dir);

    if target.is_mac() {
        println!("cargo:rustc-link-lib=c++");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=SystemConfiguration");
    } else if cfg!(windows) {
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }
}

/// We often use a fresh version of CMake and it might be missing from the default PATH.
fn cmake_path() -> String {
    fomat!("/usr/local/bin:"(unwrap!(var("PATH"))))
}

lazy_static! {
    static ref LIBEXCHANGES_SRC: Vec<PathBuf> = vec![
        rabs("iguana/exchanges/mm.c"),
    ];
}

/// Build MM1 libraries without CMake, making cross-platform builds more transparent to us.
fn manual_mm1_build(target: Target) {
    let (root, out_dir) = (root(), out_dir());

    let exchanges_build = out_dir.join("exchanges_build");
    epintln!("exchanges_build at "[exchanges_build]);

    let libexchanges_a = out_dir.join("libexchanges.a");
    if make(&libexchanges_a, &LIBEXCHANGES_SRC[..]) {
        let _ = fs::create_dir(&exchanges_build);
        let mut cc = target.cc(false);
        for p in LIBEXCHANGES_SRC.iter() {
            cc.file(p);
        }
        cc.compile("exchanges");
        assert!(libexchanges_a.is_file());
    }
    println!("cargo:rustc-link-lib=static=exchanges");
    println!("cargo:rustc-link-search=native={}", path2s(&out_dir));

    // TODO: Rebuild the libraries when the C source code is updated.
}

/// Build helper C code.
///
/// I think "git clone ... && cargo build" should be enough to start hacking on the Rust code.
///
/// For now we're building the Structured Exception Handling code here,
/// but in the future we might subsume the rest of the C build under build.rs.
fn build_c_code(mm_version: &str) {
    // Link in the Windows-specific crash handling code.

    if cfg!(windows) {
        let lm_seh = unwrap!(last_modified_sec(&"seh.c"), "Can't stat seh.c");
        let out_dir = unwrap!(var("OUT_DIR"), "!OUT_DIR");
        let lib_path = Path::new(&out_dir).join("libseh.a");
        let lm_lib = last_modified_sec(&lib_path).unwrap_or(0.);
        if lm_seh >= lm_lib - SLIDE {
            cc::Build::new().file("seh.c").warnings(true).compile("seh");
        }
        println!("cargo:rustc-link-lib=static=seh");
        println!("cargo:rustc-link-search=native={}", out_dir);
    }

    // The MM1 library.

    let target = Target::load();
    if target.is_android_cross() || target.is_ios() {
        manual_mm1_build(target);
        return;
    }

    let _ = fs::create_dir(root().join("build"));
    let _ = fs::create_dir_all(root().join("target/debug"));

    // NB: With "duct 0.11.0" the `let _` variable binding is necessary in order for the build not to fall detached into background.
    let mut cmake_prep_args: Vec<String> = Vec::new();
    if cfg!(windows) {
        // To flush the build problems early we explicitly specify that we want a 64-bit MSVC build and not a GNU or 32-bit one.
        cmake_prep_args.push("-G".into());
        cmake_prep_args.push("Visual Studio 15 2017 Win64".into());
    }
    cmake_prep_args.push(format!("-DMM_VERSION={}", mm_version));
    cmake_prep_args.push("-DCMAKE_BUILD_TYPE=Debug".into());
    cmake_prep_args.push("..".into());
    eprintln!("$ cmake{}", show_args(&cmake_prep_args));
    run!(ecmd!("cmake", i cmake_prep_args).current_dir(root().join("build")));

    let cmake_args: Vec<String> = vec![
        "--build".into(),
        ".".into(),
        "--target".into(),
        "marketmaker-lib".into(),
    ];
    eprintln!("$ cmake{}", show_args(&cmake_args));
    run!(ecmd!("cmake", i cmake_args).current_dir(root().join("build")));

    println!("cargo:rustc-link-lib=static=marketmaker-lib");

    if cfg!(windows) {
        println!("cargo:rustc-link-search=native={}", path2s(rabs("x64")));
        // When building locally with CMake 3.12.0 on Windows the artefacts are created in the "Debug" folders:
        println!(
            "cargo:rustc-link-search=native={}",
            path2s(rabs("build/iguana/exchanges/Debug"))
        );
    // https://stackoverflow.com/a/10234077/257568
    //println!(r"cargo:rustc-link-search=native=c:\Program Files (x86)\Microsoft Visual Studio\2017\BuildTools\VC\Tools\MSVC\14.14.26428\lib\x64");
    } else {
        println!(
            "cargo:rustc-link-search=native={}",
            path2s(rabs("build/iguana/exchanges"))
        );
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

/// Find shell-matching paths with the pattern relative to the `root`.
fn globʳ(root_glob: &str) -> Result<Paths, PatternError> {
    let full_glob = root().join(root_glob);
    let full_glob = unwrap!(full_glob.to_str());
    glob(full_glob)
}

fn rerun_if_changed(root_glob: &str) {
    for path in unwrap!(globʳ(root_glob)) {
        let path = unwrap!(path);
        println!("cargo:rerun-if-changed={}", path2s(path));
    }
}

fn main() {
    // NB: `rerun-if-changed` will ALWAYS invoke the build.rs if the target does not exists.
    // cf. https://github.com/rust-lang/cargo/issues/4514#issuecomment-330976605
    //     https://github.com/rust-lang/cargo/issues/4213#issuecomment-310697337
    // `RUST_LOG=cargo::core::compiler::fingerprint cargo build` shows the fingerprit files used.

    println!("cargo:rerun-if-changed={}", path2s(rabs("MM_VERSION")));
    let mm_version = mm_version();

    if cfg!(not(feature = "native")) {
        return;
    }

    rerun_if_changed("iguana/exchanges/*.c");
    rerun_if_changed("crypto777/*.c");
    rerun_if_changed("crypto777/jpeg/*.c");
    println!("cargo:rerun-if-changed={}", path2s(rabs("CMakeLists.txt")));

    // NB: Using `rerun-if-env-changed` disables the default dependency heuristics.
    // cf. https://github.com/rust-lang/cargo/issues/4587
    // We should avoid using it for now.

    // Rebuild when we change certain features.
    //println!("rerun-if-env-changed=CARGO_FEATURE_NOP");

    windows_requirements();
    libtorrent();
    build_c_code(&mm_version);
    generate_bindings();
}

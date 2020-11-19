// The script here will translate some of the C headers necessary for the gradual Rust port into the corresponding Rust files.
// Going to take the *whitelisting* approach, converting just the necessary definitions, in order to keep the builds fast.

// The script is experimentally formatted with `rustfmt`. Probably not going to use `rustfmt` for the rest of the project though.

// Bindgen requirements: https://rust-lang.github.io/rust-bindgen/requirements.html
//              Windows: https://github.com/rust-lang-nursery/rustup.rs/issues/1003#issuecomment-289825927
// On build.rs: https://doc.rust-lang.org/cargo/reference/build-scripts.html

#![allow(uncommon_codepoints)]
#![feature(non_ascii_idents)]
#![cfg_attr(not(feature = "native"), allow(dead_code))]

#[macro_use] extern crate fomat_macros;
#[macro_use] extern crate unwrap;

use chrono::DateTime;
use glob::{glob, Paths, PatternError};
use gstuff::{last_modified_sec, slurp};
use regex::Regex;
use std::env::{self, var};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::str::{from_utf8, from_utf8_unchecked};
use std::thread;
use std::{fmt, fs};

/// Ongoing (RLS) builds might interfere with a precise time comparison.
const SLIDE: f64 = 60.;

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

#[derive(PartialEq, Eq, Debug)]
enum Target {
    Unix,
    Mac,
    Windows,
    /// https://github.com/rust-embedded/cross
    AndroidCross(String),
    /// https://github.com/TimNN/cargo-lipo
    iOS(String),
}
impl Target {
    fn load() -> Target {
        let targetᴱ = unwrap!(var("TARGET"));
        match &targetᴱ[..] {
            "x86_64-unknown-linux-gnu" => Target::Unix,
            "armv7-unknown-linux-gnueabihf" => Target::Unix, // Raspbian is a modified Debian
            "arm-unknown-linux-gnueabihf" => Target::Unix,   // Raspbian under QEMU
            "x86_64-apple-darwin" => Target::Mac,
            "x86_64-pc-windows-msvc" => Target::Windows,
            "wasm32-unknown-emscripten" => Target::Unix, // Pretend.
            "armv7-linux-androideabi" | "aarch64-linux-android" | "i686-linux-android" | "x86_64-linux-android" => {
                if Path::new("/android-ndk").exists() {
                    Target::AndroidCross(targetᴱ)
                } else {
                    panic!(
                        "/android-ndk not found. Please use the `cross` as described at \
                         https://github.com/artemii235/SuperNET/blob/mm2-cross/docs/ANDROID.md"
                    )
                }
            },
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
        match self {
            Target::AndroidCross(_) => true,
            _ => false,
        }
    }
    fn is_ios(&self) -> bool {
        match self {
            &Target::iOS(_) => true,
            _ => false,
        }
    }
    fn is_mac(&self) -> bool { *self == Target::Mac }
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
            panic!("libclang-3.9-dev needs to be installed in order for the cross-compilation to work");
        }
    }

    let c_headers = out_dir().join("c_headers");
    let _ = fs::create_dir(&c_headers);
    assert!(c_headers.is_dir());

    bindgen(
        vec!["../../iguana/exchanges/LP_include.h".into()],
        c_headers.join("LP_include.rs"),
        [
            // functions
            "OS_ensure_directory",
        ]
        .iter(),
        // types
        [].iter(),
        [].iter(),
    );
}

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
        let stripped = match s.strip_prefix(r"\\?\") {
            Some(stripped) => stripped,
            None => &s,
        };
        Path::new(stripped).into()
    } else {
        super_net
    }
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

/// Build MM1 libraries without CMake, making cross-platform builds more transparent to us.
fn manual_mm1_build(target: Target) {
    let (root, out_dir) = (root(), out_dir());
    let libexchanges_src = vec![rabs("iguana/exchanges/mm.c")];

    let exchanges_build = out_dir.join("exchanges_build");
    epintln!("exchanges_build at "[exchanges_build]);

    let libexchanges_a = out_dir.join("libexchanges.a");
    if make(&libexchanges_a, &libexchanges_src[..]) {
        let _ = fs::create_dir(&exchanges_build);
        let mut cc = target.cc(false);
        for p in libexchanges_src.iter() {
            cc.file(p);
        }
        cc.compile("exchanges");
        assert!(libexchanges_a.is_file());
    }
    println!("cargo:rustc-link-lib=static=exchanges");
    println!("cargo:rustc-link-search=native={}", path2s(&out_dir));

    // TODO: Rebuild the libraries when the C source code is updated.
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
fn rabs(rrel: &str) -> PathBuf { root().join(rrel) }

fn path2s<P>(path: P) -> String
where
    P: AsRef<Path>,
{
    let path: &Path = path.as_ref();
    unwrap!(path.to_str(), "Non-stringy path {:?}", path).into()
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
    println!("cargo:rerun-if-changed={}", path2s(rabs("MM_DATETIME")));
    let mm_version = mm_version();

    if cfg!(not(feature = "native")) {
        return;
    }

    rerun_if_changed("iguana/exchanges/CMakeLists.txt");
    rerun_if_changed("iguana/exchanges/LP_include.h");
    rerun_if_changed("iguana/exchanges/mm.c");
    println!("cargo:rerun-if-changed={}", path2s(rabs("CMakeLists.txt")));

    // NB: Using `rerun-if-env-changed` disables the default dependency heuristics.
    // cf. https://github.com/rust-lang/cargo/issues/4587
    // We should avoid using it for now.

    // Rebuild when we change certain features.
    //println!("rerun-if-env-changed=CARGO_FEATURE_NOP");

    windows_requirements();
    build_c_code(&mm_version);
    generate_bindings();
}

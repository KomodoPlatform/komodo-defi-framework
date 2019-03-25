// The script here will translate some of the C headers necessary for the gradual Rust port into the corresponding Rust files.
// Going to take the *whitelisting* approach, converting just the necessary definitions, in order to keep the builds fast.

// The script is experimentally formatted with `rustfmt`. Probably not going to use `rustfmt` for the rest of the project though.

// Bindgen requirements: https://rust-lang.github.io/rust-bindgen/requirements.html
//              Windows: https://github.com/rust-lang-nursery/rustup.rs/issues/1003#issuecomment-289825927
// On build.rs: https://doc.rust-lang.org/cargo/reference/build-scripts.html

#![feature(non_ascii_idents)]

#[macro_use]
extern crate fomat_macros;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate unwrap;

use bzip2::read::BzDecoder;
use duct::cmd;
use futures::{Future, Stream};
use futures_cpupool::CpuPool;
use glob::{glob, Paths, PatternError};
use gstuff::{last_modified_sec, now_float, slurp};
use hyper_rustls::HttpsConnector;
use std::cmp::max;
use std::env::var;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::iter::empty;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tar::Archive;

/// Ongoing (RLS) builds might interfere with a precise time comparison.
const SLIDE: f64 = 60.;

/// Like the `duct` `cmd!` but also prints the command into the standard error stream.
macro_rules! ecmd {
    ( $program:expr ) => {{
        eprintln!("$ {}", $program);
        cmd($program, empty::<String>())
            .stdout_to_stderr()
    }};
    ( @s $args: expr, $arg:expr ) => {$args.push(String::from($arg));};
    ( @i $args: expr, $iterable:expr ) => {for v in $iterable {ecmd! (@s $args, v)}};
    ( @a $args: expr, i $arg:expr ) => {ecmd! (@i $args, $arg);};
    ( @a $args: expr, i $arg:expr, $( $tail:tt )* ) => {ecmd! (@i $args, $arg); ecmd! (@a $args, $($tail)*);};
    ( @a $args: expr, $arg:expr ) => {ecmd! (@s $args, $arg);};
    ( @a $args: expr, $arg:expr, $( $tail:tt )* ) => {ecmd! (@s $args, $arg); ecmd! (@a $args, $($tail)*);};
    ( $program:expr, $( $args:tt )* ) => {{
        let mut args: Vec<String> = Vec::new();
        ecmd! (@a &mut args, $($args)*);
        eprintln!("$ {}{}", $program, show_args(&args));
        cmd($program, args)
            .stdout_to_stderr()
    }};
}

/// Returns `true` if the `target` is not as fresh as the `prerequisites`.
fn make(target: &AsRef<Path>, prerequisites: &[PathBuf]) -> bool {
    let target_lm = last_modified_sec(target).expect("!last_modified") as u64;
    if target_lm == 0 {
        return true;
    }
    let mut prerequisites_lm = 0;
    for path in prerequisites {
        prerequisites_lm = max(
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

    // NB: curve25519.h and cJSON.h are needed to parse LP_include.h.
    bindgen(
        vec![
            "../../includes/curve25519.h".into(),
            "../../includes/cJSON.h".into(),
            "../../iguana/exchanges/LP_include.h".into(),
        ],
        c_headers.join("LP_include.rs"),
        [
            // functions
            "cJSON_Parse",
            "cJSON_GetErrorPtr",
            "cJSON_Delete",
            "cJSON_GetArraySize",
            "j64bits",
            "jadd",
            "jarray",
            "jitem",
            "jint",
            "juint",
            "jstr",
            "jdouble",
            "jobj",
            "jprint",
            "free_json",
            "jaddstr",
            "unstringify",
            "jaddnum",
            "LP_NXT_redeems",
            "LPinit",
            "LP_addpeer",
            "LP_peer_recv",
            "LP_initpublicaddr",
            "LP_ports",
            "LP_rpcport",
            "unbuffered_output_support",
            "calc_crc32",
            "LP_userpass",
            "LP_mutex_init",
            "LP_tradebots_timeslice",
            "stats_JSON",
            "LP_priceinfofind",
            "prices_loop",
            "LP_portfolio",
            "LP_coinadd_",
            "LP_priceinfoadd",
            "LP_dPoW_request",
            "LP_conflicts_find",
            "LP_coinjson",
            "LP_portfolio_trade",
            "LP_portfolio_order",
            "LP_pricesparse",
            "LP_ticker",
            "LP_queuecommand",
            "LP_CMCbtcprice",
            "LP_fundvalue",
            "LP_coinsearch",
            "LP_instantdex_deposit",
            "LP_mypriceset",
            "LP_pricepings",
            "LP_autopriceset",
            "LP_alicequery_clear",
            "LP_txfees",
            "LP_address_minmax",
            "LP_fomoprice",
            "LP_quoteinfoinit",
            "LP_quotedestinfo",
            "gen_quote_uuid",
            "decode_hex",
            "LP_aliceid_calc",
            "LP_rand",
            "LP_query",
            "LP_quotejson",
            "LP_mpnet_send",
            "LP_recent_swaps",
            "LP_address",
            "LP_address_utxo_ptrs",
            "LP_command_process",
            "LP_balances",
            "LP_KMDvalue",
            "LP_quoteparse",
            "LP_requestinit",
            "LP_tradecommand_log",
            "bits256_str", // cf. `impl fmt::Display for bits256`
            "vcalc_sha256",
            "calc_rmd160_sha256",
            "bitcoin_address",
            "bitcoin_pubkey33",
            "LP_alice_eligible",
            "LP_quotecmp",
            "LP_instantdex_proofcheck",
            "LP_myprice",
            "LP_pricecache",
            "LP_pricevalid",
            "LP_pubkeyadd",
            "LP_pubkeyfind",
            "LP_pubkey_sigcheck",
            "LP_aliceid",
            "LP_quotereceived",
            "LP_dynamictrust",
            "LP_kmdvalue",
            "LP_trades_alicevalidate",
            "LP_failedmsg",
            "LP_quote_validate",
            "LP_availableset",
            "LP_closepeers",
            "LP_tradebot_pauseall",
            "LP_portfolio_reset",
            "LP_priceinfos_clear",
            "LP_privkeycalc",
            "LP_privkey_updates",
            "LP_privkey_init",
            "LP_privkey",
            "LP_importaddress",
            "LP_otheraddress",
            "LP_swapsfp_update",
            "LP_reserved_msg",
            "LP_unavailableset",
            "LP_trades_pricevalidate",
            "LP_allocated",
            "LP_basesatoshis",
            "LP_trades_bobprice",
            "LP_RTmetrics_blacklisted",
            "LP_getheight",
            "LP_reservation_check",
            "LP_nanobind",
            "LP_instantdex_txids",
            "LP_pendswap_add",
            "LP_price_sig",
            "LP_coin_curl_init",
        ]
        .iter(),
        // types
        [
            "_bits256",
            "cJSON",
            "iguana_info",
            "LP_utxoinfo",
            "LP_trade",
            "LP_swap_remember",
        ]
        .iter(),
        [
            // defines
            "bitcoind_RPC_inittime",
            "GLOBAL_DBDIR",
            "DOCKERFLAG",
            "USERHOME",
            "LP_profitratio",
            "LP_RPCPORT",
            "LP_MAXPRICEINFOS",
            "LP_showwif",
            "LP_coins",
            "LP_IS_ZCASHPROTOCOL",
            "LP_IS_BITCOINCASH",
            "LP_IS_BITCOINGOLD",
            "BOTS_BONDADDRESS",
            "LP_MIN_TXFEE",
            "IAMLP",
            "LP_gui",
            "LP_canbind",
            "LP_fixed_pairport",
            "LP_myipaddr",
            "LP_myipaddr_from_command_line",
            "LP_autoprices",
            "num_LP_autorefs",
            "LP_STOP_RECEIVED",
            "IPC_ENDPOINT",
            "SPAWN_RPC",
            "LP_autorefs",
            "G",
            "LP_mypubsock",
            "LP_mypullsock",
            "LP_mypeer",
            "RPC_port",
            "LP_ORDERBOOK_DURATION",
            "LP_AUTOTRADE_TIMEOUT",
            "LP_RESERVETIME",
            "Alice_expiration",
            "LP_Alicequery",
            "LP_Alicemaxprice",
            "LP_Alicedestpubkey",
            "GTCorders",
            "LP_QUEUE_COMMAND",
            "LP_RTcount",
            "LP_swapscount",
            "LP_REQUEST",
            "LP_RESERVED",
            "LP_CONNECT",
            "LP_CONNECTED",
            "LP_Alicereserved",
            "dstr",
            "INSTANTDEX_PUBKEY",
        ]
        .iter(),
    );

    bindgen(
        vec!["../../crypto777/OS_portable.h".into()],
        c_headers.join("OS_portable.rs"),
        [
            // functions
            "OS_init",
            "OS_ensure_directory",
            "OS_compatible_path",
            "calc_ipbits",
        ]
        .iter(),
        empty(), // types
        empty(), // defines
    );
    bindgen(
        vec!["../../crypto777/nanosrc/nn.h".into()],
        c_headers.join("nn.rs"),
        [
            "nn_bind",
            "nn_connect",
            "nn_close",
            "nn_errno",
            "nn_freemsg",
            "nn_recv",
            "nn_setsockopt",
            "nn_send",
            "nn_socket",
            "nn_strerror",
        ]
        .iter(),
        empty(),
        [
            "AF_SP",
            "NN_PAIR",
            "NN_PUB",
            "NN_SOL_SOCKET",
            "NN_SNDTIMEO",
            "NN_MSG",
        ]
        .iter(),
    );
}

/// The build script will usually help us by putting the MarketMaker version
/// into the "MM_VERSION" environment or the "MM_VERSION" file.
/// If neither is there then we're probably in a non-released, local development branch
/// (we're using the "UNKNOWN" version designator then).
/// This function ensures that we have the "MM_VERSION" variable during the build.
fn mm_version() -> String {
    if let Some(have) = option_env!("MM_VERSION") {
        // The variable is already there.
        return have.into();
    }

    // Try to load the variable from the file.
    let mut buf;
    let version = if let Ok(mut file) = fs::File::open("../../MM_VERSION") {
        buf = String::new();
        unwrap!(file.read_to_string(&mut buf), "Can't read from MM_VERSION");
        buf.trim()
    } else {
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

/// See if we have the required libraries.
#[cfg(windows)]
fn windows_requirements() {
    use std::ffi::OsString;
    use std::mem::uninitialized;
    use std::os::windows::ffi::OsStringExt;
    use std::path::Path;
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
        let clang_v = cmd!("clang", "-v")
            .stderr_to_stdout()
            .read()
            .unwrap_or(Default::default());
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
    ) -> Box<Future<Item = (), Error = ()> + Send> {
        Box::new(client.request(request) .then(move |res| -> Box<Future<Item=(), Error=()> + Send> {
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
fn in_place(path: &AsRef<Path>, update: &mut dyn FnMut(Vec<u8>) -> Vec<u8>) {
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

/// Disable specific optional dependencies in CMakeLists.txt.
fn cmake_opt_out(path: &AsRef<Path>, dependencies: &[&str]) {
    in_place(path, &mut |mut clists| {
        for dep in dependencies {
            let exp = unwrap!(regex::bytes::Regex::new(
                &fomat! (r"(?xm) ^ [\t ]*? find_public_dependency\(" (regex::escape (dep)) r"\) $")
            ));
            clists = exp.replace_all(&clists, b"# $0" as &[u8]).into();
        }
        clists
    })
}

#[derive(Debug)]
struct IosClangOps {
    /// iPhone SDK (iPhoneOS for arm64, iPhoneSimulator for x86_64)
    sysroot: &'static str,
    /// "arm64", "x86_64".
    arch: &'static str,
    /// Identifies the corresponding clang options defined in "user-config.jam".
    b2_toolset: &'static str,
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
                    b2_toolset: "darwin-iphone"
                }),
                "x86_64-apple-ios" => Some(IosClangOps {
                    sysroot: "/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk",
                    arch: "x86_64",
                    b2_toolset: "darwin-iphonesim"
                }),
                //"armv7-apple-ios" => "armv7", 32-bit
                //"armv7s-apple-ios" => "armv7s", 32-bit
                _ => None,
            },
            _ => None,
        }
    }
    fn cc(&self, plus_plus: bool) -> cc::Build {
        let mut cc = cc::Build::new();
        if self.is_android_cross() {
            cc.compile(if plus_plus {
                // TODO: use clang++ if it is there in the NDK,
                // in order for `cc::Build` to match the compiler (GCC is a link to Clang in the NDK).
                "/android-ndk/bin/arm-linux-androideabi-g++"
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
            cc.flag("-miphoneos-version-min=11.0"); // 64-bit.
            cc.flag("-mios-simulator-version-min=11.0");
            cc.flag("-DIPHONEOS_DEPLOYMENT_TARGET=11.0");
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

/// Downloads and builds from bz2 sources.  
/// Targeted at Unix and Mac (we're unlikely to have bzip2 on a typical Windows).  
/// Tailored to work with Android `cross`-compilation.  
fn build_boost_bz2() -> PathBuf {
    let target = Target::load();
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
        if !out_dir.join("boost_1_68_0.tar.bz2").exists() {
            hget(
                "https://dl.bintray.com/boostorg/release/1.68.0/source/boost_1_68_0.tar.bz2",
                out_dir.join("boost_1_68_0.tar.bz2.tmp"),
            );
            unwrap!(fs::rename(
                out_dir.join("boost_1_68_0.tar.bz2.tmp"),
                out_dir.join("boost_1_68_0.tar.bz2")
            ));
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
                || pathˢ == "boost-build.jam"
                || pathˢ == "boostcpp.jam"
                || pathˢ == "boost/assert.hpp"
                || pathˢ == "boost/aligned_storage.hpp"
                || pathˢ == "boost/array.hpp"
                || pathˢ.starts_with("boost/asio/")
                || pathˢ.starts_with("boost/blank")
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
                || pathˢ.starts_with("boost/functional")
                || pathˢ.starts_with("boost/iterator/")
                || pathˢ.starts_with("boost/intrusive")
                || pathˢ.starts_with("boost/integer")
                || pathˢ.starts_with("boost/io")
                || pathˢ.starts_with("boost/lexical_cast")
                || pathˢ == "boost/limits.hpp"
                || pathˢ.starts_with("boost/mpl/")
                || pathˢ.starts_with("boost/math")
                || pathˢ.starts_with("boost/move")
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
                || pathˢ == "boost/static_assert.hpp"
                || pathˢ == "boost/shared_ptr.hpp"
                || pathˢ == "boost/shared_array.hpp"
                || pathˢ == "boost/swap.hpp"
                || pathˢ.starts_with("boost/type_traits")
                || pathˢ.starts_with("boost/type_index")
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
        let _ = fs::remove_file(out_dir.join("boost_1_68_0.tar.bz2"));
    }

    let b2 = boost.join("b2");
    if !b2.exists() {
        unwrap!(ecmd!("/bin/sh", "bootstrap.sh").dir(&boost).run());
        assert!(b2.exists());
    }

    if target.is_ios() {
        if 1 == 1 {
            return boost;
        }
        // Our hope is that the separate Boost compilation will be no longer necessary
        // as libtorrent will be building and linking in the necessary parts of Boost on its own.
        // But we should confirm that mm2 works on iOS before removing the Boost compilation here.

        let cops = unwrap!(target.ios_clang_ops());
        assert!(Path::new(cops.sysroot).is_dir());
        // NB: We're passing options for the "darwin" toolset defined in "tools/build/src/tools/darwin.jam":
        //
        //     rule init ( version ? : command * : options * : requirement * )
        let user_config_jamˢ = fomat!(
            "using darwin\n"
            ": 11.0~iphone \n"  // `version`
            ": /Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang++"
            " -target "(target)" --sysroot "(cops.sysroot)" -arch "(cops.arch)" -stdlib=libc++ \n"  // `command`
            ": <striper> \n"  // `options`
            ": <architecture>arm <target-os>iphone <address-model>64 \n"  // `requirement`
            ";\n"
        );
        let user_config_jamᵖ = boost.join("tools/build/src/user-config.jam");
        let mut user_config_jamᶠ = unwrap!(fs::File::create(&user_config_jamᵖ));
        unwrap!(user_config_jamᶠ.write_all(user_config_jamˢ.as_bytes()));
        drop(user_config_jamᶠ);
        epintln!("Created "[user_config_jamᵖ]":\n"(user_config_jamˢ));

        unwrap!(ecmd!(
            "/bin/sh",
            "-c",
            fomat!(
                "./b2 release link=static cxxflags=-fPIC cxxstd=11 toolset=darwin "
                "address-model=64 "
                "target-os=iphone "
                "architecture=arm "
                "define=BOOST_ERROR_CODE_HEADER_ONLY "
                "install --with-date_time --with-system --prefix=../boost "
                "| grep --line-buffered -v 'common.copy ../boost/include/'"
            )
        )
        .dir(&boost)
        .unchecked()
        .run());
    } else {
        // TODO: Use the "tools/build/src/user-config.jam" instead of injecting the NDK g++ into the PATH.
        let bin = out_dir.join("bin");
        let bin = unwrap!(bin.to_str());
        let _ = fs::create_dir(&bin);
        let tmp_gpp = fomat!((bin) "/g++");
        if target.is_android_cross() && !Path::new(&tmp_gpp).exists() {
            unwrap!(ecmd!(
                "ln",
                "-sf",
                "/android-ndk/bin/arm-linux-androideabi-g++",
                tmp_gpp
            )
            .run());
        }

        unwrap!(ecmd!(
            "/bin/sh",
            "-c",
            "./b2 release address-model=64 link=static cxxflags=-fPIC cxxstd=11 \
             define=BOOST_ERROR_CODE_HEADER_ONLY \
             install --with-date_time --with-system --prefix=../boost"
        )
        .env("PATH", fomat!((bin) ":"(unwrap!(var("PATH")))))
        .dir(&boost)
        .unchecked()
        .run());
    }

    assert!(boost_system.exists());
    assert!(prefix.is_dir());

    let _ = fs::remove_dir_all(boost);

    prefix
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

/// Downloads and builds libtorrent.  
/// Only for UNIX and macOS as of now (Windows needs a different approach to Boost).  
/// Tailored to work with Android `cross`-compilation.
///
/// * `boost` - Used when there is a STAGE build of Boost under that path.
///             If absent then we'll be trying to link against the system version of Boost (not recommended).
fn build_libtorrent(boost: Option<&Path>) -> (PathBuf, PathBuf) {
    let target = Target::load();
    let out_dir = out_dir();

    // Released tgz version fails to link for iOS due to https://github.com/arvidn/libtorrent/pull/3629,
    // should get a fresh Git version instead.

    let rasterbar = out_dir.join("libtorrent-rasterbar-1.2.0");
    epintln!("libtorrent at "[rasterbar]);
    if !rasterbar.exists() {
        unwrap!(
            ecmd!(
                "git",
                "clone",
                "--depth=1",
                "https://github.com/arvidn/libtorrent.git",
                "-b",
                "RC_1_2"
            )
            .dir(&out_dir)
            .run(),
            "Error git-cloning libtorrent"
        );
        let libtorrent = out_dir.join("libtorrent");
        assert!(libtorrent.is_dir());
        unwrap!(fs::rename(libtorrent, &rasterbar));
    }

    if let Target::iOS(ref targetᴱ) = target {
        // This is the latest version of the build. It doesn't compile Boost separately
        // but rather allows the libtorrent to compile it
        // "you probably want to just build libtorrent and have it build boost
        //  (otherwise you'll end up building the boost dependencies twice)"
        //  - https://github.com/arvidn/libtorrent/issues/26#issuecomment-121478708
        // After field-testing on iOS we should probably refactor
        // and merge the rest of the OS builds into this one.

        let boost = unwrap!(boost);
        let user_config_jamˢ = slurp(&root().join("mm2src/common/ios/user-config.jam"));
        let user_config_jamᵖ = boost.join("tools/build/src/user-config.jam");
        epintln!("build_libtorrent] Creating "[user_config_jamᵖ]"…");
        let mut user_config_jamᶠ = unwrap!(fs::File::create(&user_config_jamᵖ));
        unwrap!(user_config_jamᶠ.write_all(&user_config_jamˢ));
        drop(user_config_jamᶠ);

        let cops = unwrap!(target.ios_clang_ops());
        let b2 = fomat!(
            "b2 -j4 -d+2 release"
            " link=static deprecated-functions=off debug-symbols=off"
            " dht=on encryption=on crypto=built-in iconv=off i2p=off"
            " cxxflags=-DBOOST_ERROR_CODE_HEADER_ONLY=1"
            " toolset="(cops.b2_toolset)
        );
        epintln!("build_libtorrent] $ "(b2));
        unwrap!(cmd!("/bin/sh", "-c", b2)
            .env(
                "PATH",
                format!("{}:{}", unwrap!(boost.to_str()), unwrap!(var("PATH")))
            )
            .env("BOOST_BUILD_PATH", boost.join(r"tools/build"))
            .env_remove("BOOST_ROOT") // cf. https://stackoverflow.com/a/55141466/257568
            .dir(&rasterbar)
            .run());

        let a_rel = fomat!(
        "bin/darwin-iphone"
        if targetᴱ == "x86_64-apple-ios" {"sim"}
        "/release/deprecated-functions-off/i2p-off/iconv-off/link-static/threading-multi/libtorrent.a"
        );
        let a = rasterbar.join(a_rel);
        assert!(a.is_file());

        let include = rasterbar.join("include");
        assert!(include.is_dir());

        return (a, include);
    }

    if !rasterbar.join("Makefile").exists() {
        let with_boost = if let Some(boost) = boost {
            fomat!("--with-boost="(unwrap!(boost.to_str())))
        } else {
            fomat!("--with-boost")
        };

        let e = if target.is_android_cross() {
            ecmd!(
                "/bin/sh",
                "configure",
                "--host=armv7-linux-androideabi",
                "--with-openssl=/openssl",
                "--without-libiconv",
                "--disable-encryption",
                "--disable-shared",
                "--enable-static",
                "--with-pic",
                with_boost
            )
            .env("CXX", "/android-ndk/bin/clang++")
            .env("CC", "/android-ndk/bin/clang")
            .env("CXXPP", "/android-ndk/bin/arm-linux-androideabi-cpp")
        } else {
            ecmd!(
                "/bin/sh",
                "configure",
                "--without-openssl",
                "--without-libiconv",
                "--disable-encryption",
                "--disable-shared",
                "--enable-static",
                "--with-pic",
                with_boost
            )
        };

        let e = e.env("CPPFLAGS", "-DBOOST_ERROR_CODE_HEADER_ONLY=1");

        unwrap!(e.dir(&rasterbar).run());
        assert!(rasterbar.join("Makefile").exists());
    }

    let patched_flag = rasterbar.join("patched.flag");
    if !patched_flag.exists() {
        // Patch libtorrent to work with the older version of clang we have in the `cross`.

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

    unwrap!(ecmd!("make", "-j2").dir(&rasterbar).run());

    let a = rasterbar.join("src/.libs/libtorrent-rasterbar.a");
    assert!(a.exists());

    if target.is_android_cross() {
        unwrap!(ecmd!(
            "/android-ndk/bin/arm-linux-androideabi-strip",
            "--strip-debug",
            unwrap!(a.to_str())
        )
        .run());
    } else if target.is_mac() || target.is_ios() {
        unwrap!(ecmd!("strip", "-S", unwrap!(a.to_str())).run());
    } else {
        // 85 MiB reduction in mm2 binary size on Linux.
        // NB: We can't do a full strip (one without --strip-debug) as it leads to undefined symbol link errors.
        unwrap!(ecmd!("strip", "--strip-debug", unwrap!(a.to_str())).run());
    }

    let include = rasterbar.join("include");
    assert!(include.exists());

    let li_from = rasterbar.join("LICENSE");
    let li_to = if target.is_android_cross() {
        "/target/debug/LICENSE.libtorrent-rasterbar".into()
    } else {
        root().join("target/debug/LICENSE.libtorrent-rasterbar")
    };
    unwrap!(
        fs::copy(&li_from, &li_to),
        "Can't copy the rasterbar license from {:?} to {:?}",
        li_from,
        li_to
    );

    (a, include)
}

fn libtorrent() {
    // NB: Distributions should have a copy of https://github.com/arvidn/libtorrent/blob/master/LICENSE.

    let target = Target::load();

    if cfg!(windows) {
        // NB: The "marketmaker_depends" folder is cached in the AppVeyour build,
        // allowing us to build Boost only once.
        // NB: The Windows build is different from `fn build_libtorrent` in that we're
        // 1) Using the cached marketmaker_depends.
        //    (It's only cached after a successful build, cf. https://www.appveyor.com/docs/build-cache/#saving-cache-for-failed-build).
        // 2) Using ".bat" files and "cmd /c" shims.
        // 3) Using "b2" and pointing it at the Boost sources,
        // as recommended at https://github.com/arvidn/libtorrent/blob/master/docs/building.rst#building-with-bbv2,
        // in hopes of avoiding the otherwise troubling Boost linking concerns.
        //
        // We can try building the Windows libtorrent with CMake.
        // Though as of now having both build systems tested gives as a leeway in case one of them breaks.
        let mmd = root().join("marketmaker_depends");
        let _ = fs::create_dir(&mmd);

        let boost = mmd.join("boost_1_68_0");
        if boost.exists() {
            // Cache maintenance.
            let _ = fs::remove_file(mmd.join("boost_1_68_0.zip"));
            let _ = fs::remove_dir_all(boost.join("doc")); // 80 MiB.
            let _ = fs::remove_dir_all(boost.join("libs")); // 358 MiB, documentation and examples.
            let _ = fs::remove_dir_all(boost.join("more"));
        } else {
            // [Download and] unpack Boost.
            if !mmd.join("boost_1_68_0.zip").exists() {
                hget(
                    "https://dl.bintray.com/boostorg/release/1.68.0/source/boost_1_68_0.zip",
                    mmd.join("boost_1_68_0.zip.tmp"),
                );
                unwrap!(fs::rename(
                    mmd.join("boost_1_68_0.zip.tmp"),
                    mmd.join("boost_1_68_0.zip")
                ));
            }

            // TODO: Unzip without requiring the user to install unzip.
            unwrap!(
                ecmd!("unzip", "boost_1_68_0.zip").dir(&mmd).run(),
                "Can't unzip Boost. Missing http://gnuwin32.sourceforge.net/packages/unzip.htm ?"
            );
            assert!(boost.exists());
            let _ = fs::remove_file(mmd.join("boost_1_68_0.zip"));
        }

        let b2 = boost.join("b2.exe");
        if !b2.exists() {
            unwrap!(ecmd!("cmd", "/c", "bootstrap.bat").dir(&boost).run());
            assert!(b2.exists());
        }

        let boost_system = boost.join("stage/lib/libboost_system-vc141-mt-x64-1_68.lib");
        if !boost_system.exists() {
            unwrap!(ecmd!(
                // For some weird reason this particular executable won't start without the "cmd /c"
                // even though some other executables (copied into the same folder) are working NP.
                "cmd",
                "/c",
                "b2.exe",
                "release",
                "toolset=msvc-14.1",
                "address-model=64",
                // Not quite static? cf. https://stackoverflow.com/a/14368257/257568
                "link=static",
                "stage",
                "--with-date_time",
                "--with-system"
            )
            .dir(&boost)
            .unchecked()
            .run());
            assert!(boost_system.exists());
        }

        let rasterbar = mmd.join("libtorrent-rasterbar-1.2.0");
        if rasterbar.exists() {
            // Cache maintenance.
            let _ = fs::remove_file(mmd.join("libtorrent-rasterbar-1.2.0.tar.gz"));
            let _ = fs::remove_dir_all(rasterbar.join("docs"));
        } else {
            // [Download and] unpack.
            if !mmd.join("libtorrent-rasterbar-1.2.0.tar.gz").exists() {
                hget (
                    "https://github.com/arvidn/libtorrent/releases/download/libtorrent_1_2_0/libtorrent-rasterbar-1.2.0.tar.gz",
                    mmd.join ("libtorrent-rasterbar-1.2.0.tar.gz.tmp")
                );
                unwrap!(fs::rename(
                    mmd.join("libtorrent-rasterbar-1.2.0.tar.gz.tmp"),
                    mmd.join("libtorrent-rasterbar-1.2.0.tar.gz")
                ));
            }

            unwrap!(ecmd!("tar", "-xzf", "libtorrent-rasterbar-1.2.0.tar.gz")
                .dir(&mmd)
                .run());
            assert!(rasterbar.exists());
            let _ = fs::remove_file(mmd.join("libtorrent-rasterbar-1.2.0.tar.gz"));

            cmake_opt_out(
                &rasterbar.join("CMakeLists.txt"),
                &["Iconv", "OpenSSL", "LibGcrypt"],
            );
        }

        let lt = rasterbar.join(
            r"bin\msvc-14.1\release\address-model-64\iconv-off\link-static\threading-multi\libtorrent.lib",              
        );
        if !lt.exists() {
            // cf. tools\build\src\tools\msvc.jam
            unwrap!(ecmd!(
                "cmd",
                "/c",
                fomat!(
                    "b2 release "
                    "include="(unwrap!(boost.to_str()))" "
                    "toolset=msvc-14.1 address-model=64 link=static dht=on"
                    " iconv=off"
                    " encryption=on crypto=built-in"
                    " debug-symbols=off"
                )
            )
            .env(
                "PATH",
                format!("{};{}", unwrap!(boost.to_str()), unwrap!(var("PATH")))
            )
            .env("BOOST_BUILD_PATH", boost.join(r"tools\build"))
            .env_remove("BOOST_ROOT") // cf. https://stackoverflow.com/a/55141466/257568
            .dir(&rasterbar)
            .run());
            assert!(lt.exists());
        }

        let lm_dht = unwrap!(last_modified_sec(&"dht.cc"), "Can't stat dht.cc");
        let out_dir = unwrap!(var("OUT_DIR"), "!OUT_DIR");
        let lib_path = Path::new(&out_dir).join("libdht.a");
        let lm_lib = last_modified_sec(&lib_path).unwrap_or(0.);
        if lm_dht >= lm_lib - SLIDE {
            cc::Build::new()
                .file("dht.cc")
                .warnings(true)
                .include(rasterbar.join("include"))
                .include(boost)
                // https://docs.microsoft.com/en-us/cpp/porting/modifying-winver-and-win32-winnt?view=vs-2017
                .define("_WIN32_WINNT", "0x0600")
                // cf. https://stackoverflow.com/questions/4573536/ehsc-vc-eha-synchronous-vs-asynchronous-exception-handling
                .flag("/EHsc")
                // https://stackoverflow.com/questions/7582394/strdup-or-strdup
                .flag("-D_CRT_NONSTDC_NO_DEPRECATE")
                .compile("dht");
        }
        println!("cargo:rustc-link-lib=static=dht");
        println!("cargo:rustc-link-search=native={}", out_dir);

        println!("cargo:rustc-link-lib=static=libtorrent");
        println!(
            "cargo:rustc-link-search=native={}",
            unwrap!(unwrap!(lt.parent()).to_str())
        );

        println!("cargo:rustc-link-lib=static=libboost_system-vc141-mt-x64-1_68");
        println!("cargo:rustc-link-lib=static=libboost_date_time-vc141-mt-x64-1_68");
        println!(
            "cargo:rustc-link-search=native={}",
            unwrap!(unwrap!(boost_system.parent()).to_str())
        );

        println!("cargo:rustc-link-lib=iphlpapi"); // NotifyAddrChange.
    } else if cfg!(target_os = "macos") && !target.is_ios() {
        // NB: Homebrew's version of libtorrent-rasterbar (1.1.10) is currently too old.

        let boost_system_mt = Path::new("/usr/local/lib/libboost_system-mt.a");
        if !boost_system_mt.exists() {
            unwrap!(
                ecmd!("brew", "install", "boost").run(),
                "Can't brew install boost"
            );
            assert!(boost_system_mt.exists());
        }

        let (lt_a, lt_include) = build_libtorrent(None);
        println!("cargo:rustc-link-lib=static=torrent-rasterbar");
        println!(
            "cargo:rustc-link-search=native={}",
            unwrap!(unwrap!(lt_a.parent()).to_str())
        );
        println!("cargo:rustc-link-lib=c++");
        println!("cargo:rustc-link-lib=boost_system-mt");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=SystemConfiguration");

        let lm_dht = unwrap!(last_modified_sec(&"dht.cc"), "Can't stat dht.cc");
        let out_dir = unwrap!(var("OUT_DIR"), "!OUT_DIR");
        let lib_path = Path::new(&out_dir).join("libdht.a");
        let lm_lib = last_modified_sec(&lib_path).unwrap_or(0.);
        if lm_dht >= lm_lib - SLIDE {
            cc::Build::new()
                .file("dht.cc")
                .warnings(true)
                .flag("-std=c++11")
                .include(lt_include)
                .include(r"/usr/local/Cellar/boost/1.68.0/include/")
                .compile("dht");
        }
        println!("cargo:rustc-link-lib=static=dht");
        println!("cargo:rustc-link-search=native={}", out_dir);
    } else {
        let boost = build_boost_bz2();
        let (lt_a, lt_include) = build_libtorrent(Some(&boost));
        println!("cargo:rustc-link-lib=static={}", {
            let name = unwrap!(unwrap!(lt_a.file_stem()).to_str());
            &name[3..]
        });
        println!(
            "cargo:rustc-link-search=native={}",
            unwrap!(unwrap!(lt_a.parent()).to_str())
        );

        if !target.is_ios() {
            // NB: We should prefer linking boost in AFTER libtorrent,
            // cf. "Linking boost_system last fixed the issue for me" in https://stackoverflow.com/a/30877725/257568.
            println!("cargo:rustc-link-lib=static=boost_system");
            println!(
                "cargo:rustc-link-search=native={}",
                unwrap!(boost.join("lib").to_str())
            );
        }

        let lm_dht = unwrap!(last_modified_sec(&"dht.cc"), "Can't stat dht.cc");
        let out_dir = unwrap!(var("OUT_DIR"), "!OUT_DIR");
        let lib_path = Path::new(&out_dir).join("libdht.a");
        let lm_lib = last_modified_sec(&lib_path).unwrap_or(0.);
        let boost_inc = if boost.join("include/boost/version.hpp").exists() {
            boost.join("include")
        } else {
            assert!(boost.join("boost/version.hpp").exists());
            boost.clone()
        };
        if lm_dht >= lm_lib - SLIDE {
            let mut cc = target.cc(true);
            if target.is_ios() {
                // Defines spied in libtorrent (with "b2 -d+2").
                cc.flag("-fexceptions");
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
                cc.flag("-D_FILE_OFFSET_BITS=64");
                cc.flag("-D_WIN32_WINNT=0x0600");
                // Fixes the «Undefined symbols… "boost::system::detail::generic_category_ncx()"».
                cc.flag("-DBOOST_ERROR_CODE_HEADER_ONLY=1");
            } else {
                cc.flag("-DBOOST_ERROR_CODE_HEADER_ONLY=1");
                cc.flag("-DBOOST_ASIO_HAS_STD_CHRONO=1");
                cc.flag("-DBOOST_EXCEPTION_DISABLE=1");
                cc.flag("-DBOOST_ASIO_ENABLE_CANCELIO=1");
            }
            cc.file("dht.cc")
                .warnings(true)
                // Mismatch between the libtorrent and the dht.cc flags
                // might produce weird "undefined reference" link errors.
                // Building libtorrent with "-d+2" passed to "b2" should show the actual defines.
                .flag("-std=c++11")
                .opt_level(2)
                .flag("-ftemplate-depth=512")
                .flag("-fvisibility=hidden")
                .flag("-fvisibility-inlines-hidden")
                .pic(true)
                .include(lt_include)
                .include(boost_inc)
                .compile("dht");
        }
        println!("cargo:rustc-link-lib=static=dht");
        println!("cargo:rustc-link-search=native={}", out_dir);

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
        rabs("iguana/mini-gmp.c"),
        rabs("iguana/groestl.c"),
        rabs("iguana/segwit_addr.c"),
        rabs("iguana/keccak.c"),
    ];
    /// A list of nanomsg-1.1.5 source files known to cross-compile for Android (formerly android/nanomsg.mk).
    static ref LIBNANOMSG_115_ANDROID_SRC: Vec<&'static str> =
        "utils/efd.c core/sock.c core/poll.c                         \
         core/symbol.c core/ep.c core/pipe.c                         \
         core/sockbase.c core/global.c devices/device.c              \
         transports/inproc/ins.c transports/inproc/inproc.c          \
         transports/inproc/cinproc.c transports/inproc/binproc.c     \
         transports/inproc/sinproc.c transports/inproc/msgqueue.c    \
         transports/utils/dns.c transports/utils/literal.c           \
         transports/utils/streamhdr.c transports/utils/backoff.c     \
         transports/utils/iface.c transports/utils/port.c            \
         transports/tcp/tcp.c transports/tcp/stcp.c                  \
         transports/tcp/ctcp.c transports/tcp/atcp.c                 \
         transports/tcp/btcp.c transports/ipc/aipc.c                 \
         transports/ipc/bipc.c transports/ipc/cipc.c                 \
         transports/ipc/ipc.c transports/ipc/sipc.c                  \
         transports/ws/ws.c                                          \
         transports/ws/aws.c transports/ws/bws.c                     \
         transports/ws/cws.c transports/ws/sha1.c                    \
         transports/ws/sws.c transports/ws/ws_handshake.c            \
         transports/utils/base64.c                                   \
         utils/strcasestr.c utils/strncasecmp.c                      \
         protocols/survey/xrespondent.c                              \
         protocols/survey/surveyor.c protocols/survey/xsurveyor.c    \
         protocols/survey/respondent.c protocols/pair/pair.c         \
         protocols/pair/xpair.c protocols/utils/dist.c               \
         protocols/utils/priolist.c protocols/utils/fq.c             \
         protocols/utils/excl.c protocols/utils/lb.c                 \
         protocols/bus/xbus.c protocols/bus/bus.c                    \
         protocols/pipeline/xpull.c protocols/pipeline/push.c        \
         protocols/pipeline/pull.c protocols/pipeline/xpush.c        \
         protocols/reqrep/rep.c protocols/reqrep/req.c               \
         protocols/reqrep/xrep.c protocols/reqrep/task.c             \
         protocols/reqrep/xreq.c protocols/pubsub/sub.c              \
         protocols/pubsub/xpub.c protocols/pubsub/xsub.c             \
         protocols/pubsub/trie.c protocols/pubsub/pub.c              \
         aio/worker.c aio/fsm.c aio/ctx.c aio/usock.c                \
         aio/poller.c aio/pool.c aio/timerset.c                      \
         aio/timer.c utils/err.c utils/thread.c                      \
         utils/closefd.c utils/atomic.c utils/list.c                 \
         utils/stopwatch.c utils/random.c utils/wire.c               \
         utils/mutex.c utils/msg.c utils/clock.c                     \
         utils/queue.c utils/chunk.c                                 \
         utils/hash.c utils/alloc.c                                  \
         utils/sleep.c utils/chunkref.c utils/sem.c                  \
         utils/condvar.c utils/once.c"
            .split_ascii_whitespace()
            .collect();
}

fn manual_nanomsg_build(_root: &Path, out_dir: &Path, target: &Target) {
    let nanomsg = if target.is_ios() {
        out_dir.join("nanomsg.ios")
    } else {
        out_dir.join("nanomsg-1.1.5")
    };
    epintln!("nanomsg at "[nanomsg]);

    let libnanomsg_a = out_dir.join("libnanomsg.a");
    if !libnanomsg_a.exists() {
        if !nanomsg.exists() && !target.is_ios() {
            let nanomsg_tgz = out_dir.join("nanomsg.tgz");
            if !nanomsg_tgz.exists() {
                hget(
                    "https://github.com/nanomsg/nanomsg/archive/1.1.5.tar.gz",
                    nanomsg_tgz.clone(),
                );
                assert!(nanomsg_tgz.exists());
            }
            unwrap!(ecmd!("tar", "-xzf", "nanomsg.tgz").dir(&out_dir).run());
            assert!(nanomsg.exists());
        } else if !nanomsg.exists() && target.is_ios() {
            // NB: This is a port listed at https://nanomsg.org/documentation.html
            // and a cursory search has confirmed that this is what people use on iOS.
            unwrap!(ecmd!(
                "git",
                "clone",
                "--depth=1",
                "https://github.com/reqshark/nanomsg.ios.git"
            )
            .dir(&out_dir)
            .run());
            assert!(nanomsg.exists());
        }

        let mut cc = target.cc(false);
        cc.debug(false);
        cc.opt_level(2);
        cc.flag("-fPIC");
        if target.is_ios() {
            cc.include(nanomsg.join("utils")); // for `#include "attr.h"` to work
        } else {
            cc.flag("-DNN_HAVE_SEMAPHORE");
            cc.flag("-DNN_HAVE_POLL");
            cc.flag("-DNN_HAVE_MSG_CONTROL");
            cc.flag("-DNN_HAVE_EVENTFD");
            cc.flag("-DNN_USE_EVENTFD");
            cc.flag("-DNN_USE_LITERAL_IFADDR");
            cc.flag("-DNN_USE_PO");
        }
        for src_path in LIBNANOMSG_115_ANDROID_SRC.iter() {
            cc.file(if target.is_ios() {
                if src_path.ends_with("/strcasestr.c")
                    || src_path.ends_with("/strncasecmp.c")
                    || src_path.ends_with("/condvar.c")
                    || src_path.ends_with("/once.c")
                {
                    continue;
                }
                nanomsg.join(src_path)
            } else {
                nanomsg.join("src").join(src_path)
            });
        }
        if target.is_ios() {
            cc.file(nanomsg.join("utils/glock.c"));
            cc.file(nanomsg.join("core/epbase.c"));
            cc.file(nanomsg.join("transports/tcpmux/tcpmux.c"));
            cc.file(nanomsg.join("transports/tcpmux/ctcpmux.c"));
            cc.file(nanomsg.join("transports/tcpmux/stcpmux.c"));
            cc.file(nanomsg.join("transports/tcpmux/btcpmux.c"));
            cc.file(nanomsg.join("transports/tcpmux/atcpmux.c"));
        }
        cc.compile("nanomsg");
        assert!(libnanomsg_a.exists());
    }
    println!("cargo:rustc-link-lib=static=nanomsg");
    println!(
        "cargo:rustc-link-search=native={}",
        path2s(unwrap!(libnanomsg_a.parent()))
    );
}

/// Build MM1 libraries without CMake, making cross-platform builds more transparent to us.
fn manual_mm1_build(target: Target) {
    let (root, out_dir) = (root(), out_dir());
    manual_nanomsg_build(&root, &out_dir, &target);

    let exchanges_build = out_dir.join("exchanges_build");
    epintln!("exchanges_build at "[exchanges_build]);

    let libexchanges_a = out_dir.join("libexchanges.a");
    if make(&libexchanges_a, &LIBEXCHANGES_SRC[..]) {
        let _ = fs::create_dir(&exchanges_build);
        let mut cc = target.cc(false);
        for p in LIBEXCHANGES_SRC.iter() {
            cc.file(p);
        }
        cc.include(rabs("crypto777"));
        cc.compile("exchanges");
        assert!(libexchanges_a.is_file());
    }
    println!("cargo:rustc-link-lib=static=exchanges");
    println!("cargo:rustc-link-search=native={}", path2s(&out_dir));

    // TODO: Rebuild the libraries when the C source code is updated.

    let secp256k1_build = out_dir.join("secp256k1_build");
    epintln!("secp256k1_build at "[secp256k1_build]);

    let libsecp256k1_a = out_dir.join("libsecp256k1.a");
    let secp256k1_src = [root.join("iguana/secp256k1/src/secp256k1.c")];
    if make(&libsecp256k1_a, &secp256k1_src[..]) {
        let mut cc = target.cc(false);
        cc.file(&secp256k1_src[0]);
        cc.compile("secp256k1");
        assert!(libsecp256k1_a.is_file());
    }
    println!("cargo:rustc-link-lib=static=secp256k1");

    let libjpeg_a = out_dir.join("libjpeg.a");
    let mut libjpeg_src: Vec<PathBuf> = unwrap!(globʳ("crypto777/jpeg/*.c"))
        .map(|p| unwrap!(p))
        .collect();
    libjpeg_src.push(root.join("crypto777/jpeg/unix/jmemname.c"));
    if make(&libjpeg_a, &libjpeg_src[..]) {
        let mut cc = target.cc(false);
        for p in &libjpeg_src {
            cc.file(p);
        }
        cc.compile("jpeg");
        assert!(libjpeg_a.is_file());
    }
    println!("cargo:rustc-link-lib=static=jpeg");

    let libcrypto777_a = out_dir.join("libcrypto777.a");
    let libcrypto777_src: Vec<PathBuf> = unwrap!(globʳ("crypto777/*.c"))
        .map(|p| unwrap!(p))
        .collect();
    if make(&libcrypto777_a, &libcrypto777_src[..]) {
        let mut cc = target.cc(false);
        for p in &libcrypto777_src {
            cc.file(p);
        }
        cc.compile("crypto777");
        assert!(libcrypto777_a.is_file());
    }
    println!("cargo:rustc-link-lib=static=crypto777");
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
    cmake_prep_args.push("-DETOMIC=ON".into());
    cmake_prep_args.push(format!("-DMM_VERSION={}", mm_version));
    cmake_prep_args.push("-DCMAKE_BUILD_TYPE=Debug".into());
    cmake_prep_args.push("..".into());
    eprintln!("$ cmake{}", show_args(&cmake_prep_args));
    unwrap!(
        cmd("cmake", cmake_prep_args)
            .env("PATH", cmake_path())
            .env("VERBOSE", "1")
            .dir(root().join("build"))
            .stdout_to_stderr() // NB: stderr is visible through "cargo build -vv".
            .run(),
        "!cmake"
    );

    let mut cmake_args: Vec<String> = vec![
        "--build".into(),
        ".".into(),
        "--target".into(),
        "marketmaker-lib".into(),
    ];
    if !cfg!(windows) {
        // Doesn't currently work on AppVeyor.
        cmake_args.push("-j".into());
        cmake_args.push(format!("{}", num_cpus::get()));
    }
    eprintln!("$ cmake{}", show_args(&cmake_args));
    unwrap!(
        cmd("cmake", cmake_args)
            .env("PATH", cmake_path())
            .dir(root().join("build"))
            .stdout_to_stderr() // NB: stderr is visible through "cargo build -vv".
            .run(),
        "!cmake"
    );

    println!("cargo:rustc-link-lib=static=marketmaker-lib");

    // Link in the libraries needed for MM1.

    println!("cargo:rustc-link-lib=static=libcrypto777");
    println!("cargo:rustc-link-lib=static=libjpeg");
    //Already linked from etomicrs->ethkey->eth-secp256k1//println!("cargo:rustc-link-lib=static=libsecp256k1");

    if cfg!(windows) {
        println!("cargo:rustc-link-search=native={}", path2s(rabs("x64")));
        // When building locally with CMake 3.12.0 on Windows the artefacts are created in the "Debug" folders:
        println!(
            "cargo:rustc-link-search=native={}",
            path2s(rabs("build/iguana/exchanges/Debug"))
        );
        println!(
            "cargo:rustc-link-search=native={}",
            path2s(rabs("build/crypto777/Debug"))
        );
        println!(
            "cargo:rustc-link-search=native={}",
            path2s(rabs("build/crypto777/jpeg/Debug"))
        );
    // https://stackoverflow.com/a/10234077/257568
    //println!(r"cargo:rustc-link-search=native=c:\Program Files (x86)\Microsoft Visual Studio\2017\BuildTools\VC\Tools\MSVC\14.14.26428\lib\x64");
    } else {
        println!(
            "cargo:rustc-link-search=native={}",
            path2s(rabs("build/iguana/exchanges"))
        );
        println!(
            "cargo:rustc-link-search=native={}",
            path2s(rabs("build/crypto777"))
        );
        println!(
            "cargo:rustc-link-search=native={}",
            path2s(rabs("build/crypto777/jpeg"))
        );
        println!(
            "cargo:rustc-link-search=native={}",
            path2s(rabs("build/nanomsg-build"))
        );
    }

    println!(
        "cargo:rustc-link-lib={}",
        if cfg!(windows) { "libcurl" } else { "curl" }
    );
    if cfg!(windows) {
        // https://sourceware.org/pthreads-win32/
        // ftp://sourceware.org/pub/pthreads-win32/prebuilt-dll-2-9-1-release/
        println!("cargo:rustc-link-lib=pthreadVC2");
        println!("cargo:rustc-link-lib=static=nanomsg");
        println!("cargo:rustc-link-lib=mswsock"); // For nanomsg.
        unwrap!(
            fs::copy(
                root().join("x64/pthreadVC2.dll"),
                root().join("target/debug/pthreadVC2.dll")
            ),
            "Can't copy pthreadVC2.dll"
        );
        unwrap!(
            fs::copy(
                root().join("x64/libcurl.dll"),
                root().join("target/debug/libcurl.dll")
            ),
            "Can't copy libcurl.dll"
        );
    } else {
        println!("cargo:rustc-link-lib=crypto");
        println!("cargo:rustc-link-lib=static=nanomsg");
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

    rerun_if_changed("iguana/exchanges/*.c");
    rerun_if_changed("iguana/secp256k1/src/*.c");
    rerun_if_changed("crypto777/*.c");
    rerun_if_changed("crypto777/jpeg/*.c");
    println!("cargo:rerun-if-changed={}", path2s(rabs("CMakeLists.txt")));

    // NB: Using `rerun-if-env-changed` disables the default dependency heuristics.
    // cf. https://github.com/rust-lang/cargo/issues/4587
    // We should avoid using it for now.

    // Rebuild when we change certain features.
    //println!("rerun-if-env-changed=CARGO_FEATURE_ETOMIC");
    //println!("rerun-if-env-changed=CARGO_FEATURE_NOP");

    windows_requirements();
    libtorrent();
    let mm_version = mm_version();
    build_c_code(&mm_version);
    generate_bindings();
}

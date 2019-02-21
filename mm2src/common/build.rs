// The script here will translate some of the C headers necessary for the gradual Rust port into the corresponding Rust files.
// Going to take the *whitelisting* approach, converting just the necessary definitions, in order to keep the builds fast.

// The script is experimentally formatted with `rustfmt`. Probably not going to use `rustfmt` for the rest of the project though.

// Bindgen requirements: https://rust-lang-nursery.github.io/rust-bindgen/requirements.html
//              Windows: https://github.com/rust-lang-nursery/rustup.rs/issues/1003#issuecomment-289825927
// On build.rs: https://doc.rust-lang.org/cargo/reference/build-scripts.html

#![feature(non_ascii_idents)]

extern crate bindgen;
extern crate cc;
extern crate duct;
#[macro_use]
extern crate fomat_macros;
extern crate futures;
extern crate futures_cpupool;
extern crate gstuff;
extern crate hyper;
extern crate hyper_rustls;
extern crate num_cpus;
extern crate regex;
#[macro_use]
extern crate unwrap;
extern crate winapi;

use bzip2::read::BzDecoder;
use duct::cmd;
use futures::{Future, Stream};
use futures_cpupool::CpuPool;
use gstuff::{last_modified_sec, now_float, slurp};
use hyper_rustls::HttpsConnector;
use std::env::var;
use std::fs;
use std::io::{Read, Write};
use std::iter::empty;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tar::Archive;

/// Ongoing (RLS) builds might interfere with a precise time comparison.
const SLIDE: f64 = 60.;

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
    let _ = fs::create_dir("c_headers");

    // NB: curve25519.h and cJSON.h are needed to parse LP_include.h.
    bindgen(
        vec![
            "../../includes/curve25519.h".into(),
            "../../includes/cJSON.h".into(),
            "../../iguana/exchanges/LP_include.h".into(),
        ],
        "c_headers/LP_include.rs",
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
            "LP_coin_curl_init",
            "LP_mutex_init",
            "LP_tradebots_timeslice",
            "stats_JSON",
            "LP_priceinfofind",
            "prices_loop",
            "LP_portfolio",
            "LP_coinadd_",
            "LP_priceinfoadd",
            "LP_unspents_load",
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
            "LP_autoprice",
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
            "LP_calc_waittimeout",
            "LP_numconfirms",
            "LP_pendswap_add",
            "LP_price_sig",
        ]
        .iter(),
        // types
        [
            "_bits256",
            "cJSON",
            "iguana_info",
            "LP_utxoinfo",
            "electrum_info",
            "LP_trade",
            "LP_swap_remember",
        ]
        .iter(),
        [
            // defines
            "LP_eth_node_url",
            "LP_alice_contract",
            "LP_bob_contract",
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
            "LP_swap_critical",
            "LP_swap_endcritical",
            "INSTANTDEX_PUBKEY",
            "BASILISK_BOBDEPOSIT",
            "BASILISK_ALICEPAYMENT",
            "BASILISK_BOBREFUND",
            "BASILISK_BOBSPEND",
        ]
        .iter(),
    );

    bindgen(
        vec!["../../crypto777/OS_portable.h".into()],
        "c_headers/OS_portable.rs",
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
        "c_headers/nn.rs",
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

/// Like the `duct` `cmd!` but also prints the command into the standard error stream.
macro_rules! ecmd {
    ( $program:expr ) => {{
        eprintln!("$ {}", $program);
        cmd($program, empty::<String>())
            .stdout_to_stderr()
    }};
    ( $program:expr $(, $arg:expr )* ) => {{
        let mut args: Vec<String> = Vec::new();
        $(
            args.push(Into::<String>::into($arg));
        )*
        eprintln!("$ {}{}", $program, show_args(&args));
        cmd($program, args)
            .stdout_to_stderr()
    }};
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

fn path2s(path: PathBuf) -> String {
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

#[derive(PartialEq, Eq, Debug)]
enum Target {
    Unix,
    Mac,
    Windows,
    /// https://github.com/rust-embedded/cross
    AndroidCross,
}
impl Target {
    fn load() -> Target {
        match &unwrap!(var("TARGET"))[..] {
            "x86_64-unknown-linux-gnu" => Target::Unix,
            "x86_64-apple-darwin" => Target::Mac,
            "x86_64-pc-windows-msvc" => Target::Windows,
            "armv7-linux-androideabi" => {
                if Path::new("/android-ndk").exists() {
                    Target::AndroidCross
                } else {
                    panic! ("/android-ndk not found. Please use the `cross` to cross-compile for Android.")
                }
            }
            t => panic!("Target not (yet) supported: {}", t),
        }
    }
    /// True if building for ARM under https://github.com/rust-embedded/cross or a similar setup.
    fn is_android_cross(&self) -> bool {
        *self == Target::AndroidCross
    }
    fn is_mac(&self) -> bool {
        *self == Target::Mac
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

        // Example using bcp to help with finding the subset:
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
            let pathˇ = unwrap!(path.to_str());
            assert!(pathˇ.starts_with("boost_1_68_0/"));
            let pathˇ = &pathˇ[13..];
            let unpack = pathˇ == "bootstrap.sh"
                || pathˇ == "boost-build.jam"
                || pathˇ == "boostcpp.jam"
                || pathˇ == "boost/assert.hpp"
                || pathˇ == "boost/aligned_storage.hpp"
                || pathˇ.starts_with("boost/asio/")
                || pathˇ.starts_with("boost/blank")
                || pathˇ == "boost/call_traits.hpp"
                || pathˇ.starts_with("boost/callable_traits/")
                || pathˇ == "boost/cerrno.hpp"
                || pathˇ == "boost/config.hpp"
                || pathˇ == "boost/concept_check.hpp"
                || pathˇ == "boost/crc.hpp"
                || pathˇ.starts_with("boost/container_hash/")
                || pathˇ.starts_with("boost/concept/")
                || pathˇ.starts_with("boost/config/")
                || pathˇ.starts_with("boost/core/")
                || pathˇ.starts_with("boost/chrono")
                || pathˇ == "boost/cstdint.hpp"
                || pathˇ == "boost/current_function.hpp"
                || pathˇ == "boost/checked_delete.hpp"
                || pathˇ.starts_with("boost/date_time/")
                || pathˇ.starts_with("boost/detail/")
                || pathˇ.starts_with("boost/exception/")
                || pathˇ.starts_with("boost/fusion/")
                || pathˇ.starts_with("boost/functional")
                || pathˇ.starts_with("boost/iterator/")
                || pathˇ.starts_with("boost/intrusive")
                || pathˇ.starts_with("boost/integer")
                || pathˇ == "boost/limits.hpp"
                || pathˇ.starts_with("boost/mpl/")
                || pathˇ.starts_with("boost/move")
                || pathˇ == "boost/next_prior.hpp"
                || pathˇ == "boost/noncopyable.hpp"
                || pathˇ.starts_with("boost/none")
                || pathˇ.starts_with("boost/numeric/")
                || pathˇ == "boost/operators.hpp"
                || pathˇ.starts_with("boost/optional")
                || pathˇ.starts_with("boost/predef")
                || pathˇ.starts_with("boost/preprocessor/")
                || pathˇ.starts_with("boost/pool/")
                || pathˇ == "boost/ref.hpp"
                || pathˇ.starts_with("boost/range/")
                || pathˇ.starts_with("boost/ratio")
                || pathˇ.starts_with("boost/system/")
                || pathˇ.starts_with("boost/smart_ptr/")
                || pathˇ == "boost/static_assert.hpp"
                || pathˇ == "boost/shared_ptr.hpp"
                || pathˇ == "boost/shared_array.hpp"
                || pathˇ.starts_with("boost/type_traits")
                || pathˇ.starts_with("boost/type_index")
                || pathˇ.starts_with("boost/tuple/")
                || pathˇ.starts_with("boost/thread")
                || pathˇ == "boost/throw_exception.hpp"
                || pathˇ == "boost/type.hpp"
                || pathˇ.starts_with("boost/utility/")
                || pathˇ == "boost/utility.hpp"
                || pathˇ.starts_with("boost/variant")
                || pathˇ == "boost/version.hpp"
                || pathˇ.starts_with("boost/winapi/")
                || pathˇ.starts_with("libs/config/")
                || pathˇ.starts_with("libs/chrono/")
                || pathˇ.starts_with("libs/date_time/")
                || pathˇ.starts_with("libs/system/")
                || pathˇ.starts_with("tools/build/")
                || pathˇ == "Jamroot";
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

    let bin = out_dir.join("bin");
    let bin = unwrap!(bin.to_str());
    let _ = fs::create_dir(&bin);
    if target.is_android_cross() && !Path::new("/tmp/bin/g++").exists() {
        unwrap!(ecmd!(
            "ln",
            "-sf",
            "/android-ndk/bin/arm-linux-androideabi-g++",
            fomat!((bin) "/g++")
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

    let tgz = out_dir().join("libtorrent-rasterbar-1.2.0.tar.gz");
    if !tgz.exists() {
        hget (
            "https://github.com/arvidn/libtorrent/releases/download/libtorrent_1_2_0/libtorrent-rasterbar-1.2.0.tar.gz",
            tgz.clone()
        );
        assert!(tgz.exists());
    }

    let rasterbar = out_dir().join("libtorrent-rasterbar-1.2.0");
    epintln!("libtorrent at "[rasterbar]);
    if !rasterbar.exists() {
        unwrap!(
            ecmd!("tar", "-xzf", "libtorrent-rasterbar-1.2.0.tar.gz")
                .dir(&out_dir())
                .run(),
            "Can't unpack libtorrent-rasterbar-1.2.0.tar.gz"
        );
        assert!(rasterbar.exists());
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
    } else if target.is_mac() {
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
    // TODO: If we decide to keep linking with libtorrent then we should distribute the
    //       https://github.com/arvidn/libtorrent/blob/master/LICENSE.

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

        let rasterbar = mmd.join("libtorrent-rasterbar-1.2.0-rc");
        if rasterbar.exists() {
            // Cache maintenance.
            let _ = fs::remove_file(mmd.join("libtorrent-rasterbar-1.2.0.tar.gz"));
            let _ = fs::remove_dir_all(rasterbar.join("docs"));
        } else {
            // [Download and] unpack.
            if !mmd.join("libtorrent-rasterbar-1.2.0.tar.gz").exists() {
                hget (
                    "https://github.com/arvidn/libtorrent/releases/download/libtorrent-1_2_0_RC/libtorrent-rasterbar-1.2.0.tar.gz",
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
            r"bin\msvc-14.1\release\address-model-64\link-static\threading-multi\libtorrent.lib",
        );
        if !lt.exists() {
            unwrap!(
                ecmd! (
                    "cmd", "/c",
                    "b2 release toolset=msvc-14.1 address-model=64 link=static dht=on debug-symbols=off"
                )
                .env(
                    "PATH",
                    format!("{};{}", unwrap!(boost.to_str()), unwrap!(var("PATH")))
                )
                .env("BOOST_BUILD_PATH", unwrap!(boost.to_str()))
                .env("BOOST_ROOT", unwrap!(boost.to_str()))
                .dir(&rasterbar)
                .run()
            );
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
    } else if cfg!(target_os = "macos") {
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
        println!("cargo:rustc-link-lib=static=torrent-rasterbar");
        println!(
            "cargo:rustc-link-search=native={}",
            unwrap!(unwrap!(lt_a.parent()).to_str())
        );

        // NB: We should prefer linking boost in AFTER libtorrent,
        // cf. "Linking boost_system last fixed the issue for me" in https://stackoverflow.com/a/30877725/257568.
        println!("cargo:rustc-link-lib=static=boost_system");
        println!(
            "cargo:rustc-link-search=native={}",
            unwrap!(boost.join("lib").to_str())
        );

        let lm_dht = unwrap!(last_modified_sec(&"dht.cc"), "Can't stat dht.cc");
        let out_dir = unwrap!(var("OUT_DIR"), "!OUT_DIR");
        let lib_path = Path::new(&out_dir).join("libdht.a");
        let lm_lib = last_modified_sec(&lib_path).unwrap_or(0.);
        let boost_inc = boost.join("include");
        assert!(boost_inc.join("boost/version.hpp").exists());
        if lm_dht >= lm_lib - SLIDE {
            cc::Build::new()
                .file("dht.cc")
                .warnings(true)
                // cf. .../out/libtorrent-rasterbar-1.2.0/config.report and Makefile/CXX
                // Mismatch between the libtorrent and the dht.cc flags
                // might produce weird "undefined reference" link errors.
                .flag("-std=c++11")
                .flag("-g")
                .flag("-O2")
                .flag("-ftemplate-depth=512")
                .flag("-fvisibility=hidden")
                .flag("-fvisibility-inlines-hidden")
                .flag("-fPIC")
                .flag("-DBOOST_ERROR_CODE_HEADER_ONLY=1")
                .flag("-DTORRENT_DISABLE_ENCRYPTION=1")
                .flag("-DBOOST_ASIO_HAS_STD_CHRONO=1")
                .flag("-DBOOST_EXCEPTION_DISABLE=1")
                .flag("-DBOOST_ASIO_ENABLE_CANCELIO=1")
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

/// Build MM1 libraries without CMake, making cross-platform builds more transparent to us.
fn manual_mm1_build(target: Target) {
    let nanomsg = out_dir().join("nanomsg-1.1.5");
    if !nanomsg.exists() {
        let nanomsg_tgz = out_dir().join("nanomsg.tgz");
        if !nanomsg_tgz.exists() {
            hget(
                "https://github.com/nanomsg/nanomsg/archive/1.1.5.tar.gz",
                nanomsg_tgz.clone(),
            );
            assert!(nanomsg_tgz.exists());
        }
        unwrap!(ecmd!("tar", "-xzf", "nanomsg.tgz").dir(out_dir()).run());
        assert!(nanomsg.exists());
    }
    epintln!("nanomsg at "[nanomsg]);

    let libnanomsg_a = nanomsg.join("libnanomsg.a");
    if !libnanomsg_a.exists() {
        if target.is_android_cross() {
            unwrap!(
                ecmd!("make", "-f", "/project/mm2src/common/android/nanomsg.mk")
                    .dir(&nanomsg)
                    .run()
            );
        } else {
            panic!("Target {:?}", target);
        }
    }

    let libmm1_a = out_dir().join("libmm1.a");
    if !libmm1_a.exists() {
        let mm1_build = out_dir().join("mm1_build");
        let _ = fs::create_dir(&mm1_build);
        epintln!("mm1_build at "[mm1_build]);
        if target.is_android_cross() {
            unwrap!(ecmd!(
                "/android-ndk/bin/clang",
                "-O2",
                "-g3",
                "-I/project/crypto777",
                "/project/iguana/exchanges/mm.c",
                "/project/iguana/mini-gmp.c",
                "/project/iguana/groestl.c",
                "/project/iguana/segwit_addr.c",
                "/project/iguana/keccak.c",
                "-c"
            )
            .dir(&mm1_build)
            .run());

            unwrap!(ecmd!(
                "/android-ndk/bin/arm-linux-androideabi-ar",
                "-rcs",
                unwrap!(libmm1_a.to_str()),
                "groestl.o",
                "keccak.o",
                "mini-gmp.o",
                "mm.o",
                "segwit_addr.o"
            )
            .dir(&mm1_build)
            .run());
        } else {
            panic!("Target {:?}", target);
        }
    }

    panic!("TBD")
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
    if target.is_android_cross() {
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

fn main() {
    // NB: `rerun-if-changed` will ALWAYS invoke the build.rs if the target does not exists.
    // cf. https://github.com/rust-lang/cargo/issues/4514#issuecomment-330976605
    //     https://github.com/rust-lang/cargo/issues/4213#issuecomment-310697337
    // `RUST_LOG=cargo::core::compiler::fingerprint cargo build` shows the fingerprit files used.

    // Rebuild when we work with C files.
    println!(
        "rerun-if-changed={}",
        path2s(rabs("iguana/exchanges/etomicswap"))
    );
    println!("rerun-if-changed={}", path2s(rabs("iguana/exchanges")));
    println!("rerun-if-changed={}", path2s(rabs("iguana/secp256k1")));
    println!("rerun-if-changed={}", path2s(rabs("crypto777")));
    println!("rerun-if-changed={}", path2s(rabs("crypto777/jpeg")));
    println!("rerun-if-changed={}", path2s(rabs("OSlibs/win")));
    println!("rerun-if-changed={}", path2s(rabs("CMakeLists.txt")));

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

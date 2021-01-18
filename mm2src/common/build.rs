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
use gstuff::{last_modified_sec, slurp};
use regex::Regex;
use std::env::{self, var};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::from_utf8;

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
fn build_c_code() {
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

    if !cfg!(windows) {
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
    build_c_code();
}

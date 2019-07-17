# AtomicDEX Marketmaker V2

This repository contains the `work in progress` code of brand new Marketmaker version built mainly on Rust.  
The current state can be considered as very early alpha.  
**Use with test coins only. You risk to lose your money in case of trying to trade assets with real market cost.**

## Rewrite goals

[REWRITE](docs/REWRITE.md)

## Project structure

[mm2src](mm2src) - Rust code, contains some parts ported from C `as is` (e.g. `lp_ordermatch`) to reach the most essential/error prone code. Some other modules/crates are reimplemented from scratch.  
[iguana](iguana) - Old C code that is still used and linked to Rust binary. According to gradual port goals this code will be either ported/removed in favor of Rust.  
 
## How to build

1. Tools required: [Cmake](https://cmake.org/install/), [Rustup](https://rustup.rs/). You will also need your OS specific build tools (e.g. build-essentials on Linux, XCode on OSX or MSVC on Win).
1. (Optional) OSX: install openssl, e.g. `brew install openssl`.  
1. (Optional) OSX: run `LIBRARY_PATH=/usr/local/opt/openssl/lib`
1. Run

    rustup install nightly-2019-06-26
    rustup default nightly-2019-06-26
    rustup component add rustfmt-preview

1. (Optional) Win: run `marketmaker_build_depends.cmd` to build dependencies.
1. Run `cargo build --features native` (or `cargo build --features native -vv` to get verbose build output).

## Building for Android

[ANDROID](docs/ANDROID.md)

## Help and troubleshooting

If you have any question/want to report a bug/suggest an improvement feel free to [open an issue](https://github.com/artemii235/SuperNET/issues/new) or reach the team at [Discord `dev-marketmaker` channel](https://discord.gg/PGxVm2y).  

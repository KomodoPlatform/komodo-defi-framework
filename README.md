# AtomicDEX Marketmaker V2

This repository contains the `work in progress` code of brand new Marketmaker version built mainly on Rust.  
The current state can be considered as very early alpha.  
**Use with test coins only. You risk to lose your money in case of trying to trade assets with real market cost.**

## Project structure

[mm2src](mm2src) - Rust code, contains some parts ported from C `as is` (e.g. `lp_ordermatch`) to reach the most essential/error prone code. Some other modules/crates are reimplemented from scratch.

## How to build

1. Tools required: [Rustup](https://rustup.rs/). You will also need your OS specific build tools (e.g. build-essentials on Linux, XCode on OSX or MSVC on Win).
1. (Optional) OSX: install openssl, e.g. `brew install openssl`.  
1. (Optional) OSX: run `LIBRARY_PATH=/usr/local/opt/openssl/lib`
1. Run
    ```
    rustup install nightly-2021-05-17
    rustup default nightly-2021-05-17
    rustup component add rustfmt-preview
    ```
1. Run `cargo build` (or `cargo build -vv` to get verbose build output).

## Help and troubleshooting

If you have any question/want to report a bug/suggest an improvement feel free to [open an issue](https://github.com/artemii235/SuperNET/issues/new) or reach the team at [Discord `dev-marketmaker` channel](https://discord.gg/PGxVm2y).  

## Additional docs for developers

[Contribution guide](./CONTRIBUTING.md)  
[Setting up the environment to run the full tests suite](./docs/DEV_ENVIRONMENT.md)  
[Git flow and general workflow](./docs/GIT_FLOW_AND_WORKING_PROCESS.md)  

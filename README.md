# AtomicDEX Marketmaker V2

<p align="center">
    <a href="https://github.com/komodoplatform/atomicdex-api/graphs/contributors" alt="Contributors">
        <img src="https://img.shields.io/github/contributors/komodoplatform/atomicdex-api" /></a>
    <a href="https://github.com/komodoplatform/atomicdex-api/releases">
        <img src="https://img.shields.io/github/downloads/komodoplatform/atomicdex-api/total" alt="downloads"></a>
    <a href="https://github.com/komodoplatform/atomicdex-api/">
        <img src="https://img.shields.io/github/last-commit/komodoplatform/atomicdex-api" alt="last commit"></a>
    <a href="https://github.com/komodoplatform/atomicdex-api/pulse" alt="Activity">
        <img src="https://img.shields.io/github/commit-activity/m/komodoplatform/atomicdex-api" /></a>
    <a href="https://github.com/komodoplatform/atomicdex-api/issues">
        <img src="https://img.shields.io/github/issues-raw/komodoplatform/atomicdex-api" alt="issues"></a>
    <a href="https://github.com/komodoplatform/atomicdex-api/issues?q=is%3Aissue+is%3Aclosed">
        <img src="https://img.shields.io/github/issues-closed-raw/komodoplatform/atomicdex-api" alt="issues closed"></a>
    <a href="https://github.com/komodoplatform/atomicdex-api/pulls">
        <img src="https://img.shields.io/github/issues-pr/komodoplatform/atomicdex-api" alt="pulls"></a>
    <a href="https://github.com/komodoplatform/atomicdex-api/pulls?q=is%3Apr+is%3Aclosed">
        <img src="https://img.shields.io/github/issues-pr-closed/komodoplatform/atomicdex-api" alt="pulls closed"></a>
    <a href="https://dev.azure.com/ortgma/Marketmaker/_build?definitionId=2">
        <img src="https://img.shields.io/azure-devops/build/ortgma/marketmaker/2/mm2.1" alt="build status"></a>
    <a href="https://github.com/KomodoPlatform/dPoW/releases">
        <img src="https://img.shields.io/github/v/release/komodoplatform/atomicdex-api" alt="release version"></a>
    <a href="https://discord.gg/3rzDPAr">
        <img src="https://img.shields.io/discord/412898016371015680?logo=discord" alt="chat on Discord"></a>
    <a href="https://twitter.com/intent/follow?screen_name=https://twitter.com/atomicdex">
        <img src="https://img.shields.io/twitter/follow/atomicdex?style=social&logo=twitter"
            alt="follow on Twitter"></a>
</p>
<hr>

This repository contains the `work in progress` code of the brand new AtomicDEX API core (mm2) built mainly on Rust.  
The current state can be considered as a alpha version.

**<b>WARNING: Use with test coins only or with assets which value does not exceed an amount you are willing to lose. This is alpha stage software! </b>**


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

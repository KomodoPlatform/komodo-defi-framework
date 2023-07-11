<p align="center">
    <a href="https://atomicdex.io" alt="Contributors">
        <img width="420" src="https://user-images.githubusercontent.com/24797699/252396802-de8f9264-8056-4430-a17d-5ecec9668dfc.png" />
    </a>
</p>

<p align="center">
    <a href="https://github.com/komodoplatform/komodo-defi-framework/graphs/contributors" alt="Contributors">
        <img src="https://img.shields.io/github/contributors/komodoplatform/komodo-defi-framework" />
    </a>
    <a href="https://github.com/komodoplatform/komodo-defi-framework/releases">
        <img src="https://img.shields.io/github/downloads/komodoplatform/komodo-defi-framework/total" alt="downloads">
    </a>
    <a href="https://github.com/komodoplatform/komodo-defi-framework/">
        <img src="https://img.shields.io/github/last-commit/komodoplatform/komodo-defi-framework" alt="last commit">
    </a>
    <a href="https://github.com/komodoplatform/komodo-defi-framework/pulse" alt="Activity">
        <img src="https://img.shields.io/github/commit-activity/m/komodoplatform/komodo-defi-framework" />
    </a>
    <br/>
    <a href="https://github.com/komodoplatform/komodo-defi-framework/issues">
        <img src="https://img.shields.io/github/issues-raw/komodoplatform/komodo-defi-framework" alt="issues">
    </a>
    <a href="https://github.com/komodoplatform/komodo-defi-framework/issues?q=is%3Aissue+is%3Aclosed">
        <img src="https://img.shields.io/github/issues-closed-raw/komodoplatform/komodo-defi-framework" alt="issues closed">
    </a>
    <a href="https://github.com/komodoplatform/komodo-defi-framework/pulls">
        <img src="https://img.shields.io/github/issues-pr/komodoplatform/komodo-defi-framework" alt="pulls">
    </a>
    <a href="https://github.com/komodoplatform/komodo-defi-framework/pulls?q=is%3Apr+is%3Aclosed">
        <img src="https://img.shields.io/github/issues-pr-closed/komodoplatform/komodo-defi-framework" alt="pulls closed">
    </a>
    <br/>
    <a href="https://github.com/KomodoPlatform/komodo-defi-framework/actions/workflows/release-build.yml">
        <img src="https://github.com/KomodoPlatform/komodo-defi-framework/actions/workflows/release-build.yml/badge.svg?branch=main" alt="build status">
    </a>
    <a href="https://github.com/KomodoPlatform/komodo-defi-framework/releases">
        <img src="https://img.shields.io/github/v/release/komodoplatform/komodo-defi-framework" alt="release version">
    </a>
    <a href="https://discord.gg/3rzDPAr">
        <img src="https://img.shields.io/discord/412898016371015680?logo=discord" alt="chat on Discord">
    </a>
    <a href="https://twitter.com/komodoplatform">
        <img src="https://img.shields.io/twitter/follow/komodoplatform?style=social&logo=twitter" alt="follow on Twitter">
    </a>
</p>


## What is the Komodo DeFi Framework?

The Komodo DeFi Framework is open-source [atomic-swap](https://komodoplatform.com/en/academy/atomic-swaps/) software for seamless, decentralised, peer to peer trading between almost every blockchain asset in existence. This software works with propagation of orderbooks and swap states through the [libp2p](https://libp2p.io/) protocol and uses [Hash Time Lock Contracts (HTLCs)](https://en.bitcoinwiki.org/wiki/Hashed_Timelock_Contracts) for ensuring that the two parties in a swap either mutually complete a trade, or funds return to thier original owner.

There is no 3rd party intermediary, no proxy tokens, and at all times users remain in sole possession of their private keys.

A [well documented API](https://developers.komodoplatform.com/basic-docs/atomicdex/introduction-to-atomicdex.html) offers simple access to the underlying services using simple language agnostic JSON structured methods and parameters such that users can communicate with the core in a variety of methods such as [curl](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/buy.html) in CLI, or fully functioning [desktop and mobile applications](https://atomicdex.io/) like [Komodo Wallet Desktop](https://github.com/KomodoPlatform/komodo-wallet-desktop).

For a curated list of Komodo DeFi Framework based projects and resources, check out [Awesome AtomicDEX](https://github.com/KomodoPlatform/awesome-atomicdex).


## Features

- Perform blockchain transactions without a local native chain (e.g. via Electrum servers)
- Query orderbooks for all pairs within the [supported coins](https://github.com/KomodoPlatform/coins/blob/master/coins)
- Buy/sell from the orderbook, or create maker orders
- Configure automated ["makerbot" trading](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20-dev/start_simple_market_maker_bot.html) with periodic price updates and optional [telegram](https://telegram.org/) alerts


## System Requirements

- 64-bit MacOS, Windows, or Linux operating system
- 2GB of free RAM (or more)
- User account with admin/root privileges

## Building from source

[Pre-built release binaries](https://developers.komodoplatform.com/basic-docs/atomicdex/atomicdex-setup/get-started-atomicdex.html) are available for OSX, Linux or Windows.

If you want to build from source, the following prerequisites are required:
- [Rustup](https://rustup.rs/)
- OS specific build tools (e.g. [build-essential](https://linuxhint.com/install-build-essential-ubuntu/) on Linux, [XCode](https://apps.apple.com/us/app/xcode/id497799835?mt=12) on OSX or [MSVC](https://docs.microsoft.com/en-us/cpp/build/vscpp-step-0-installation?view=vs-2017) on Win)
- (Optional) OSX: install [openssl](https://www.openssl.org/), e.g. `brew install openssl`.
- (Optional) OSX: run `LIBRARY_PATH=/usr/local/opt/openssl/lib`
- (Optional) Linux: Install libudev-dev (dpkg) or libudev-devel (rpm) package.
- [Download](https://github.com/protocolbuffers/protobuf/releases) or [compile](https://github.com/protocolbuffers/protobuf) `protoc 3.21.x+` and add it to your PATH env. It is also available via package managers depending on the OS.
- Additional Rust Components
    ```
    rustup install nightly-2022-10-29
    rustup default nightly-2022-10-29
    rustup component add rustfmt-preview
    ```

To build, run `cargo build` (or `cargo build -vv` to get verbose build output).

For more detailed instructions, please refer to the [Installation Guide](https://developers.komodoplatform.com/basic-docs/atomicdex/atomicdex-setup/get-started-atomicdex.html).

## Building WASM binary

Please refer to the [WASM Build Guide](./docs/WASM_BUILD.md).

## Configuration

Basic config is contained in two files, `MM2.json` and `coins`

The user configuration [MM2.json file](https://developers.komodoplatform.com/basic-docs/atomicdex/atomicdex-setup/configure-mm2-json.html) contains rpc credentials, your mnemonic seed phrase, a `netid` (7777 is the current main network) and some extra [optional parameters](https://developers.komodoplatform.com/basic-docs/atomicdex/atomicdex-setup/get-started-atomicdex.html).

For example:
```json
{
  "gui": "core_readme",
  "netid": 7777,
  "rpc_password": "Ent3r_Un1Qu3_Pa$$w0rd",
  "passphrase": "ENTER_UNIQUE_SEED_PHRASE_DONT_USE_THIS_CHANGE_IT_OR_FUNDS_NOT_SAFU"
}
```

The coins file contains information about the coins and tokens you want to trade. A regularly updated version is maintained in the [Komodo Platform coins repository](https://github.com/KomodoPlatform/coins/blob/master/coins). Pull Requests to add any coins not yet included are welcome.

To facilitate interoperability with the `mm2` service, there is the `adex-cli` command line utility. It provides a questionnaire initialization mode to set up the configuration and obtain the proper coin set through the internet. It can also be used to start or stop the service.

## Usage

To launch the Komodo DeFi Framework, run `./mm2` (or `mm2.exe` in Windows)

To activate a coin:
```bash
curl --url "http://127.0.0.1:7783" --data '{
	"coin": "KMD",
	"method": "electrum",
	"servers": [
		{"url": "electrum1.cipig.net:10001"},
		{"url": "electrum2.cipig.net:10001"},
		{"url": "electrum3.cipig.net:10001"}
	],
	"required_confirmations":10,
	"requires_notarization":true,
	"mm2":1,
	"userpass": "$userpass"
}'
```

To view the orderbook for a pair:
```bash
curl --url "http://127.0.0.1:7783" --data '{
    "userpass":"$userpass",
    "method":"orderbook",
    "base":"KMD",
    "rel":"BTC"
}'
```

To place a buy order:
```bash
curl --url "http://127.0.0.1:7783" --data '{
  "userpass": "$userpass",
  "method": "buy",
  "base": "KMD",
  "rel": "DOGE",
  "volume": "10",
  "price": "4"
}'
```

Refer to the [Komodo Developer Docs](https://developers.komodoplatform.com/basic-docs/atomicdex/introduction-to-atomicdex.html) for details of additional RPC methods and parameters


## Project structure

[mm2src](mm2src) - Rust code, contains some parts ported from C `as is` (e.g. `lp_ordermatch`) to reach the most essential/error prone code. Some other modules/crates are reimplemented from scratch.


## Additional docs for developers

- [Contribution guide](./docs/CONTRIBUTING.md)
- [Setting up the environment to run the full tests suite](./docs/DEV_ENVIRONMENT.md)
- [Git flow and general workflow](./docs/GIT_FLOW_AND_WORKING_PROCESS.md)
- [Komodo Developer Docs](https://developers.komodoplatform.com/basic-docs/atomicdex/introduction-to-atomicdex.html)


## Disclaimer

This repository contains the `work in progress` code of the brand new Komodo DeFi Framework (mm2) built mainly on Rust.  
The current state can be considered as a alpha version.

**<b>WARNING: Use with test coins only or with assets which value does not exceed an amount you are willing to lose. This is alpha stage software! </b>**


## Help and troubleshooting

If you have any question/want to report a bug/suggest an improvement feel free to [open an issue](https://github.com/artemii235/SuperNET/issues/new) or join the  [Komodo Platform Discord](https://discord.gg/PGxVm2y) `dev-marketmaker` channel.  

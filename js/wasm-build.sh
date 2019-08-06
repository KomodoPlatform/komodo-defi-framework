#!/bin/dash
# Run with `dash js/wasm-build.sh`.
set -ex
cargo build --target=wasm32-unknown-unknown --release --package=peers
ln -f target/wasm32-unknown-unknown/release/peers.wasm js/peers.wasm

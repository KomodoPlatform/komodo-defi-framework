#!/bin/dash
# Run with `dash js/wasm-build.sh`.
set -ex

cargo build --bin mm2
ln -f target/debug/mm2 js/mm2

cargo build --target=wasm32-unknown-unknown --release
ln -f target/wasm32-unknown-unknown/release/mm2.wasm js/mm2.wasm

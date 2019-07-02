#!/bin/bash

# Builds WASM in a separate folder in order not to mess the native build.
# Run with `bash wasm-build.sh`.

ORIGINAL=`pwd`

. ~/.profile

rsync -av --delete \
  ./ /tmp/supernet-wasm/ \
  --exclude=/target \
  --exclude=/build \
  --exclude=/x64 \
  --exclude=/marketmaker_depends \
  --exclude=/.git \
  --exclude=/.vscode \
  --exclude=/DB \
  --exclude=/wasm-build.log

cd /tmp/supernet-wasm/

# Plug in the Emscripten
. ~/emsdk/emsdk_env.sh

cargo build --target=wasm32-unknown-emscripten 2>&1 | tee $ORIGINAL/wasm-build.log

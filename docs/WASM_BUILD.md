# Building WASM binary

## From Container:

If you want to build from source without installing prerequisites to your host system, you can do so by binding the source code inside a container and compiling it there.

Build the image:

```sh
docker build -t kdf-build-container -f .docker/Dockerfile .
```

Bind source code into container and compile it:
```sh
docker run -v "$(pwd)":/app -w /app kdf-build-container wasm-pack build mm2src/mm2_bin_lib --target web --out-dir wasm_build/deps/pkg/
```

## Setting up the environment

To build WASM binary from source, the following prerequisites are required:

1. Install `wasm-pack`
   ```
   curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
   ```
2. OSX specific: install `llvm`
   ```
   brew install llvm
   ```

## Compiling WASM release binary

To build WASM release binary run one of the following commands according to your environment:

- for Linux users:
   ```
   wasm-pack build mm2src/mm2_bin_lib --target web --out-dir wasm_build/deps/pkg/
   ```
- for OSX users (Intel):
   ```
   CC=/usr/local/opt/llvm/bin/clang AR=/usr/local/opt/llvm/bin/llvm-ar wasm-pack build mm2src/mm2_bin_lib --target web --out-dir wasm_build/deps/pkg/
   ```
- for OSX users (Apple Silicon):
   ```
   CC=/opt/homebrew/opt/llvm/bin/clang AR=/opt/homebrew/opt/llvm/bin/llvm-ar wasm-pack build mm2src/mm2_bin_lib --target web --out-dir wasm_build/deps/pkg/
   ```

Please note `CC` and `AR` must be specified in the same line as `wasm-pack build`.

### Troubleshooting wasm-opt errors

If you encounter errors during the wasm-opt optimization step (e.g., "Bulk memory operations require bulk memory" or sign extension errors), this is a known issue with wasm-pack 0.10.0 on macOS when using Rust 1.72+ with certain WASM features.

**Solution: Build with cargo and wasm-bindgen directly**
```bash
# Build WASM with cargo (macOS Apple Silicon)
CC=/opt/homebrew/opt/llvm/bin/clang AR=/opt/homebrew/opt/llvm/bin/llvm-ar \
cargo build --release --target wasm32-unknown-unknown -p mm2_bin_lib

# Generate JS bindings without wasm-opt
wasm-bindgen target/wasm32-unknown-unknown/release/kdflib.wasm \
--out-dir wasm_build/deps/pkg --target web
```

For macOS Intel, use `/usr/local/opt/llvm/bin/clang` and `/usr/local/opt/llvm/bin/llvm-ar` instead.

Note: This approach bypasses wasm-opt entirely - the resulting WASM file is larger but functionally identical, with all Rust release optimizations still applied. Linux environments typically don't experience this issue.

This workaround is only necessary when using Rust 1.72+ with wasm-pack 0.10.0 on macOS. Updating to a newer Rust toolchain and compatible wasm-pack version would eliminate the need for this workaround.

## Compiling WASM binary with debug symbols

If you want to disable optimizations to reduce the compilation time, run `wasm-pack build mm2src/mm2_bin_lib` with an additional `--dev` flag:
```
wasm-pack build mm2src/mm2_bin_lib --target web --out-dir wasm_build/deps/pkg/ --dev
```

Please don't forget to specify `CC` and `AR` if you run the command on OSX.



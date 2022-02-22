# MarketMaker2 wasm example

**wasm_build** is an example of using **MarketMaker2** in webpages
via [WebAssembly](https://developer.mozilla.org/en-US/docs/WebAssembly)

## How to build

The below steps are performed in the `atomicDEX-API/` root directory.

### Compile MarketMaker2 to wasm

```
wasm-pack build --target web --out-dir wasm_build/deps/pkg/
```

Note the command above compiles and optimizes the wasm binary. If you want to disable optimizations to reduce the
compilation time, run the following:

```
wasm-pack build --dev --target web --out-dir wasm_build/deps/pkg/
```

#### Compile MarketMaker2 to wasm on MacOS

```
brew install llvm
# on M1
CC=/opt/homebrew/opt/llvm/bin/clang AR=/opt/homebrew/opt/llvm/bin/llvm-ar wasm-pack build --target web --out-dir wasm_build/deps/pkg/
# on Intel/AMD
CC=/usr/local/opt/llvm/bin/clang AR=/usr/local/opt/llvm/bin/llvm-ar wasm-pack build --target web --out-dir wasm_build/deps/pkg/
```

### Running a local HTTP server

If you have Python 3, run

```
python3 -m http.server 8000
```

Or run

```
python -m SimpleHTTPServer 8000
```

Read more
about [running a simple local HTTP server](https://developer.mozilla.org/en-US/docs/Learn/Common_questions/set_up_a_local_testing_server#running_a_simple_local_http_server)

### Open webpage in your browser

`http://localhost:8000/wasm_build/index.html`

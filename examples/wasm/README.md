# AtomicDEX-API WASM example

**wasm_build** is an example of using **MarketMaker2** in webpages
via [WebAssembly](https://developer.mozilla.org/en-US/docs/WebAssembly)

1. Build WASM binary according to the [WASM Build Guide](../docs/WASM_BUILD.md)
2. Run a local HTTP server
    - if you use Python 3, run:
   ```
   python3 -m http.server 8000
   ```
    - if you use Python 2, run:
   ```
   python -m SimpleHTTPServer 8000
   ```
   Read more about
   [running a simple local HTTP server](https://developer.mozilla.org/en-US/docs/Learn/Common_questions/set_up_a_local_testing_server#running_a_simple_local_http_server)
3. Open webpage in your browser http://localhost:8000/wasm_build/index.html
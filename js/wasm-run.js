// Run with:
// 
//     (cd js && node wasm-run.js)

const fs = require ('fs');

// npm install -g node-gyp
// Using a version of 'ffi' that works with Node 12.6,
// cf. https://github.com/node-ffi/node-ffi/pull/544
// npm install --save lxe/node-ffi#node-12
const ffi = require ('ffi');
const ref = require ('ref');

// Preparing the library:
// 
//     cargo build --features native --package peers --release
//     cp target/release/peers.dll ./
//     cp x64/pthreadVC2.dll ./
// 
// cf. https://github.com/node-ffi/node-ffi/wiki/Node-FFI-Tutorial
const libpeers = ffi.Library ('peers', {
  'is_loopback_ip': [ref.types.uint8, ['string']]
});
console.log ('is_loopback_ip (127.0.0.1) = ' + libpeers.is_loopback_ip ('127.0.0.1'));
console.log ('is_loopback_ip (8.8.8.8) = ' + libpeers.is_loopback_ip ('8.8.8.8'));

async function runWasm() {
  const wasmPath = process.env.WASM_PATH;
  if (!wasmPath) throw new Error ('No WASM_PATH');
  const wasmBytes = fs.readFileSync (wasmPath);
  const wasmArray = new Uint8Array (wasmBytes);
  const wasmEnv = {
    bitcoin_ctx_destroy: function() {console.log ('bitcoin_ctx_destroy')}
  };
  const wasmInstantiated = await WebAssembly.instantiate (wasmBytes, {env: wasmEnv});
  const peers_check = wasmInstantiated.instance.exports.peers_check();
  console.log ('peers_check: ' + peers_check);
}

runWasm().catch (ex => console.log (ex));

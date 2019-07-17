// Run with:
// 
//     bash js/wasm-build.sh && (cd js && node wasm-run.js | rustfilt)

const fs = require ('fs');

// npm install -g node-gyp
// Using a version of 'ffi' that works with Node 12.6,
// cf. https://github.com/node-ffi/node-ffi/pull/544
// npm install --save lxe/node-ffi#node-12
const ffi = require ('ffi');
// https://github.com/TooTallNate/ref; http://tootallnate.github.io/ref/
const ref = require ('ref');
const { Buffer } = require ('buffer');

// Preparing the library:
// 
//     cargo build --features native --package peers --release
//     cp target/release/peers.dll js/
//     cp x64/pthreadVC2.dll js/
// 
// cf. https://github.com/node-ffi/node-ffi/wiki/Node-FFI-Tutorial
const libpeers = ffi.Library ('peers', {
  'is_loopback_ip': [ref.types.uint8, ['string']],
  'ctx2helpers': [ref.types.void, [ref.refType (ref.types.uint8), ref.types.uint32]],
  'peers_initialize': [ref.types.void, [
    ref.refType (ref.types.uint8), ref.types.uint32, ref.refType (ref.types.uint8), ref.refType (ref.types.uint32)]]
});
const ili_127_0_0_1 = libpeers.is_loopback_ip ('127.0.0.1');
//console.log ('is_loopback_ip (127.0.0.1) = ' + ili_127_0_0_1);
const ili_8_8_8_8 = libpeers.is_loopback_ip ('8.8.8.8');
//console.log ('is_loopback_ip (8.8.8.8) = ' + ili_8_8_8_8);

function from_utf8 (memory, ptr, len) {
  const view = new Uint8Array (memory.buffer, ptr, len);
  const utf8dec = new TextDecoder ('utf-8');
  return utf8dec.decode (view)}

async function runWasm() {
  const wasmPath = process.env.WASM_PATH;
  if (!wasmPath) throw new Error ('No WASM_PATH');
  const wasmBytes = fs.readFileSync (wasmPath);
  const wasmArray = new Uint8Array (wasmBytes);
  let wasmShared = {};
  const wasmEnv = {
    bitcoin_ctx: function() {console.log ('env/bitcoin_ctx')},
    bitcoin_ctx_destroy: function() {console.log ('env/bitcoin_ctx_destroy')},
    console_log: function (ptr, len) {
      const decoded = from_utf8 (wasmShared.memory, ptr, len);
      console.log (decoded)},
    ctx2helpers: function (ptr, len) {
      const ctx_s = Buffer.from (wasmShared.memory.buffer.slice (ptr, ptr + len));
      libpeers.ctx2helpers (ctx_s, ctx_s.byteLength)},
    date_now: function() {return Date.now()},
    peers_initialize: function (ptr, len, rbuf, rlen) {
      const to_helper = Buffer.from (wasmShared.memory.buffer.slice (ptr, ptr + len));
      const rlen_slice = new Uint32Array (wasmShared.memory.buffer, rlen, 4);
      const rbuf_capacity = rlen_slice[0];
      const rbuf_slice = new Uint8Array (wasmShared.memory.buffer, rbuf, rbuf_capacity);
      const node_rbuf = Buffer.alloc (rbuf_capacity);  // `ffi` only understands Node arrays.
      const node_rlen = ref.alloc (ref.types.uint32, rbuf_capacity);
      libpeers.peers_initialize (to_helper, to_helper.byteLength, node_rbuf, node_rlen);
      const rbuf_len = ref.deref (node_rlen);
      if (rbuf_len >= rbuf_capacity) throw new Error ('Bad rbuf_len');
      node_rbuf.copy (rbuf_slice, 0, 0, rbuf_len);
      rlen_slice[0] = rbuf_len}};
  const wasmInstantiated = await WebAssembly.instantiate (wasmBytes, {env: wasmEnv});
  const exports = wasmInstantiated.instance.exports;
  /** @type {WebAssembly.Memory} */
  wasmShared.memory = exports.memory;

  exports.set_panic_hook();

  const peers_check = exports.peers_check();
  //console.log ('peers_check: ' + peers_check);

  console.log ('running test_peers_dht...');
  exports.test_peers_dht();
  console.log ('done with test_peers_dht')}

runWasm().catch (ex => console.log (ex));

// Run with:
// 
//     dash js/wasm-build.sh && (cd js && node wasm-run.js | rustfilt)

const fs = require ('fs');
const http = require('http');  // https://nodejs.org/dist/latest-v12.x/docs/api/http.html

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
const io_buf_args = [ref.types.void, [
  ref.refType (ref.types.uint8), ref.types.uint32, ref.refType (ref.types.uint8), ref.refType (ref.types.uint32)]];
const libpeers = ffi.Library ('./peers', {
  'ctx2helpers': [ref.types.void, [ref.refType (ref.types.uint8), ref.types.uint32]],
  'is_loopback_ip': [ref.types.uint8, ['string']],
  'peers_drop_send_handler': [ref.types.void, [ref.types.int32, ref.types.int32]],
  'start_helpers': [ref.types.int32, []]
});
const ili_127_0_0_1 = libpeers.is_loopback_ip ('127.0.0.1');
//console.log ('is_loopback_ip (127.0.0.1) = ' + ili_127_0_0_1);
const ili_8_8_8_8 = libpeers.is_loopback_ip ('8.8.8.8');
//console.log ('is_loopback_ip (8.8.8.8) = ' + ili_8_8_8_8);

function from_utf8 (memory, ptr, len) {
  const view = new Uint8Array (memory.buffer, ptr, len);
  const utf8dec = new TextDecoder ('utf-8');
  return utf8dec.decode (view)}

/** Proxy invoking a helper function which takes the (ptr, len) input and fills the (rbuf, rlen) output. */
function io_buf_proxy (wasmShared, helper, ptr, len, rbuf, rlen) {
  const encoded_args = Buffer.from (wasmShared.memory.buffer.slice (ptr, ptr + len));
  const rlen_slice = new Uint32Array (wasmShared.memory.buffer, rlen, 4);
  const rbuf_capacity = rlen_slice[0];
  const rbuf_slice = new Uint8Array (wasmShared.memory.buffer, rbuf, rbuf_capacity);
  const node_rbuf = Buffer.alloc (rbuf_capacity);  // `ffi` only understands Node arrays.
  const node_rlen = ref.alloc (ref.types.uint32, rbuf_capacity);
  helper (encoded_args, encoded_args.byteLength, node_rbuf, node_rlen);
  const rbuf_len = ref.deref (node_rlen);
  if (rbuf_len >= rbuf_capacity) throw new Error ('Bad rbuf_len');
  node_rbuf.copy (rbuf_slice, 0, 0, rbuf_len);
  rlen_slice[0] = rbuf_len}

async function runWasm() {
  const wasmBytes = fs.readFileSync ('peers.wasm');
  const wasmArray = new Uint8Array (wasmBytes);
  const keepAliveAgent = new http.Agent ({keepAlive: true});
  const ctx_and_port = libpeers.start_helpers();
  console.log ('ctx_and_port', ctx_and_port);
  let httpRequests = {};
  let wasmShared = {};
  const wasmEnv = {
    bitcoin_ctx: function() {console.log ('env/bitcoin_ctx')},
    bitcoin_ctx_destroy: function() {console.log ('env/bitcoin_ctx_destroy')},
    console_log: function (ptr, len) {
      const decoded = from_utf8 (wasmShared.memory, ptr, len);
      console.log (decoded)},
    common_wait_for_log_re: function (ptr, len, rbuf, rlen) {
      io_buf_proxy (wasmShared, libpeers.common_wait_for_log_re, ptr, len, rbuf, rlen)},
    ctx2helpers: function (ptr, len) {
      const ctx_s = Buffer.from (wasmShared.memory.buffer.slice (ptr, ptr + len));
      libpeers.ctx2helpers (ctx_s, ctx_s.byteLength)},
    date_now: function() {return Date.now()},
    http_helper_check: function (http_request_id, rbuf, rcap) {
      let ris = '' + http_request_id;
      if (httpRequests[ris] == null) return -1;
      if (httpRequests[ris].buf == null) return -1;
      /** @type {Uint8Array} */ 
      const buf = httpRequests[ris].buf;
      if (buf.length > rcap) return -buf.length;
      const rbuf_slice = new Uint8Array (wasmShared.memory.buffer, rbuf, rcap);
      for (let i = 0; i < buf.length; ++i) rbuf_slice[i] = buf[i];
      return buf.length
    },
    http_helper_if: function (helper_ptr, helper_len, payload_ptr, payload_len, timeout_ms) {
      const helper = from_utf8 (wasmShared.memory, helper_ptr, helper_len);
      const payload = from_utf8 (wasmShared.memory, payload_ptr, payload_len);
      console.log ('http_helper_if, helper is', helper);

      // Find a random ID.
      let ri, ris;
      for (;;) {
        ri = Math.ceil (Math.random() * 2147483647);
        ris = '' + ri;
        if (httpRequests[ris] == null) {
          httpRequests[ris] = {};
          break
        }
      }

      let chunks = [];
      const req = http.request ({
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': payload.length
        },
        hostname: '127.0.0.1',
        port: ctx_and_port,
        path: '/helper/' + helper,
        agent: keepAliveAgent,
        timeout: timeout_ms
      }, (res) => {
        res.on ('data', (chunk) => chunks.push (chunk));
        res.on ('end', () => {
          let len = 0;
          for (const chunk of chunks) {len += chunk.length}
          if (res.headers["content-length"] != null && len != res.headers["content-length"]) {
            throw new Error ('Content-Length mismatch')
          }
          const buf = new Uint8Array (len);
          let pos = 0;
          for (const chunk of chunks) {
            for (let i = 0; i < chunk.length; ++i) {
              buf[pos] = chunk[i];
              ++pos
            }
          }
          if (pos != len) throw new Error ('length mismatch')
          // TODO: Wake that particular future immediately?
          httpRequests[ris].buf = buf
        });
      });
      req.on ('error', function (err) {
        // TODO: Propagate to the client.
        console.log ('req err is', err)
      })
      req.write (payload);
      req.end();
      return ri  //< request id
    },
    peers_drop_send_handler: function (shp1, shp2) {
      libpeers.peers_drop_send_handler (shp1, shp2)},
    peers_initialize: function (ptr, len, rbuf, rlen) {
      io_buf_proxy (wasmShared, libpeers.peers_initialize, ptr, len, rbuf, rlen)},
    peers_recv: function (ptr, len, rbuf, rlen) {
      io_buf_proxy (wasmShared, libpeers.peers_recv, ptr, len, rbuf, rlen)},
    peers_send: function (ptr, len, rbuf, rlen) {
      io_buf_proxy (wasmShared, libpeers.peers_send, ptr, len, rbuf, rlen)}};
  const wasmInstantiated = await WebAssembly.instantiate (wasmBytes, {env: wasmEnv});
  const exports = wasmInstantiated.instance.exports;
  /** @type {WebAssembly.Memory} */
  wasmShared.memory = exports.memory;

  setImmediate (function() {
    console.log ('immediate at', Date.now());
    exports.run_executor();
  });
  // NB: The interval seems to run independently from the event loop,
  // whereas the `setImmediate` and `nextTick` callbacks only run when the control is returned from Rust.
  // On the other hand, an active interval prevents the Node.js process from exiting (on a panic).
  let i = setInterval (function() {
    console.log ('interval at', Date.now());
    exports.run_executor();
  }, 2000);
  process.nextTick (function() {
    console.log ('nextTick at', Date.now());
    exports.run_executor();
  });

  exports.set_panic_hook();

  const peers_check = exports.peers_check();
  //console.log ('peers_check: ' + peers_check);

  console.log ('running test_peers_dht...');
  // TODO: A way to wait for that Future on the JavaScript side.
  exports.test_peers_dht();
  console.log ('done with test_peers_dht');

  //clearInterval (i)
}

runWasm().catch (ex => console.log (ex));

// Run with:
// 
//     dash js/wasm-build.sh && (cd js && node wasm-run.js 2>&1 | rustfilt)

const bencode = require('bencode')
const { Buffer } = require('buffer');
const crc32 = require('crc-32');  // TODO: Calculate the checksum in Rust instead.
const ElectrumCli = require('electrum-client');  // https://www.npmjs.com/package/electrum-client
const fs = require('fs');
const http = require('http');  // https://nodejs.org/dist/latest-v12.x/docs/api/http.html
const os = require('os');
// https://nodejs.org/api/child_process.html
// https://www.npmjs.com/package/cross-spawn
const spawn = require('cross-spawn');
// https://nodejs.org/api/worker_threads.html
// https://medium.com/@Trott/using-worker-threads-in-node-js-80494136dbb6
// https://github.com/WebAssembly/threads/blob/master/proposals/threads/Overview.md
const worker_threads = require('worker_threads');

const snooze = ms => new Promise (resolve => setTimeout (resolve, ms));

const keepAliveAgent = new http.Agent ({keepAlive: true});

function from_utf8 (memory, ptr, len) {
  const view = new Uint8Array (memory.buffer, ptr, len);
  const utf8dec = new TextDecoder ('utf-8');
  return utf8dec.decode (view)}

function to_utf8 (memory, rbuf, rcap, str) {
  const encoder = new TextEncoder();
  const view = encoder.encode (str);
  if (view.length > rcap) return -1;
  const rbuf_slice = new Uint8Array (wasmShared.memory.buffer, rbuf, rcap);
  for (let i = 0; i < view.length; ++i) rbuf_slice[i] = view[i];
  return view.length}

function http_helper (helper, timeout_ms, payload, cb) {
  const cs = crc32.buf (payload);
  return http.request ({
    method: 'POST',
    headers: {
      'Content-Type': 'application/octet-stream',
      'Content-Length': payload.length,
      'X-Helper-Checksum': cs
    },
    hostname: '127.0.0.1',
    port: 7783,
    path: '/helper/' + helper,
    agent: keepAliveAgent,
    timeout: timeout_ms
  }, cb)
}

const wasmShared = {};

function registerCallback (f) {
  for (;;) {
    const ri = Math.ceil (Math.random() * 2147483647);
    const ris = '' + ri;  // TODO: Use a sparse array.
    if (wasmShared.callbacks[ris] != null) continue;
    wasmShared.callbacks[ris] = f;
    return ri}}

async function runWasm() {
  // Wait for the helpers RPC server to start.
  await snooze (500);

  const wasmBytes = fs.readFileSync ('mm2.wasm');
  const httpRequests = {};
  wasmShared.callbacks = {};
  wasmShared.electrums = {};
  const memory = new WebAssembly.Memory ({initial: 1, shared: true});
  const wasmEnv = {
    //memory: memory,
    call_back: function (cb_id, ptr, len) {
      //console.log ('call_back', cb_id, 'invoked, ptr', ptr, 'len', len);
      const cb_id_s = '' + cb_id;
      const f = wasmShared.callbacks[cb_id_s];
      if (f != null) f (Buffer.from (wasmShared.memory.buffer.slice (ptr, ptr + len)));
      delete wasmShared.callbacks[cb_id_s]},
    console_log: function (ptr, len) {
      const decoded = from_utf8 (wasmShared.memory, ptr, len);
      console.log (decoded)},
    date_now: function() {return Date.now()},
    host_env: function (ptr, len, rbuf, rcap) {
      const name = from_utf8 (wasmShared.memory, ptr, len);
      const v = process.env[name];
      if (v == null) return -1;
      return to_utf8 (wasmShared.memory, rbuf, rcap, v)},
    host_electrum_connect: function (ptr, len) {
      const args_s = from_utf8 (wasmShared.memory, ptr, len);
      const args = JSON.parse (args_s);
      const url = args.url;
      const protocol = args.protocol.toLowerCase();
      const disable_cert_verification = args.disable_cert_verification;
      const caps = /^(.*?):(\d+)$/.exec (url);
      if (caps == null) return -1;
      const host = caps[1];
      const port = Number (caps[2]);
      if (protocol != 'tls' && protocol != 'tcp') return -2;
      const ecl = new ElectrumCli (port, host, protocol);
      var ri = 0, ris = '';
      for (;;) {
        ri = Math.ceil (Math.random() * 2147483647);
        ris = '' + ri;  // TODO: Use a sparse array.
        if (wasmShared.electrums[ris] == null) {
          wasmShared.electrums[ris] = {
            host: host,
            port: port,
            ecl: ecl,
            connected: false,
            replies: []};
          break}}
      ecl.connect().then (_ => {wasmShared.electrums[ris].connected = true});
      // cf. https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-headers-subscribe
      ecl.subscribe.on ('blockchain.headers.subscribe', (header) => {
        console.log ('host_electrum_connect] TBD, Electrum header', header);
        // TODO: Pass the `header` to Rust?
        // The `header` looks like `[{hex: '040000', height: 175560}]` except the `hex` is large (seen ~3k).
      });
      return ri},
    host_electrum_is_connected: function (ri) {
      const ris = '' + ri;  // TODO: Use a sparse array.
      const en = wasmShared.electrums[ris];
      if (en == null) return -1;
      return en.connected ? 1 : 0},
    host_electrum_request: function (ri, ptr, len) {
      const ris = '' + ri;  // TODO: Use a sparse array.
      const en = wasmShared.electrums[ris];
      if (en == null) return -1;
      const req_s = from_utf8 (wasmShared.memory, ptr, len);
      const req = JSON.parse (req_s);  // JsonRpcRequest
      const id = Number (req.id);
      if (id > 2147483647) throw new Error ('Electrum JsonRpcRequest id is too large');
      //console.log ('host_electrum_request] ri', ri, 'req:', req);

      en.ecl.request (req.method, req.params)
        .then (res => {
          //console.log ('host_electrum_request] ri', ri, 'req', req, '⇒', res);
          const res_s = JSON.stringify (res);
          //console.log ('res_s', res_s);
          en.replies[id] = res_s;
          wasmShared.exports.electrum_replied (ri, id)})
        .catch (err => {
          wasmShared.exports.electrum_replied (ri, id)});
      return 0},
    host_electrum_reply: function (ri, id, rbuf, rcap) {
      const ris = '' + ri;  // TODO: Use a sparse array.
      const en = wasmShared.electrums[ris];
      if (en == null) return -2;
      const res_s = en.replies[id];
      if (res_s == null) return -3;
      return to_utf8 (wasmShared.memory, rbuf, rcap, res_s)},
    host_ensure_dir_is_writable: function (ptr, len) {
      const path = from_utf8 (wasmShared.memory, ptr, len);
      // https://nodejs.org/docs/latest/api/fs.html#fs_fs_existssync_path
      const exists = fs.existsSync (path);
      if (exists) {
        // https://nodejs.org/docs/latest/api/fs.html#fs_fs_statsync_path_options
        const stats = fs.statSync (path);
        if (!stats.isDirectory()) throw new Error ('path ' + path + ' is not a directory')
      } else {
        // https://nodejs.org/docs/latest/api/fs.html#fs_fs_mkdirsync_path_options
        fs.mkdirSync (path, {recursive: true})}
      return 0},
    host_read_dir: function (path_p, path_l, rbuf, rcap) {
      const path = from_utf8 (wasmShared.memory, path_p, path_l);
      const dir = fs.opendirSync (path);  // NB: Needs NodeJS >= 12.12.0
      let entries = [];
      for (;;) {
        const en = dir.readSync();
        if (en == null) break;
        if (!en.isFile()) continue;
        const name = en.name;
        const stats = fs.statSync (path + '/' + name);
        const lm = Math.floor (stats.mtimeMs);
        entries.push ([lm, name])}
      const jens = JSON.stringify (entries);
      console.log ('host_read_dir:', jens);
      return to_utf8 (wasmShared.memory, rbuf, rcap, jens)},
    host_rm: function (ptr, len) {
      const path = from_utf8 (wasmShared.memory, ptr, len);
      fs.unlinkSync (path);
      return 0},
    host_slurp: function (path_p, path_l, rbuf, rcap) {
      const path = from_utf8 (wasmShared.memory, path_p, path_l);
      if (!fs.existsSync (path)) return 0;
      const content = fs.readFileSync (path, {encoding: 'utf8'});
      return to_utf8 (wasmShared.memory, rbuf, rcap, content)},
    host_write: function (path_p, path_l, ptr, len) {
      const path = from_utf8 (wasmShared.memory, path_p, path_l);
      const content = new Uint8Array (wasmShared.memory.buffer, ptr, len);
      // https://nodejs.org/docs/latest/api/fs.html#fs_fs_writefilesync_file_data_options
      fs.writeFileSync (path, content);
      return 0},
    http_helper_check: function (http_request_id, rbuf, rcap) {
      let ris = '' + http_request_id;  // TODO: Use a sparse array.
      if (httpRequests[ris] == null) return -1;
      if (httpRequests[ris].buf == null) return -1;
      const ben = {
        status: httpRequests[ris].status,
        ct: httpRequests[ris].ct,
        cs: httpRequests[ris].cs,
        body: httpRequests[ris].buf};
      const buf = bencode.encode (ben);
      if (buf.length > rcap) return -buf.length;
      const rbuf_slice = new Uint8Array (wasmShared.memory.buffer, rbuf, rcap);
      for (let i = 0; i < buf.length; ++i) rbuf_slice[i] = buf[i];
      return buf.length},
    http_helper_if: function (helper_ptr, helper_len, payload_ptr, payload_len, timeout_ms) {
      const helper = from_utf8 (wasmShared.memory, helper_ptr, helper_len);
      //const payload = new Uint8Array (wasmShared.memory, payload_ptr, payload_len);
      const payload = Buffer.from (wasmShared.memory.buffer.slice (payload_ptr, payload_ptr + payload_len));

      // Find a random ID.
      let ri, ris;
      for (;;) {
        ri = Math.ceil (Math.random() * 2147483647);
        ris = '' + ri;  // TODO: Use a sparse array.
        if (httpRequests[ris] == null) {
          httpRequests[ris] = {};
          break}}

      let chunks = [];
      const req = http_helper (helper, timeout_ms, payload, (res) => {
        res.on ('data', (chunk) => chunks.push (chunk));
        res.on ('end', () => {
          let len = 0;
          for (const chunk of chunks) {len += chunk.length}
          if (res.headers['content-length'] != null && len != res.headers['content-length']) {
            throw new Error ('Content-Length mismatch')}
          const buf = new Uint8Array (len);
          let pos = 0;
          for (const chunk of chunks) {
            for (let i = 0; i < chunk.length; ++i) {
              buf[pos] = chunk[i];
              ++pos}}
          if (pos != len) throw new Error ('length mismatch');
          httpRequests[ris].status = res.statusCode;
          httpRequests[ris].ct = res.headers['content-type'];
          httpRequests[ris].cs = res.headers['x-helper-checksum'];
          httpRequests[ris].buf = buf;
          wasmShared.exports.http_ready (ri)});});
      req.on ('error', function (err) {
        httpRequests[ris].status = 0;
        httpRequests[ris].ct = 'nodejs error';
        httpRequests[ris].buf = '' + err;
        wasmShared.exports.http_ready (ri)});
      req.write (payload);
      req.end();
      return ri},  //< request id
    peers_drop_send_handler: function (shp1, shp2) {
      const payload = bencode.encode ([shp1, shp2]);
      const req = http_helper ('peers_drop_send_handler', 100, payload, (res) => {res.on ('end', () => {})});
      req.on ('error', function (_) {});
      req.write (payload);
      req.end()},
    temp_dir: function (rbuf, rcap) {return to_utf8 (wasmShared.memory, rbuf, rcap, os.tmpdir())}};

  const worker = new worker_threads.Worker ('./worker.js', {
    // Fails with `DOMException [DataCloneError]: #<Memory> could not be cloned.`
    //workerData: memory
  });
  worker.on ('error', (err) => {throw err});
  // Fails with `DOMException [DataCloneError]: #<Memory> could not be cloned.`
  //worker.on ('online', (_) => {worker.postMessage (memory)});

  const wasmInstantiated = await WebAssembly.instantiate (wasmBytes, {env: wasmEnv});
  const exports = wasmInstantiated.instance.exports;
  /** @type {WebAssembly.Memory} */
  wasmShared.memory = exports.memory;
  wasmShared.exports = exports;

  const executor_i = setInterval (function() {exports.run_executor()}, 200);

  exports.set_panic_hook();

  //await test_peers_dht();
  await trade_test_electrum_and_eth_coins();

  clearInterval (executor_i)}

async function test_peers_dht() {
  const peers_check = exports.peers_check();
  console.log ('wasm-run] test_peers_dht…');
  const test_finished = {};
  const cb_id = registerCallback (r => test_finished.yep = true);
  wasmShared.exports.test_peers_dht (cb_id);
  while (!test_finished.yep) {await snooze (100)}
  console.log ('wasm-run] test_peers_dht ✅')}

async function trade_test_electrum_and_eth_coins() {
  console.log ('wasm-run] trade_test_electrum_and_eth_coins…');
  const test_finished = {};
  const cb_id = registerCallback (r => test_finished.yep = true);
  wasmShared.exports.trade_test_electrum_and_eth_coins (cb_id);
  while (!test_finished.yep) {await snooze (100)}
  console.log ('wasm-run] trade_test_electrum_and_eth_coins ✅')}

function stop() {
  const req = http.request ({
    method: 'POST',
    hostname: '127.0.0.1',
    port: 7783,
    agent: keepAliveAgent,
  }, (res) => {});
  req.on ('error', function (_) {});
  req.write ('{"method": "stop", "userpass": "pass"}');
  req.end()}

// Start the native helpers.
const mm2 = spawn ('js/mm2', ['{"passphrase": "-", "rpc_password": "pass", "coins": []}'], {cwd: '..'});
mm2.stdout.on ('data', (data) => console.log ('native] ' + String (data) .trim()));
mm2.stderr.on ('data', (data) => console.log ('native] ' + String (data) .trim()));

runWasm().then (_ => stop()) .catch (ex => {console.log (ex); stop()});

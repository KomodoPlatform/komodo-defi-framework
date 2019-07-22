use common::{bits256, now_float, small_rng};
#[cfg(feature = "native")]
use common::wio::{drive, CORE};
use common::for_tests::wait_for_log_re;
use common::mm_ctx::{MmArc, MmCtxBuilder};
use crdts::CmRDT;
use futures::Future;
use rand::{self, Rng};
use serde_json::Value as Json;
use std::mem::zeroed;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

#[cfg(feature = "native")]
use crate::http_fallback::UniqueActorId;

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn ulimit_n() -> Option<u32> {
    let mut lim: libc::rlimit = unsafe {zeroed()};
    let rc = unsafe {libc::getrlimit (libc::RLIMIT_NOFILE, &mut lim)};
    if rc == 0 {
        Some (lim.rlim_cur as u32)
    } else {
        None
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn ulimit_n() -> Option<u32> {None}

fn peer (conf: Json, port: u16) -> MmArc {
    if let Some (n) = ulimit_n() {
      assert! (n > 2000, "`ulimit -n` is too low: {}", n)
    }

    let ctx = MmCtxBuilder::new().with_conf (conf) .into_mm_arc();
    unwrap! (ctx.log.thread_gravity_on());

    if let Some (seednodes) = ctx.conf["seednodes"].as_array() {
        let mut seeds = unwrap! (ctx.seeds.lock());
        assert! (seeds.is_empty());  // `fn lp_initpeers` was not invoked.
        assert! (!seednodes.is_empty());
        seeds.push (unwrap! (unwrap! (seednodes[0].as_str()) .parse()))
    }

    let mut key: bits256 = unsafe {zeroed()};
    small_rng().fill (&mut key.bytes[..]);
    unwrap! (super::initialize (&ctx, 9999, key, port));

    ctx
}

fn destruction_check (mm: MmArc) {
    mm.stop();
    if let Err (err) = wait_for_log_re (&mm, 1., "delete_dugout finished!") {
        // NB: We want to know if/when the `peers` destruction doesn't happen, but we don't want to panic about it.
        pintln! ((err))
    }
}

fn peers_exchange (conf: Json) {
    let fallback_on = conf["http-fallback"] == "on";
    let fallback = if fallback_on {1} else {255};

    let alice = peer (conf.clone(), 2111);
    let bob = peer (conf, 2112);

    if !fallback_on {
        unwrap! (wait_for_log_re (&alice, 99., r"\[dht-boot] DHT bootstrap \.\.\. Done\."));
        unwrap! (wait_for_log_re (&bob, 33., r"\[dht-boot] DHT bootstrap \.\.\. Done\."));
    }

    let tested_lengths: &[usize] = &[
        2222,  // Send multiple chunks.
        1,  // Reduce the number of chunks *in the same subject*.
        // 992 /* (1000 - bencode overhead - checksum) */ * 253 /* Compatible with (1u8..) */ - 1 /* space for number_of_chunks */
    ];
    let mut rng = small_rng();
    for message_len in tested_lengths.iter() {
        // Send a message to Bob.

        let message: Vec<u8> = (0..*message_len) .map (|_| rng.gen()) .collect();

        log! ("Sending " (message.len()) " bytes …");
        let sending_f = unwrap! (super::send (
            &alice, unwrap! (super::key (&bob)), b"test_dht", fallback, message.clone()));

        // Get that message from Alice.

        #[cfg(feature = "native")]
        fn fixed_validator (expect: Vec<u8>) -> Box<dyn Fn(&[u8])->bool + Send> {
            Box::new (move |payload| payload == &expect[..])
        }

        #[cfg(not(feature = "native"))]
        fn fixed_validator (expect: Vec<u8>) -> crate::FixedValidator {
            crate::FixedValidator::Exact (expect)
        }

        let receiving_f = super::recv (&bob, b"test_dht", fallback, fixed_validator (message.clone()));
        let received = unwrap! (receiving_f.wait());
        assert_eq! (received, message);

        if fallback_on {
            // TODO: Refine the log test.
            // TODO: Check that the HTTP fallback was NOT used if `!fallback_on`.
            unwrap! (wait_for_log_re (&alice, 0.1, r"transmit] TBD, time to use the HTTP fallback\.\.\."))
            // TODO: Check the time delta, with fallback 1 the delivery shouldn't take long.
        }

        let hn1 = crate::send_handlers_num();
        drop (sending_f);
        let hn2 = crate::send_handlers_num();
        if cfg! (feature = "native") {
            // Dropping SendHandlerRef results in the removal of the corresponding `Arc<SendHandler>`.
            assert! (hn1 > 0 && hn2 == hn1 - 1, "hn1 {} hn2 {}", hn1, hn2)
        } else {
            // `SEND_HANDLERS` only tracks the arcs in the native helper.
            assert! (hn1 == 0 && hn2 == 0, "hn1 {} hn2 {}", hn1, hn2)
        }
    }

    destruction_check (alice);
    destruction_check (bob);
}

/// Send and receive messages of various length and chunking via the DHT.
pub fn peers_dht() {peers_exchange (json! ({"dht": "on"}))}

// Temporarily exposed in order to experiment with portability helpers.
#[no_mangle]
pub extern fn test_peers_dht() {peers_dht()}

/// Using a minimal one second HTTP fallback which should happen before the DHT kicks in.
#[cfg(feature = "native")]
pub fn peers_http_fallback_recv() {
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let addr = SocketAddr::new (unwrap! ("127.0.0.1".parse()), 30204);
    let server = unwrap! (super::http_fallback::new_http_fallback (ctx.weak(), addr));
    CORE.spawn (move |_| server);

    peers_exchange (json! ({
        "http-fallback": "on",
        "seednodes": ["127.0.0.1"],
        "http-fallback-port": 30204
    }))
}

#[cfg(not(feature = "native"))]
pub fn peers_http_fallback_recv() {}

#[cfg(feature = "native")]
pub fn peers_direct_send() {
    use common::for_tests::wait_for_log;

    // Unstable results on our MacOS CI server,
    // which isn't a problem in general (direct UDP communication is a best effort optimization)
    // but is bad for the CI tests.
    // Might experiment more with MacOS in the future.
    if cfg! (target_os = "macos") {return}

    // NB: Still need the DHT enabled in order for the pings to work.
    let alice = peer (json! ({"dht": "on"}), 2121);
    let bob = peer (json! ({"dht": "on"}), 2122);

    let bob_key = unwrap! (super::key (&bob));

    // Bob isn't a friend yet.
    let alice_pctx = unwrap! (super::PeersContext::from_ctx (&alice));
    {
        let alice_trans = unwrap! (alice_pctx.trans_meta.lock());
        assert! (!alice_trans.friends.contains_key (&bob_key))
    }

    let mut rng = small_rng();
    let message: Vec<u8> = (0..33) .map (|_| rng.gen()) .collect();

    let _send_f = super::send (&alice, bob_key, b"subj", 255, message.clone());
    let recv_f = super::recv (&bob, b"subj", 255, Box::new (|_| true));

    // Confirm that Bob was added into the friendlist and that we don't know its address yet.
    {
        let alice_trans = unwrap! (alice_pctx.trans_meta.lock());
        assert! (alice_trans.friends.contains_key (&bob_key))
    }

    let bob_pctx = unwrap! (super::PeersContext::from_ctx (&bob));
    assert_eq! (0, alice_pctx.direct_pings.load (Ordering::Relaxed));
    assert_eq! (0, bob_pctx.direct_pings.load (Ordering::Relaxed));

    // Hint at the Bob's endpoint.
    unwrap! (super::investigate_peer (&alice, "127.0.0.1", 2122));

    // Direct pings triggered by `investigate_peer`.
    // NB: The sleep here is larger than expected because the actual pings start to fly only after the DHT initialization kicks in.
    unwrap! (wait_for_log (&bob.log, 22., &|_| bob_pctx.direct_pings.load (Ordering::Relaxed) > 0));
    // Bob's reply.
    unwrap! (wait_for_log (&alice.log, 22., &|_| alice_pctx.direct_pings.load (Ordering::Relaxed) > 0));

    // Confirm that Bob now has the address.
    let bob_addr = SocketAddr::new (Ipv4Addr::new (127, 0, 0, 1) .into(), 2122);
    {
        let alice_trans = unwrap! (alice_pctx.trans_meta.lock());
        assert! (alice_trans.friends[&bob_key].endpoints.contains_key (&bob_addr))
    }

    // Finally see if Bob got the message.
    unwrap! (wait_for_log (&bob.log, 1., &|_| bob_pctx.direct_chunks.load (Ordering::Relaxed) > 0));
    let start = now_float();
    let received = unwrap! (recv_f.wait());
    assert_eq! (received, message);
    assert! (now_float() - start < 0.1);  // Double-check that we're not waiting for DHT chunks.

    destruction_check (alice);
    destruction_check (bob);
}

/// Check the primitives used to communicate with the HTTP fallback server.  
/// These are useful in implementing NAT traversal in situations
/// where a truly distributed no-single-point-of failure operation is not necessary,
/// like when we're using the fallback server to drive a tested mm2 instance.
#[cfg(feature = "native")]
pub fn peers_http_fallback_kv() {
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let addr = SocketAddr::new (unwrap! ("127.0.0.1".parse()), 30205);
    let server = unwrap! (super::http_fallback::new_http_fallback (ctx.weak(), addr));
    CORE.spawn (move |_| server);

    // Wait for the HTTP server to start.
    thread::sleep (Duration::from_millis (20));

    // Insert several entries in parallel, relying on CRDT to ensure that no entry is lost.
    let entries = 9;
    let mut handles = Vec::with_capacity (entries);
    for en in 1 ..= entries {
        let unique_actor_id = (99 + en) as UniqueActorId;
        let key = fomat! ((en));
        let f = super::http_fallback::fetch_map (&addr, Vec::from (&b"test-id"[..]));
        let f = f.and_then (move |mut map| {
            let read_ctx = map.len();
            map.apply (
                map.update (
                    key,
                    read_ctx.derive_add_ctx (unique_actor_id),
                    |set, ctx| set.add ("1".into(), ctx)
                )
            );
            super::http_fallback::merge_map (&addr, Vec::from (&b"test-id"[..]), &map)
        });
        handles.push ((en, drive (f)))
    }
    for (en, f) in handles {
        let map = unwrap! (unwrap! (f.wait()));
        let _v = unwrap! (map.get (&fomat! ((en))) .val, "No such value: {}", en);
    }

    // See if all entries survived.
    let map = unwrap! (super::http_fallback::fetch_map (&addr, Vec::from (&b"test-id"[..])) .wait());
    for en in 1 ..= entries {
        let v = unwrap! (map.get (&fomat! ((en))) .val, "No such value: {}", en);
        let members = v.read().val;
        log! ("members of " (en) ": " [members]);
    }

    // TODO: Shut down the HTTP server as well.
    drop (ctx)
}

#[cfg(not(feature = "native"))]
pub fn peers_http_fallback_kv() {}

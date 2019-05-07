use common::{bits256, drive, CORE};
use common::for_tests::wait_for_log;
use common::mm_ctx::{MmArc, MmCtx};
use crdts::CmRDT;
use futures::Future;
use gstuff::now_float;
use rand::{self, Rng};
use serde_json::Value as Json;
use std::mem::zeroed;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use super::http_fallback::UniqueActorId;

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

    let ctx = MmCtx::new (conf);
    unwrap! (ctx.log.thread_gravity_on());
    let mut rng = rand::thread_rng();

    let mut alice_key: bits256 = unsafe {zeroed()};
    unsafe {rng.fill (&mut alice_key.bytes[..])}
    unwrap! (super::initialize (&ctx, 9999, alice_key, port, 0));

    ctx
}

fn destruction_check (mm: MmArc) {
    mm.stop();
    if let Err (err) = wait_for_log (&mm.log, 1., &|en| en.contains ("delete_dugout finished!")) {
        // NB: We want to know if/when the `peers` destruction doesn't happen, but we don't want to panic about it.
        pintln! ((err))
    }
}

pub fn peers_dht() {
    let alice = peer (json! ({"dht": "on"}), 2111);
    let bob = peer (json! ({"dht": "on"}), 2112);

    unwrap! (wait_for_log (&alice.log, 99., &|en| en.contains ("[dht-boot] DHT bootstrap ... Done.")));
    unwrap! (wait_for_log (&bob.log, 33., &|en| en.contains ("[dht-boot] DHT bootstrap ... Done.")));

    let tested_lengths: &[usize] = &[
        2222,  // Send multiple chunks.
        1,  // Reduce the number of chunks *in the same subject*.
        // 992 /* (1000 - bencode overhead - checksum) */ * 253 /* Compatible with (1u8..) */ - 1 /* space for number_of_chunks */
    ];
    let mut rng = rand::thread_rng();
    for message_len in tested_lengths.iter() {
        // Send a message to Bob.

        let message: Vec<u8> = (0..*message_len) .map (|_| rng.gen()) .collect();

        println! ("Sending {} bytes …", message.len());
        let _sending_f = super::send (&alice, unwrap! (super::key (&bob)), b"test_dht", message.clone());

        // Get that message from Alice.

        let receiving_f = super::recv (&bob, b"test_dht", Box::new ({
            let message = message.clone();
            move |payload| payload == &message[..]
        }));
        let received = unwrap! (receiving_f.wait());
        assert_eq! (received, message);
    }

    destruction_check (alice);
    destruction_check (bob);
}

pub fn peers_direct_send() {
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

    let mut rng = rand::thread_rng();
    let message: Vec<u8> = (0..33) .map (|_| rng.gen()) .collect();

    let _send_f = super::send (&alice, bob_key, b"subj", message.clone());
    let recv_f = super::recv (&bob, b"subj", Box::new (|_| true));

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

pub fn peers_http_fallback() {
    let ctx = MmCtx::new (json! ({}));
    let addr = SocketAddr::new (unwrap! ("127.0.0.1".parse()), 30204);
    let server = unwrap! (super::http_fallback::new_http_fallback (ctx.weak(), addr));
    CORE.spawn (move |_| server);
/*
    let alice = peer ({seed: server_ip, lt dht disabled, lt direct disabled});
    let bob = peer ({seed: server_ip, lt dht disabled, lt direct disabled});
    //With libtorrent DHT disabled the peers will hit the timeout that activates the HTTP fallback.

        println! ("Sending {} bytes …", message.len());
        let _sending_f = super::send (&alice, unwrap! (super::key (&bob)), b"test_dht", message.clone());

        let receiving_f = super::recv (&bob, b"test_dht", Box::new ({
            let message = message.clone();
            move |payload| payload == &message[..]
        }));
        let received = unwrap! (receiving_f.wait());
        assert_eq! (received, message);

    destruction_check (alice);
    destruction_check (bob);
*/
}

// Check the primitives used to communicate with the HTTP fallback server.  
// These are useful in implementing NAT traversal in situations
// where a truly distributed no-single-point-of failure operation is not necessary,
// like when we're using the fallback server to drive a tested mm2 instance.
pub fn peers_http_fallback_kv() {
    let ctx = MmCtx::new (json! ({}));
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
            super::http_fallback::merge_map (&addr, Vec::from (&b"test-id"[..]), map)
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

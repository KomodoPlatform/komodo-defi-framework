use common::bits256;
use common::for_tests::wait_for_log;
use common::mm_ctx::{MmArc, MmCtx};
use futures::Future;
use gstuff::now_float;
use rand::{self, Rng};
use serde_json::Value as Json;
use std::mem::zeroed;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;

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

pub fn test_peers_dht() {
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

pub fn test_peers_direct_send() {
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

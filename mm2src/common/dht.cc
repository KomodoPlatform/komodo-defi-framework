#include <algorithm>
#include <cassert>
#include <cstdint>
#include <cstdlib>  // getenv
#include <functional>
#include <iostream>
#include <sstream>
#include <stdexcept>
#include <vector>
#include <string.h>  // strdup
#include "libtorrent/address.hpp"
#include "libtorrent/alert_types.hpp"
#include "libtorrent/bencode.hpp"
#include "libtorrent/entry.hpp"
#include "libtorrent/kademlia/dht_state.hpp"
#include "libtorrent/kademlia/ed25519.hpp"
#include "libtorrent/kademlia/item.hpp"
#include "libtorrent/session.hpp"

/// A common context shared between the functions and tracked on the Rust side.
struct dugout_struct_t {
    lt::session* session;
    char const* err;

    dugout_struct_t(): session (nullptr), err (nullptr) {}
};
typedef struct dugout_struct_t dugout_t;

extern "C" char const* delete_dugout (dugout_t* dugout) try {
    if (dugout->session) {delete dugout->session; dugout->session = nullptr;}
    if (dugout->err) {free ((void*) dugout->err); dugout->err = nullptr;}
    return nullptr;
} catch (std::exception const& ex) {
    return strdup (ex.what());
}

extern "C" dugout_t dht_init (char const* listen_interfaces, bool read_only) {
    dugout_t dugout;

    try {
        // cf. https://www.libtorrent.org/reference-Settings.html
        lt::settings_pack sett;
        sett.set_bool (lt::settings_pack::enable_dht, false);
        sett.set_int (lt::settings_pack::alert_mask, 0x7fffffff);
        sett.set_str (lt::settings_pack::listen_interfaces, listen_interfaces);

        // cf. https://stackoverflow.com/a/31093221/257568, https://github.com/arvidn/libtorrent/issues/1489
        sett.set_bool (lt::settings_pack::prefer_rc4, true);
        sett.set_int (lt::settings_pack::out_enc_policy, lt::settings_pack::pe_forced);
        sett.set_int (lt::settings_pack::in_enc_policy, lt::settings_pack::pe_forced);
        sett.set_int (lt::settings_pack::allowed_enc_level, lt::settings_pack::pe_rc4);

        char* MM_DHT_NODES = std::getenv ("MM_DHT_NODES");
        if (MM_DHT_NODES != nullptr && *MM_DHT_NODES != 0) {
            sett.set_str (lt::settings_pack::dht_bootstrap_nodes, MM_DHT_NODES);
        } else {
            sett.set_str (lt::settings_pack::dht_bootstrap_nodes,
                // https://stackoverflow.com/a/32797766/257568
                "router.utorrent.com:6881"
                ",router.bittorrent.com:6881"
                ",dht.transmissionbt.com:6881"
                ",router.bitcomet.com:6881"
                ",dht.aelitis.com:6881");
        }

        lt::session* session = dugout.session = new lt::session (sett);

        lt::dht::dht_settings dsett;
        dsett.item_lifetime = 600;
        dsett.upload_rate_limit = 128 * 1024;
        dsett.read_only = read_only;
        session->set_dht_settings (dsett);
    } catch (std::exception const& ex) {
        dugout.err = strdup (ex.what());
    }
    return dugout;
}

extern "C" void dht_load_state (dugout_t* dugout, char const* dht_state, int32_t dht_state_len) try {
    if (!dugout->session) throw std::runtime_error ("Not initialized");
    lt::bdecode_node en;
    lt::error_code ec;
    int rc = lt::bdecode (dht_state, dht_state + dht_state_len, en, ec);
    if (rc) {
        std::ostringstream ss;
        ss << "Can't bdecode the DHT state: " << ec.message();
        throw std::runtime_error (ss.str());
    }
    dugout->session->load_state (en, lt::session::save_dht_state);
} catch (std::exception const& ex) {
    dugout->err = strdup (ex.what());
}

extern "C" void enable_dht (dugout_t* dugout) try {
    if (!dugout->session) throw std::runtime_error ("Not initialized");
    lt::settings_pack spack = dugout->session->get_settings();
    spack.set_bool (lt::settings_pack::enable_dht, true);
    dugout->session->apply_settings (spack);
} catch (std::exception const& ex) {
    dugout->err = strdup (ex.what());
}

extern "C" char* dht_save_state (dugout_t* dugout, int32_t* buflen) try {
    if (!dugout->session) throw std::runtime_error ("Not initialized");
    if (!dugout->session->is_dht_running()) throw std::runtime_error ("DHT is off");

    lt::entry en;
    dugout->session->save_state (en, lt::session::save_dht_state);

    std::vector<char> buf;
    lt::bencode (std::back_inserter (buf), en);

    char* cbuf = (char*) malloc (buf.size() + 1);
    if (cbuf == nullptr) {
        std::ostringstream ss;
        ss << "Error allocating " << (buf.size() + 1) << " bytes with malloc";
        throw std::runtime_error (ss.str());
    }
    std::copy (buf.begin(), buf.end(), cbuf);
    cbuf[buf.size()] = 0;
    *buflen = (int32_t) buf.size();
    return cbuf;
} catch (std::exception const& ex) {
    dugout->err = strdup (ex.what());
    return nullptr;
}

extern "C" void dht_alerts (dugout_t* dugout, void (*cb) (dugout_t*, void*, lt::alert*), void* cbctx) try {
    std::vector<lt::alert*> alerts;
    dugout->session->pop_alerts (&alerts);
    for (lt::alert* a : alerts) cb (dugout, cbctx, a);
} catch (std::exception const& ex) {
    dugout->err = strdup (ex.what());
}

extern "C" char const* alert_message (lt::alert const* alert) {
    std::ostringstream ss;
    ss << alert->type() << " (" << alert->what() << ") " << alert->message();
    return strdup (ss.str().c_str());
}

extern "C" bool is_dht_bootstrap_alert (lt::alert const* alert) {
    return alert->type() == lt::dht_bootstrap_alert::alert_type;
}

template <class A>
char const* endpoint_format (A const* alert) {
    std::ostringstream ss;
    ss << alert->address.to_string() << ':' << alert->port;
    if (alert->socket_type == lt::socket_type_t::tcp || alert->socket_type == lt::socket_type_t::tcp_ssl) ss << "@tcp";
    else if (alert->socket_type == lt::socket_type_t::udp) ss << "@udp";
    return strdup (ss.str().c_str());
}

extern "C" char const* as_listen_succeeded_alert (lt::alert const* alert) {
    if (alert->type() == lt::listen_succeeded_alert::alert_type)
        return endpoint_format (static_cast<lt::listen_succeeded_alert const*> (alert));
    return nullptr;
}

extern "C" char const* as_listen_failed_alert (lt::alert const* alert) {
    if (alert->type() == lt::listen_failed_alert::alert_type)
        return endpoint_format (static_cast<lt::listen_failed_alert const*> (alert));
    return nullptr;
}

extern "C" int32_t as_dht_mutable_item_alert (
    lt::alert const* alert,
    uint8_t* pkbuf, int32_t pkbuflen,
    int8_t* saltbuf, int32_t saltbuflen,
    uint8_t* buf, int32_t buflen,
    int64_t* seq, bool* auth
) {
    if (alert->type() != lt::dht_mutable_item_alert::alert_type) return 0;
    auto mia = static_cast<lt::dht_mutable_item_alert const*> (alert);

    *seq = mia->seq;
    *auth = mia->authoritative;

    // NB: This is not the `seed` but rather the public key generated from it with `ed25519_create_keypair`.
    assert (pkbuflen == 32);
    assert (pkbuflen == (int32_t) mia->key.size());
    std::copy (mia->key.begin(), mia->key.end(), pkbuf);

    if ((int32_t) mia->salt.size() + 1 > saltbuflen) return -1;
    std::copy (mia->salt.begin(), mia->salt.end(), saltbuf);
    saltbuf[mia->salt.size()] = 0;

    std::vector<char> v;
    lt::bencode (std::back_inserter (v), mia->item);
    if ((int32_t) v.size() > buflen) return -2;
    std::copy (v.begin(), v.end(), buf);
    return (int32_t) v.size();
}

extern "C" int32_t as_external_ip_alert (
    lt::alert const* alert,
    // Out: The new IP address. Supposedly triggers a DHT restart.
    uint8_t* ipbuf,
    // In: The length (= capacity) of the `ipbuf`.
    // Out: The length of the IP address copied to `ipbuf`.
    int32_t* ipbuflen
) {
    if (alert->type() != lt::external_ip_alert::alert_type) return 0;
    assert (ipbuf != nullptr && ipbuflen != nullptr && *ipbuflen > 0);

    auto eia = static_cast<lt::external_ip_alert const*> (alert);
    std::ostringstream ip_buf;
    ip_buf << eia->external_address;
    std::string ip = ip_buf.str();
    if ((int32_t) ip.size() > *ipbuflen) return -2;
    std::copy (ip.begin(), ip.end(), ipbuf);
    *ipbuflen = (int32_t) ip.size();
    return 1;
}

extern "C" int32_t as_dht_pkt_alert (
    lt::alert const* alert,
    // Out: The raw contents of the DHT packet. Usually bencoded.
    uint8_t* buf, int32_t buflen,
    // In: 1 .. interested in incoming packets. -1 .. in outgoing. 0 .. in both.
    // Out: 1 .. incoming packet. -1 .. outgoing packet.
    int8_t* direction,
    // Out: The IP address of the remote.
    uint8_t* ipbuf,
    // In: `ipbuf` capacity.
    // Out: The length of the IP address copied to `ipbuf`.
    int32_t* ipbuflen,
    // Out: Remote port.
    uint16_t* port
) {
    if (alert->type() != lt::dht_pkt_alert::alert_type) return 0;

    assert (buf != nullptr);
    assert (buflen > 0);
    assert (direction != nullptr);
    assert (ipbuf != nullptr);
    assert (ipbuflen != nullptr);
    assert (*ipbuflen != 0);
    assert (port != nullptr);

    auto dpa = static_cast<lt::dht_pkt_alert const*> (alert);
    int8_t packet_direction = dpa->direction == lt::dht_pkt_alert::direction_t::incoming ? 1 : -1;
    if (*direction != 0 && *direction != packet_direction) return 0;
    *direction = packet_direction;

    lt::span<char const> pkt = dpa->pkt_buf();
    if ((int32_t) pkt.size() > buflen) return -1;
    std::copy (pkt.begin(), pkt.end(), buf);

    std::ostringstream ip_buf;
    ip_buf << dpa->node.address();
    std::string ip = ip_buf.str();
    if ((int32_t) ip.size() > *ipbuflen) return -2;
    std::copy (ip.begin(), ip.end(), ipbuf);
    *ipbuflen = (int32_t) ip.size();
    *port = dpa->node.port();

    return (int32_t) pkt.size();
}

extern "C" void dht_put (dugout_t* dugout,
                         uint8_t const* key, int32_t keylen,
                         uint8_t const* salt_c, int32_t saltlen,
                         void (*callback) (void*, uint64_t, uint8_t*, int32_t, uint8_t**, int32_t*, int64_t*), void* arg, uint64_t arg2) {
    assert (key != nullptr);
    assert (keylen == 32);
    assert (salt_c != nullptr);
    assert (callback != nullptr);

    std::array<char, 32> seed;
    std::copy (key, key + keylen, seed.begin());

	lt::dht::public_key pk;
    lt::dht::secret_key sk;
    std::tie (pk, sk) = lt::dht::ed25519_create_keypair (seed);

    std::string salt ((char const*) salt_c, saltlen);

    dugout->session->dht_put_item (
        pk.bytes,
        [salt, pk, sk, callback, arg, arg2] (lt::entry& en, std::array<char, 64>& sig, std::int64_t& seq, std::string const&) {
            std::vector<char> have;
            lt::bencode (std::back_inserter (have), en);

            uint8_t* benload; int32_t benlen;
            callback (arg, arg2, (uint8_t*) have.data(), (int32_t) have.size(), &benload, &benlen, &seq);

            lt::span<char> benspan ((char*) benload, (lt::span<char>::difference_type) benlen);
            en = lt::bdecode (benspan);

            lt::dht::signature sign;
            sign = lt::dht::sign_mutable_item (benspan, salt, lt::dht::sequence_number (seq), pk, sk);
            sig = sign.bytes;
        },
        salt);
}

extern "C" void dht_get (dugout_t* dugout,
                         uint8_t const* key, int32_t keylen,
                         uint8_t const* salt_c, int32_t saltlen,
                         uint8_t* pkbuf, int32_t pkbuflen) {
    assert (keylen == 32);
    assert (salt_c != nullptr);
    assert (pkbuflen == 32);

    std::array<char, 32> seed;
    std::copy (key, key + keylen, seed.begin());

	lt::dht::public_key pk;
    lt::dht::secret_key sk;

    if (std::all_of (pkbuf, pkbuf + pkbuflen, [](uint8_t v) {return v == 0;})) {  // If `pkbuf` is zero.
        std::tie (pk, sk) = lt::dht::ed25519_create_keypair (seed);
        assert (pk.bytes.size() == 32);
        std::copy (pk.bytes.begin(), pk.bytes.end(), pkbuf);
    } else {  // Reuse the public key from `pkbuf`.
        assert (pk.bytes.size() == 32);
        std::copy (pkbuf, pkbuf + pkbuflen, pk.bytes.begin());
    }

    std::string salt ((char const*) salt_c, saltlen);

    dugout->session->dht_get_item (pk.bytes, salt);
}

extern "C" void lt_send_udp (dugout_t* dugout, char const* ip, uint16_t port, uint8_t const* benload, int32_t benlen) {
    lt::error_code ec;
    lt::address addr = lt::make_address (ip, ec);
    if (ec) {
        std::ostringstream ss;
        ss << "make_address error: " << ec;
        dugout->err = strdup (ss.str().c_str());
        return;
    }
    lt::udp::endpoint ep (addr, port);

    lt::span<char> benspan ((char*) benload, (lt::span<char>::difference_type) benlen);
    lt::entry en = lt::bdecode (benspan);

    void* userdata = nullptr;
    dugout->session->dht_direct_request (ep, en, userdata);
}

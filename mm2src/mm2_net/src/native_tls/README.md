# HTTPS Support with TLSAcceptor and Builder

This mod provides HTTPS support for [hyper](https://github.com/hyperium/hyper) using [rustls](https://github.com/rustls/rustls). The code in this mod is a port of the [acceptor](https://github.com/rustls/hyper-rustls/tree/286e1fa57ff5cac99994fab355f91c3454d6d83d/src/acceptor) module and the [acceptor.rs](https://github.com/rustls/hyper-rustls/blob/286e1fa57ff5cac99994fab355f91c3454d6d83d/src/acceptor.rs) file from the [hyper-rustls](https://github.com/rustls/hyper-rustls) repository at revision [286e1fa57ff5cac99994fab355f91c3454d6d83d](https://github.com/rustls/hyper-rustls/tree/286e1fa57ff5cac99994fab355f91c3454d6d83d).
> **Note:** Please be aware that the acceptor module was not available in the latest version of [hyper-rustls](https://docs.rs/hyper-rustls/0.24.0/hyper_rustls/index.html) at the time of writing this, the latest version was 0.24.0 at this time.

## Compatibility

The ported mod is compatible with hyper 0.14 and rustls 0.20.

## Purpose

The purpose of porting these files is to enable retrieving the remote address from the incoming connection and to expose the `TlsStream` type.
> **Note:** The following commit [fe6cd24](https://github.com/KomodoPlatform/komodo-defi-framework/commit/fe6cd24c760e4aad760201723b7c3b846309254d) show the changes applied to the ported code.
# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

### Native Build
```bash
# Basic build
cargo build

# Release build
cargo build --release

# Verbose output
cargo build -vv
```

### WASM Build

**Note**: WASM builds on macOS may encounter wasm-opt errors due to compatibility issues between wasm-pack 0.10.0 and the Rust toolchain when using sign extension and bulk memory features.

```bash
# Linux (typically works without issues)
wasm-pack build mm2src/mm2_bin_lib --target web --out-dir wasm_build/deps/pkg/

# macOS (Intel) - May fail with wasm-opt errors
CC=/usr/local/opt/llvm/bin/clang AR=/usr/local/opt/llvm/bin/llvm-ar wasm-pack build mm2src/mm2_bin_lib --target web --out-dir wasm_build/deps/pkg/

# macOS (Apple Silicon) - May fail with wasm-opt errors
CC=/opt/homebrew/opt/llvm/bin/clang AR=/opt/homebrew/opt/llvm/bin/llvm-ar wasm-pack build mm2src/mm2_bin_lib --target web --out-dir wasm_build/deps/pkg/
```

#### Alternative: Build with cargo and wasm-bindgen directly (Recommended for macOS)

If you encounter "Bulk memory operations require bulk memory" or sign extension errors on macOS, use this approach:

```bash
# macOS (Apple Silicon)
CC=/opt/homebrew/opt/llvm/bin/clang AR=/opt/homebrew/opt/llvm/bin/llvm-ar \
cargo build --release --target wasm32-unknown-unknown -p mm2_bin_lib

wasm-bindgen target/wasm32-unknown-unknown/release/kdflib.wasm \
--out-dir wasm_build/deps/pkg --target web

# macOS (Intel)
CC=/usr/local/opt/llvm/bin/clang AR=/usr/local/opt/llvm/bin/llvm-ar \
cargo build --release --target wasm32-unknown-unknown -p mm2_bin_lib

wasm-bindgen target/wasm32-unknown-unknown/release/kdflib.wasm \
--out-dir wasm_build/deps/pkg --target web
```

This bypasses wasm-opt entirely, producing a larger but functionally identical WASM file. The only difference is the lack of wasm-opt's size optimizations - all Rust release optimizations are still applied.

Note: This workaround is only needed for Rust 1.72+ with wasm-pack 0.10.0. Updating to a newer Rust toolchain and compatible wasm-pack version would eliminate the need for this workaround.

### Docker Build
```bash
docker build -t kdf-build-container -f .docker/Dockerfile .
docker run -v "$(pwd)":/app -w /app kdf-build-container cargo build
```

## Testing

### Run All Tests
```bash
cargo test --all --features run-docker-tests -- --test-threads=16
```

### Unit Tests Only
```bash
cargo test --bins --lib --no-fail-fast
```

### Integration Tests
```bash
cargo test --test 'mm2_tests_main' --no-fail-fast
```

### Docker Tests
```bash
cargo test --test 'docker_tests_main' --features run-docker-tests --no-fail-fast
```

### WASM Tests
```bash
# Set environment variables first
export WASM_BINDGEN_TEST_TIMEOUT=180
export GECKODRIVER=PATH_TO_GECKO_DRIVER_BIN
export BOB_PASSPHRASE="also shoot benefit prefer juice shell elder veteran woman mimic image kidney"
export ALICE_PASSPHRASE="spice describe gravity federal blast come thank unfair canal monkey style afraid"

# Run tests
wasm-pack test --firefox --headless mm2src/mm2_main
```

### Run Specific Test
```bash
# Example for a specific test module
cargo test --package coins --lib utxo::utxo_tests::test_function_name
```

## Linting and Formatting

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt -- --check

# Run clippy
cargo clippy -- -D warnings

# Clippy for all targets
cargo clippy --all-targets --all-features -- --D warnings

# WASM-specific clippy
cargo clippy --target wasm32-unknown-unknown -- --D warnings
```

## Architecture Overview

The Komodo DeFi Framework is a decentralized exchange engine built in Rust that enables atomic swaps between different blockchain assets.

### Core Components

#### 1. Swap Protocol (`mm2src/mm2_main/src/lp_swap/`)
The heart of the DEX - implements atomic swaps using HTLCs (Hash Time Lock Contracts). Key files:
- `maker_swap.rs` / `maker_swap_v2.rs`: Maker (liquidity provider) swap logic
- `taker_swap.rs` / `taker_swap_v2.rs`: Taker (liquidity consumer) swap logic
- `swap_lock.rs`: Prevents conflicting swaps
- `swap_watcher.rs`: Monitors and can recover stuck swaps

Swap flow: Negotiation → TakerFee → MakerPayment → TakerPayment → SecretReveal → Completion

#### 2. Coin Integration (`mm2src/coins/`)
Abstractions for different blockchain types:
- `utxo/`: Bitcoin-like chains (BTC, LTC, DOGE, etc.)
- `eth/`: Ethereum and ERC20 tokens
- `tendermint/`: Cosmos-based chains
- `lightning/`: Lightning Network integration
- `z_coin/`: Zcash shielded transactions

Each coin type implements the `MmCoin` trait providing unified interface for balance queries, transaction building, and swap operations.

#### 3. P2P Network (`mm2src/mm2_p2p/`)
Built on libp2p, handles:
- Order broadcasting via GossipSub
- Direct peer communication via Request/Response
- Peer discovery and NAT traversal
- Message signing and verification

Topics are used to namespace different message types (orderbook updates, swap messages, etc.).

#### 4. Order Management (`mm2src/mm2_main/src/lp_ordermatch/`)
Distributed orderbook system:
- Each node maintains its own orderbook view
- Orders are propagated via P2P network
- Trie-based synchronization for consistency
- Best order matching algorithm

#### 5. Event System (`mm2src/mm2_event_stream/`)
Modern event streaming for real-time updates:
- SSE (Server-Sent Events) for web clients
- Configurable event subscriptions
- Covers swaps, orders, balances, network events

### Key Patterns

- **MmCtx**: Central context object containing configuration, coin instances, and shared state
- **State Machines**: Swaps use persistent state machines for crash recovery
- **Storage Abstraction**: SQLite for native, IndexedDB for WASM
- **Error Handling**: Extensive Result types with custom error contexts
- **Async/Actor Model**: Components communicate via channels and message passing

### Module Hierarchy

```
mm2_main (application core)
├── coins (blockchain implementations)
├── coins_activation (coin initialization)
├── mm2_p2p (networking)
├── mm2_event_stream (events)
├── mm2_rpc (API interface)
└── mm2_db (storage)
```

### Important Configuration

- `MM2.json`: Main config with RPC credentials and seed phrase
- `coins`: Coin configurations (use official Komodo coins repo)
- `.env.client` / `.env.seed`: Test environment passphrases
- Requires ZCash params files for shielded operations

### Development Tips

- The codebase uses workspace dependencies - check root `Cargo.toml`
- Default toolchain: `nightly-2023-06-01`
- For macOS: May need to configure loopback interfaces for tests
- Docker/Podman required for integration tests
- Use `--test-threads=16` for faster test execution
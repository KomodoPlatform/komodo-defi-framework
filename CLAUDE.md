# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Effective Code Search and Retrieval (RAG) Strategies

When working with the Komodo DeFi Framework codebase, follow this systematic approach for efficient code discovery and understanding.

### 0. DeepWiki MCP - Your First Stop for Code Discovery

**ALWAYS try DeepWiki MCP first before manual searching!** DeepWiki provides AI-powered code understanding for the KDF repository.

#### Using DeepWiki MCP

The DeepWiki MCP server is already configured for this project. Use these tools:

```bash
# Query the codebase with natural language
mcp__deepwiki__ask_question: "How does the maker swap protocol work?"

# Get repository structure and documentation overview
mcp__deepwiki__read_wiki_structure: "KomodoPlatform/komodo-defi-framework"

# Read specific wiki page contents
mcp__deepwiki__read_wiki_contents: "KomodoPlatform/komodo-defi-framework"
```

#### When to Use DeepWiki vs Manual Search

**Use DeepWiki when:**
- Starting fresh with a new concept or module
- Need high-level understanding of architecture
- Want to understand relationships between components
- Looking for implementation patterns and best practices
- Need quick overview before diving into details
- Want comprehensive answers about implementation details (ask_question provides full context)
- Need to explore available documentation topics (read_wiki_structure)
- Looking for conceptual explanations of complex features

**Use Manual Search when:**
- DeepWiki results need more specific details
- Need to see exact implementation code
- Looking for recent changes not yet indexed
- Debugging specific line-by-line issues
- Need to verify DeepWiki's suggestions

**Combine Both Approaches:**
1. Start with DeepWiki for conceptual understanding
2. Use manual search to find exact files mentioned by DeepWiki
3. Read specific implementations for detailed understanding
4. Validate with tests and examples

Example workflow:
```bash
# Step 1: Ask DeepWiki for overview
mcp__deepwiki__ask_question: "How do I add a new RPC endpoint in KDF?"

# Step 2: Based on DeepWiki's response, search for specific examples
Grep: "experimental::" path="**/dispatcher.rs"

# Step 3: Read the exact implementation
Read: "mm2src/mm2_main/src/rpc/lp_commands/experimental/your_example.rs"
```

### Search Decision Tree

```
Start Here → What do you need?
├── Understanding concepts → Try DeepWiki ask_question first, then Hierarchical Search
├── Finding specific code → Try DeepWiki ask_question, then Direct Search Patterns
├── Adding new functionality → DeepWiki for patterns, then Implementation Discovery
├── Debugging an issue → Manual Error Tracing (DeepWiki for context)
├── Exploring documentation → DeepWiki read_wiki_structure for available topics
└── Finding examples → DeepWiki + Example Discovery Patterns
```

### DeepWiki Response Insights

**Understanding DeepWiki Responses:**
- DeepWiki provides comprehensive answers with references to specific files and functions
- Responses include wiki page links for deeper exploration (e.g., `/wiki/KomodoPlatform/komodo-defi-framework#3.4`)
- Each response ends with a DeepWiki search URL for sharing or reference
- DeepWiki may reference trait names, function paths, and module structures that can be verified with manual search

**Follow-up Actions After DeepWiki:**
1. Note specific file paths and function names mentioned
2. Use Grep/Read to verify exact implementations
3. Check for recent changes not in DeepWiki's index
4. Explore suggested wiki sections for related topics

### 1. Hierarchical Search Workflow

Always search from broad to specific to ensure you don't miss important context:

```bash
# Level 1: Understand the module structure
Glob: "**/module_name/**/*.rs"
LS: "/path/to/module"

# Level 2: Find key types and traits
Grep: "^pub struct|^pub enum|^pub trait" in module
Task: "Find all public types in module X"

# Level 3: Find specific implementations
Grep: "impl.*TraitName.*for"
Read: specific files found in previous steps

# Level 4: Validate understanding
Grep: "#[test]" in module  # Find tests that demonstrate usage
```

### 2. Tool Selection Guide

**Use Task Tool when:**
- You need to understand how a complex system works
- Searching across multiple interconnected files
- Need comprehensive analysis with context
- Example: `Task: "Explain how maker swaps work, find all relevant files"`

**Use Grep when:**
- You know specific patterns to search for
- Need to find all occurrences of something
- Quick searches across codebase
- Example: `Grep: "fn handle_mmrpc"`

**Use Glob when:**
- Finding files by name pattern
- Exploring directory structures
- Locating test or module files
- Example: `Glob: "**/swap*test*.rs"`

**Use Read when:**
- You know the exact file
- Need to understand full context
- Following up on search results
- Example: `Read: "/path/to/specific/file.rs"`

**Combine Tools for Power:**
```bash
# First: Find all files related to a feature
Glob: "**/*order*match*.rs"

# Second: Search for specific patterns in those files
Grep: "OrderbookItem" include="*ordermatch*.rs"

# Third: Read the most relevant file
Read: "mm2src/mm2_main/src/lp_ordermatch.rs"
```

### 3. Code Discovery Patterns

#### Finding RPC Implementations

```bash
# Step 1: Find where RPC is registered
Read: "mm2src/mm2_main/src/rpc/dispatcher/dispatcher.rs"
# Look for: "method_name" => handle_mmrpc(ctx, request, actual_handler_function)

# Step 2: Find the handler function
Grep: "async fn handler_function_name"

# Step 3: Find request/response types
Grep: "struct.*Request|struct.*Response" near the handler

# Step 4: Find tests
Grep: "method_name" include="*test*.rs"
```

#### Finding Coin Implementations

```bash
# Step 1: Find coin module
Glob: "**/coins/**/coin_name*.rs"

# Step 2: Find trait implementations
Grep: "impl (MmCoin|SwapOps|MarketCoinOps) for CoinName"

# Step 3: Find activation logic
Grep: "coin_name" path="**/coins_activation/**"

# Step 4: Find coin-specific tests
Glob: "**/coins/**/coin_name*test*.rs"
```

#### Finding Error Handling

```bash
# Step 1: Find error types in module
Grep: "#[derive.*Error.*]|enum.*Error" path="module_path"

# Step 2: Find error conversions
Grep: "impl From.*for.*Error"

# Step 3: Find error usage
Grep: "\.map_err|MmError|\.mm_err"
```

### 4. Implementation Discovery Workflow

When adding new functionality similar to existing code:

```bash
# Step 1: Find similar implementations
Task: "Find all implementations similar to [feature]"

# Step 2: Identify the pattern
- Note file locations
- Note naming conventions  
- Note trait implementations
- Note test patterns

# Step 3: Find the simplest example
Grep: "impl.*for.*" include="*simple*|*basic*"

# Step 4: Trace dependencies
- Follow imports in the simple example
- Find required traits
- Find helper functions

# Step 5: Find integration points
- Where is it registered/initialized?
- Where is it called from?
- What depends on it?
```

### 5. Search Result Validation

Always validate your search results:

```bash
# After finding code, verify:
1. Is this the latest version? (check git history if needed)
2. Is this used in production? (find callers)
3. Are there tests? (find test files)
4. Is there newer alternative? (search for "deprecated", "legacy", "v2")

# Validation searches:
Grep: "function_name\(" # Find all callers
Grep: "#[deprecated]|// deprecated|DEPRECATED"
Grep: "todo!|unimplemented!|unreachable!" # Find incomplete code
```

### 6. Common Search Patterns for KDF

#### RPC Patterns
```bash
# Find all RPC methods
Grep: '"[a-z_]+" => handle_mmrpc' path="**/dispatcher.rs"

# Find streaming RPC methods  
Grep: 'stream::' path="**/dispatcher.rs"

# Find task-based RPC methods
Grep: 'task::' path="**/dispatcher.rs"

# Find experimental methods
Grep: 'experimental::' path="**/dispatcher.rs"
```

#### Trait Implementation Patterns
```bash
# Find all coins implementing a trait
Grep: "impl.*MmCoin.*for"

# Find all swap implementations
Grep: "impl.*SwapOps.*for"

# Find all HD wallet implementations
Grep: "impl.*HDWalletCoinOps.*for"
```

#### Test Patterns
```bash
# Find unit tests
Grep: "#[test]"

# Find integration tests
Glob: "**/tests/**/*.rs"

# Find docker tests
Grep: "#[cfg(feature = \"run-docker-tests\")]"

# Find wasm tests
Grep: "#[wasm_bindgen_test]"
```

### 7. Troubleshooting Failed Searches

**Too many results?**
```bash
# Add more context
Grep: "pattern" include="*.rs" path="specific/module"

# Use more specific patterns
Grep: "^pub fn function_name" # Start of line
Grep: "struct Name {" # Exact match
```

**Too few/no results?**
```bash
# Try variations
- Singular vs plural (order vs orders)
- Underscores vs camelCase
- Abbreviations (tx vs transaction)

# Broaden search
Grep: "partial_name"
Task: "Find anything related to [concept]"

# Check for aliases/renames
Grep: "type.*=.*OriginalName"
Grep: "use.*as"
```

**Can't find implementation?**
```bash
# It might be generated
Grep: "#[derive" # Derived traits
Grep: "macro_rules"

# It might be in a dependency
Read: "Cargo.toml" # Check external crates

# It might be feature-gated
Grep: "#[cfg(feature"
```

### 8. Real-World Search Workflow Example

**Task: Add a new RPC endpoint called "get_node_status"**

```bash
# Step 1: Understand RPC structure
Read: "mm2src/mm2_main/src/rpc/dispatcher/dispatcher.rs"
# Found: RPC methods are registered in match statement

# Step 2: Find simple RPC example to copy
Grep: "get_enabled_coins" path="**/dispatcher.rs"
# Found: "get_enabled_coins" => handle_mmrpc(ctx, request, get_enabled_coins)

# Step 3: Find the handler implementation
Grep: "pub async fn get_enabled_coins"
# Found: mm2src/coins/rpc_command/get_enabled_coins.rs

# Step 4: Understand the pattern
Read: "mm2src/coins/rpc_command/get_enabled_coins.rs"
# Found: Request struct, Response struct, Error enum, handler function

# Step 5: Find where new RPCs should go
LS: "mm2src/mm2_main/src/rpc/lp_commands"
# Found: This is where new RPC commands go

# Step 6: Check for experimental namespace
Grep: "experimental::" path="**/dispatcher.rs"  
# Found: Experimental RPCs use experimental:: prefix

# Step 7: Find tests for similar RPC
Grep: "get_enabled_coins" include="*test*.rs"
# Found: Test patterns to follow

# Step 8: Verify no existing implementation
Grep: "node_status|node_info|get_node"
# Found: No existing implementation
```

### 9. Performance Tips

1. **Batch similar searches:**
   ```bash
   # Good: One Task call
   Task: "Find all swap-related modules, their main types, and entry points"
   
   # Avoid: Multiple separate searches
   Grep: "swap"
   Grep: "Swap" 
   Grep: "SWAP"
   ```

2. **Use include/exclude patterns:**
   ```bash
   Grep: "pattern" include="*.rs" exclude="*test*"
   ```

3. **Start with most specific known path:**
   ```bash
   # If you know the module
   Grep: "pattern" path="mm2src/coins"
   ```

4. **Cache mental model:**
   - Remember key file locations
   - Note naming conventions
   - Track module relationships

### 10. Common Gotchas and Solutions

Based on real implementation experience:

#### Import Path Confusion
```bash
# Wrong: Assuming crate name matches import path
use mm2_p2p::P2PContext;  # ❌ Won't work

# Correct: Check actual import paths in existing code
Grep: "use.*P2PContext"
# Found: use mm2_libp2p::p2p_ctx::P2PContext; ✅
```

#### API Assumptions
```bash
# Don't assume method names - always verify:
# Assumed: p2p_ctx.connected_peers_len()  ❌
# Reality: Need to use mm2_libp2p::get_directly_connected_peers()

# Always check available methods:
Grep: "impl.*P2PContext" 
Bash: "grep -E 'pub.*fn' path/to/impl/file.rs"
```

#### Context Availability
```rust
// Wrong: Assuming context always exists
let p2p = P2PContext::fetch_from_mm_arc(&ctx); // May panic!

// Right: Check if context exists first
if ctx.p2p_ctx.lock().unwrap().is_some() {
    let p2p = P2PContext::fetch_from_mm_arc(&ctx);
}
```

#### Module Organization
```bash
# New modules need proper registration:
1. Create the module file
2. Add to parent mod.rs: pub(crate) mod module_name;
3. Import in dispatcher if needed
```

#### Experimental vs Stable
- Experimental RPCs: Handle in `experimental_rpcs_dispatcher`
- Method name gets `experimental::` prefix automatically
- Don't add prefix manually in the match statement

#### Verifying Import Paths
- Never assume import paths - always verify with `Grep` on existing code
- Types are often re-exported at different module levels
- Functions in submodules may require full path: `module::submodule::function`
- Workspace crate names may differ from their import paths

#### Type Conversion Patterns
- Fixed-size array conversions often require explicit copying
- When multiple trait implementations exist, explicit type hints may be needed
- `Display` trait implementations often handle specialized formatting

#### Offline vs Online Operations
- Some functions require network/service connections, others work offline
- Check function requirements before using - offline alternatives often exist
- Configuration data is usually available without activation
- Look for utility functions that work with just configuration data

### 11. Learning Loop - IMPORTANT

**At the end of EVERY task, ALWAYS ask:**

> "Based on this implementation, are there any patterns, gotchas, or insights I discovered that should be added to CLAUDE.md to help with future tasks?"

This ensures continuous improvement of the documentation. Examples of learnings to capture:
- Unexpected API behaviors
- Common compilation errors and fixes
- Module organization patterns
- Import path discoveries
- Testing patterns
- Build system quirks

**Format for adding learnings:**
1. Add to relevant existing section if applicable
2. Create new subsection in "Common Gotchas and Solutions" for specific issues
3. Update code examples with real working code
4. Include error messages and solutions

## Project Overview

The Komodo DeFi Framework (KDF) is a decentralized exchange (DEX) engine that enables peer-to-peer atomic swaps between different blockchain assets without intermediaries. It serves as the backend for various DEX applications and provides:

- Cross-chain atomic swaps using HTLCs (Hash Time Lock Contracts)
- Support for 100+ blockchain protocols and thousands of tokens
- Non-custodial trading (users control their private keys)
- Order matching without central servers
- Hardware wallet integration (Trezor, Ledger)
- WASM support for browser-based applications

## Key Concepts

Before working with this codebase, understand these domain-specific terms:

- **Atomic Swap**: A peer-to-peer exchange of cryptocurrencies from different blockchains without intermediaries
- **HTLC**: Hash Time Lock Contract - a smart contract that enables atomic swaps
- **Maker**: The party that creates and broadcasts an order (provides liquidity)
- **Taker**: The party that accepts an existing order (takes liquidity)
- **TPU (Trading Protocol Upgrade)**: Version 2 of the swap protocol that solves the back-out problem
- **Back-out Problem**: When a maker doesn't send their payment after the taker has paid the dex fee
- **Dex Fee**: A small fee (1/777 of trade volume) that prevents spam and funds development
- **Order Matching**: The P2P process of finding and matching compatible trade orders
- **Electrum**: A protocol for lightweight Bitcoin/UTXO clients used for blockchain interaction

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

## Repository Conventions

### Commit Messages
Follow conventional commits format:
- `feat(module):` - New features
- `fix(module):` - Bug fixes  
- `refactor(module):` - Code refactoring
- `test(module):` - Test changes
- `docs(module):` - Documentation updates
- `chore(module):` - Maintenance tasks
- `perf(module):` - Performance improvements

Examples:
- `feat(coins): add Solana integration`
- `fix(swap): handle timeout in maker payment`
- `refactor(error): replace try_s! with MmError in coins module`

### Pull Request Process
1. Create feature branch from `dev`
2. Make atomic commits following conventional format
3. Ensure all tests pass
4. Update relevant documentation
5. Request review from at least one team member

## Critical Development Rules

### RPC Development
- **ALWAYS** use v2 RPC structure for new endpoints
- **NEVER** modify anything in `dispatcher_legacy.rs` or v1 endpoints
- **ALWAYS** add new RPCs to experimental namespace first: `mm2src/mm2_main/src/rpc/lp_commands/experimental/`
- Move to stable namespace only after thorough testing and team approval
- Maintain backward compatibility for all existing endpoints
- **COMBINE** similar functionalities into single RPCs using enums rather than creating multiple separate endpoints
  - Use tagged enums (with serde's `tag` attribute) to differentiate operation modes
  - Examples: `lightning/send_payment` (Invoice vs Keysend), `tokens` (different token types)
  - This reduces API surface area and improves maintainability

### Error Handling Transition
- Moving toward idiomatic Rust error handling:
  - One public error type per crate using `MmError` (e.g., `coins::CoinError`, `mm2_main::MainError`)
  - Module-specific error variants within the crate error type
  - Use `MmError` framework, not `thiserror`
  - Proper error conversion with `From` implementations
- **USE**: `MmError` with `?` operator
- **DON'T USE**: `try_s!`, `ERR!`, `ERRL!` macros (deprecated - see issue #1250)
- When refactoring, replace error macros incrementally with proper error types
- RPC errors need `HttpStatusCode` implementation and proper serialization attributes

### Rust Version Migration
- Project is transitioning from `nightly-2023-06-01` to stable Rust
- Check for nightly-only features before using them
- Prefer stable Rust patterns and features

### Module Structure Goals
- We're moving toward generic traits and modular architecture
- New code should favor trait-based abstractions over concrete implementations
- Keep modules focused and single-purpose

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
export WASM_BINDGEN_TEST_TIMEOUT=600
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

## Guidelines for Claude

### Common Pitfalls to Avoid
- **Error Handling**: Always use `MmError` with `?`, never use `try_s!`, `ERR!`, or `ERRL!`
- **Imports**: Use absolute imports from crate root, not relative imports
- **Module Placement**: 
  - New RPC endpoints go in `mm2src/mm2_main/src/rpc/lp_commands/`
  - Never add to `dispatcher_legacy.rs`
  - Coin-specific logic stays in `mm2src/coins/`

### Code Review Checklist
When reviewing code, check for:
- [ ] Proper error handling with MmError
- [ ] No unnecessary derives
- [ ] Absolute imports used
- [ ] Tests included for new functionality
- [ ] Documentation for public APIs
- [ ] No sensitive data in logs
- [ ] Platform-specific code uses `cfg_native!` / `cfg_wasm32!`

## Code Review Standards

Claude should check all code for:

### Performance
- [ ] No unnecessary clones or allocations
- [ ] Efficient async patterns (avoid blocking operations)
- [ ] Proper use of references vs ownership
- [ ] No N+1 query problems in database operations

### Security
- [ ] No private keys or sensitive data in logs
- [ ] Input validation on all RPC endpoints
- [ ] Proper error messages (don't leak internal details)
- [ ] Safe handling of user-provided data
- [ ] Check for potential DoS vectors

### Testing
- [ ] Unit tests for new functions
- [ ] Integration tests for new RPC endpoints
- [ ] Docker tests for cross-chain functionality
- [ ] Edge cases covered (empty inputs, max values, etc.)

### Documentation
- [ ] Public APIs have doc comments with examples
- [ ] Complex logic has explanatory comments
- [ ] Update CLAUDE.md if patterns change
- [ ] Update relevant docs/ files

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

The framework supports two swap protocols:
- **Legacy (v1)**: Taker sends dex fee upfront (irrecoverable if maker backs out)
  - Flow: Negotiation → TakerFee → MakerPayment → TakerPayment → SecretReveal → Completion
- **TPU (v2)**: Trading Protocol Upgrade that solves the back-out problem
  - Flow: Negotiation → TakerFunding → MakerPayment → TakerPayment → MakerSpendsTakerPayment → TakerSpendsMakerPayment
  - Dex fees are only collected on successful swaps (refundable on failure)
  - Supports maker rewards/premiums
  - Provides immediate refund paths

#### 2. Coin Integration (`mm2src/coins/`)
Each blockchain protocol has its own module implementing common traits (`MmCoin`, `MarketCoinOps`, `SwapOps`) for unified operations.

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
- SSE (Server-Sent Events) for native clients
- SharedWorker-based streaming for WASM clients
- Configurable event subscriptions
- Covers swaps, orders, balances, network events

### Key Patterns

- **MmCtx**: Central context object containing configuration, coin instances, and shared state
- **State Machines**: TPU (v2) swaps use persistent state machines for crash recovery. Legacy swaps use SavedSwap structs
- **Storage Abstraction**: SQLite for native, IndexedDB for WASM
- **Error Handling**: Extensive Result types with custom error contexts
- **Async Architecture**: Heavy use of tokio async runtime with channels (mpsc) for component communication

### Module Hierarchy

```
mm2_main (application core)
├── coins (blockchain implementations)
├── coins_activation (coin initialization)
├── mm2_p2p (networking layer)
├── mm2_event_stream (event system)
├── mm2_net (HTTP/WebSocket transport)
├── mm2_rpc (RPC protocol definitions)
├── mm2_db (IndexedDB for WASM)
├── mm2_core (core context and types)
├── mm2_metamask (MetaMask integration)
├── rpc_task (async RPC task management)
├── trading_api (external DEX integrations)
├── crypto (key management and crypto ops)
├── db_common (SQL abstraction layer)
├── mm2_err_handle (error handling utilities)
├── mm2_number (numeric types)
├── mm2_bitcoin/* (Bitcoin and UTXO forks protocol implementation)
├── trezor (hardware wallet support)
├── ledger (hardware wallet support)
└── common (shared utilities)
```

### Important Configuration

- `MM2.json`: Main config with RPC credentials and seed phrase
- `coins`: Coin configurations (use official Komodo coins repo)
- `.env.client` / `.env.seed`: Test environment passphrases
- Requires ZCash params files for shielded operations

### Code Navigation

- **RPC**: Check `dispatcher.rs` for v2 endpoints, `dispatcher_legacy.rs` for v1 (read-only)
- **Swaps**: `mm2_main/src/lp_swap/`
- **Coins**: `coins/` with subdirs per protocol type
- **Orders**: `mm2_main/src/lp_ordermatch/`
- **P2P**: `mm2_p2p/src/`
- **Events**: `mm2_event_stream/src/`
- **Activation**: `coins_activation/src/` (v2), `legacy.rs` (v1)

### Common Development Tasks

When implementing features, I'll discover the specific patterns from existing code. Key principles:
- New RPCs go in `lp_commands/experimental/` first
- Follow existing patterns in the module you're working in
- Check test files for examples

#### Adding Support for a New Blockchain Protocol

When integrating a completely new blockchain protocol, the complexity varies significantly based on protocol characteristics:

**Protocol Types**:
- **Native HTLC Support**: Protocols with built-in timelock and hash conditions (like UTXO-based chains)
- **Smart Contract HTLC**: Protocols requiring smart contracts for atomic swaps (like Ethereum)
- **Token Support**: Protocols that support additional tokens on the same blockchain
- **Single Asset**: Protocols that only support the native coin

**Implementation Strategy**: Start with minimum required for coin activation - implement trait methods as `todo!()` or return errors, then implement features incrementally.

**Minimum Required for Activation**:
1. Basic `MarketCoinOps` methods:
   - `ticker()` - return coin ticker
   - `my_address()` - return user's address  
   - `my_balance()` - return current balance
   - `current_block()` - return current block height
   - Other methods can return `unimplemented!()` or errors
2. Coin struct that converts `Into<MmCoinEnum>`
3. Choose activation pattern:
   - **Standalone coins**: Implement `InitStandaloneCoinActivationOps`
   - **Platform coins with tokens**: Implement `PlatformCoinWithTokensActivationOps`
4. Activation result must implement `CurrentBlock` trait

**Note**: Coin registration (`lp_register_coin`) is handled automatically by the generic init methods.

**Full Implementation Requirements**:
- Core traits: `MmCoin` (combines `SwapOps` + `WatcherOps` + `MarketCoinOps`)
- TPU support: `MakerCoinSwapOpsV2`/`TakerCoinSwapOpsV2`
- HD wallet (mandatory): `CoinWithDerivationMethod`, `HDWalletCoinOps`
- HTLC implementation (native or smart contract based)
- Task-based activation with RpcTaskManager (mandatory)
- Activation with `PrivKeyActivationPolicy` (ContextPrivKey, Trezor, WalletOnly)
- Transaction history and event streaming

**Key Decision Points**:
- Native HTLC (UTXO-style) vs Smart Contract HTLC (Ethereum-style)
- Single asset vs Platform coin with tokens
- External wallet support (MetaMask, WalletConnect, hardware wallets)

**Note**: Iguana (single address) mode is deprecated - HD wallet support is required for all new coins.

**Implementation Approach**:
- Always examine existing coin implementations (UTXO, ETH, Tendermint) for patterns
- Use `cargo clippy` early and often to catch trait mismatches
- Check trait definitions directly rather than assuming method signatures
- Use grep/search tools to find how types are used in existing code
- Don't assume async/sync patterns - verify in the trait definition
- Look for type aliases and re-exports (e.g., `Secp256k1ExtendedPublicKey` in crypto crate)

## Coin Integration Standards

### Task-Based Activation (Required for New Coins)
All new coin integrations MUST use task-based activation pattern:

```rust
// Example structure for new coins
// Location: mm2src/coins_activation/src/
pub async fn init_new_coin_activation(
    ctx: MmArc,
    req: NewCoinActivationRequest,
) -> MmResult<NewCoinActivationResult, NewCoinActivationError> {
    // Use task manager for long-running operations
    let task = RpcTask::new(/* ... */);
    // Implementation following task-based pattern
}
```

- Use RpcTaskManager for progress tracking
- Support cancellation via task handles
- Emit status updates through event system
- Never use legacy enable pattern

### Experimental Features
New features follow this lifecycle:

1. **Experimental**: Initial implementation in `experimental/` namespace
2. **Stable**: Move out of experimental namespace after thorough testing and team approval

#### Current Experimental Features
[List any current experimental features here]

#### Adding Experimental Features
```rust
// Always start in experimental namespace
// File: mm2src/mm2_main/src/rpc/lp_commands/experimental/your_feature.rs
pub async fn your_new_rpc(ctx: MmArc, req: Request) -> MmResult<Response, Error> {
    // Implementation
}

// Register in experimental dispatcher section
// Once stable, move to mm2src/mm2_main/src/rpc/lp_commands/your_feature.rs
```

### Code Style and Conventions

- Use `MmError` with `?` operator for error handling (transitioning away from `try_s!`, `ERR!`, `ERRL!` macros - see https://github.com/KomodoPlatform/komodo-defi-framework/issues/1250)
- Use `cfg_native!` and `cfg_wasm32!` for platform-specific code
- Always use absolute imports starting from crate root
- Document public APIs with examples
- Use minimal required derives - only `Serialize` for response types, only `Deserialize` for request types, both only when needed
- Follow the principle of minimal required changes - make only necessary changes while refactoring when possible

## Common Patterns and Antipatterns

### DO: Correct Patterns
```rust
// Error handling with MmError - crate-level error type
#[derive(Debug)]
pub enum CoinError {
    InvalidAddress(String),
    RpcError(String),
    // Module-specific variants
}

// Using mm_err for conversions
let result = some_operation().mm_err(|e| CoinError::RpcError(e.to_string()))?;

// Using map_to_mm
let value = some_result.map_to_mm(|_| CoinError::InvalidAddress("Invalid format".into()))?;

// Async operations
let response = coin.get_balance().await?;

// Module imports
use mm2_main::lp_swap::{MakerSwap, SwapError};
```

### DON'T: Antipatterns
```rust
// Wrong error handling
let result = try_s!(some_operation());  // Don't use deprecated macros

// Blocking in async
let response = block_on(coin.get_balance());  // Never block async runtime

// Relative imports
use super::super::swap::*;  // Use absolute imports
```

### Documentation Maintenance

**IMPORTANT**: When making changes or refactoring:

1. **Always update CLAUDE.md** if architectural patterns change
2. **Optimize CLAUDE.md** - don't just add content, remove what's no longer relevant, consolidate duplicates, and streamline structure
3. **Make changes step by step** - avoid large, sweeping changes unless stated otherwise. Break down modifications into small, incremental steps that can be easily reviewed and reverted if needed
4. **Update relevant documentation** in `docs/` directory
5. **Update code comments** and inline documentation
6. **Update README.md** if public APIs or build instructions change
7. **Check for affected documentation** in other files that might reference changed code

This ensures documentation stays accurate, concise, and helpful for both AI assistants and human developers.

### Development Tips

- The codebase uses workspace dependencies - check root `Cargo.toml`
- Default toolchain: `nightly-2023-06-01`
- For macOS: May need to configure loopback interfaces for tests
- Docker/Podman required for integration tests
- Use `--test-threads=16` for faster test execution
- Set `MM2_UNBUFFERED_OUTPUT=1` for real-time log output
- Use `cargo check` frequently - it's much faster than full builds
- Always run `cargo clippy --all-targets --all-features -- -D warnings` and `cargo clippy --target wasm32-unknown-unknown -- -D warnings` before considering work complete and/or before each commit
- Run `cargo fmt` to ensure consistent formatting before each commit
- Run `cargo test --all --all-features` before the last commit of a broken down task


## Additional Features

### Trading Features
- **1inch Integration**: DEX aggregation via `trading_api` module for same-chain swaps
- **Simple Market Maker**: Automated market making with configurable spreads (`lp_ordermatch/simple_market_maker.rs`)
- **LP Bot**: Advanced trading bot functionality (`lp_ordermatch/lp_bot.rs`)
- **Best Orders**: Optimized order matching across the network

### Wallet Modes and External Wallets
- **Full Trading**: Complete DEX capabilities with private key control
- **WalletOnly Mode**: No trading, just balance/withdraw operations
- **MetaMask Integration**: Via `mm2_metamask` module for Ethereum/ERC20
- **Hardware Wallets**: Trezor and Ledger support via `PrivKeyActivationPolicy`
- **WalletConnect**: For mobile wallet integration (when implemented)

### Storage Systems
- **GUI Storage**: Persistent storage for GUI applications (`mm2_gui_storage`)
- **Native Storage**: SQLite for desktop/mobile applications
- **WASM Storage**: IndexedDB for browser environments
- **Transaction History**: Configurable storage per coin type

### Advanced Features
- **NFT Support**: Via `MakerNftSwapOpsV2` trait for NFT atomic swaps
- **Lightning Network**: Layer 2 support for Bitcoin
- **IBC (Inter-Blockchain Communication)**: For Cosmos ecosystem
- **Fee Estimation**: Dynamic fee calculation for different protocols
- **Multi-address Support**: HD wallets with account/address derivation

## Appendix: DeepWiki MCP Setup

If DeepWiki MCP is not already configured for your Claude Code environment, follow these steps:

### Setup DeepWiki MCP for Komodo DeFi Framework

DeepWiki MCP provides AI-powered code understanding for the KDF repository. The recommended setup includes all tools for comprehensive code analysis.

#### Recommended Setup (With All Tools)

```bash
# Remove any existing configuration
claude mcp remove deepwiki

# Add DeepWiki with all tools enabled and repository focus
claude mcp add-json deepwiki '{
  "type": "sse",
  "url": "https://mcp.deepwiki.com/sse",
  "allowed_tools": ["ask_question", "read_wiki_structure", "read_wiki_contents"],
  "env": {
    "REPO_FOCUS": "KomodoPlatform/komodo-defi-framework"
  }
}'

# Verify configuration
claude mcp get deepwiki
```

This configuration enables:
- `ask_question`: Query the codebase with natural language questions
- `read_wiki_structure`: Explore repository structure and documentation
- `read_wiki_contents`: Access full content of documentation and code explanations

#### Alternative Configurations

**Basic Setup (if JSON config fails):**
```bash
claude mcp add --transport sse deepwiki https://mcp.deepwiki.com/sse
```

**Global Installation (for all projects):**
```bash
claude mcp add --transport sse deepwiki https://mcp.deepwiki.com/sse -s user
```

### Test the Connection

Once connected, test with these queries:
```
> What's the structure of KomodoPlatform/komodo-defi-framework?
> How does the order matching work in KomodoPlatform/komodo-defi-framework?
> Explain the swap protocol in KomodoPlatform/komodo-defi-framework
```

### Troubleshooting

If DeepWiki is not working:
1. Check MCP status: `/mcp` command in Claude Code
2. Restart Claude Code session
3. Re-add the server with the setup command above
4. Ensure you have internet connectivity to `https://mcp.deepwiki.com`

Note: The DeepWiki index may not include very recent commits. Always verify with manual search for the latest changes.
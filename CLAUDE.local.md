# CLAUDE.local.md - Personal Development Environment

## My Environment
- OS: macOS
- Architecture: aarch64 (Apple Silicon)
- Rust version: nightly-2023-06-01
- LLVM path: /opt/homebrew/opt/llvm

## Personal Shortcuts
```bash
# My common build command
alias kdf-build='cargo build'

# My test command
alias kdf-test='cargo test --all --features run-docker-tests -- --test-threads=16'

# WASM build for Apple Silicon
alias kdf-wasm='CC=/opt/homebrew/opt/llvm/bin/clang AR=/opt/homebrew/opt/llvm/bin/llvm-ar wasm-pack build mm2src/mm2_bin_lib --target web --out-dir wasm_build/deps/pkg/'

# Quick check
alias kdf-check='cargo check'
```

## Current Focus
- Working on: [Update this section when focus changes]
- Branch: [Update with current branch]
- Related issues: [Update with relevant issue numbers]

## Local Dev Notes
- Using RustRover as IDE
- [Add any personal notes about your setup or current work]
1. Install latest nightly toolchain and make it default:
```
rustup install nightly
rustup default nightly
```
2. Install cross: `cargo install cross`.
3. Build the Docker image for cross compilation: `docker build -f .docker/Dockerfile.armv7-unknown-linux-gnueabihf -t mm2-armv7-unknown-linux-gnueabihf .`
4. Build mm2: `cross build --target armv7-unknown-linux-gnueabihf` or `cross build --target armv7-unknown-linux-gnueabihf --release` for release build.
5. The binary path will be `target/armv7-unknown-linux-gnueabihf/debug/mm2` or `target/armv7-unknown-linux-gnueabihf/release/mm2` for release build.   
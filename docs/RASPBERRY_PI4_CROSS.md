1. Install cross: `cargo install cross`.
2. Build the Docker image for cross compilation: `docker build -f .docker/Dockerfile.armv7-unknown-linux-gnueabihf -t mm2-armv7-unknown-linux-gnueabihf .`
3. Build mm2: `cross build --target armv7-unknown-linux-gnueabihf` or `cross build --target armv7-unknown-linux-gnueabihf --release` for release build.
4. The binary path will be `target/armv7-unknown-linux-gnueabihf/debug/mm2` or `target/armv7-unknown-linux-gnueabihf/release/mm2` for release build.   
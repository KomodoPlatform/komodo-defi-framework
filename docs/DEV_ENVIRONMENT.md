# Setting up the dev environment for AtomicDEX-API to run full tests suite

## Running native tests

1. Install Docker or Podman.
2. Download ZCash params files: [Windows](https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.bat),
   [Unix/Linux](https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.sh)
3. Create `.env.client` file with the following content
   ```
   PASSPHRASE=spice describe gravity federal blast come thank unfair canal monkey style afraid
   ```
4. Create `.env.seed` file with the following content
   ```
   PASSPHRASE=also shoot benefit prefer juice shell elder veteran woman mimic image kidney
   ```
5. MacOS specific: run script (required after each reboot)
   ```shell
   #!/bin/bash
   for ((i=2;i<256;i++))
   do
       sudo ifconfig lo0 alias 127.0.0.$i up
   done
   sudo ifconfig lo0 inet6 -alias ::1
   sudo ifconfig lo0 inet6 -alias fe80::1%lo0
   ```
   Please note that you have to run it again after each reboot
6. Linux specific:
    - for Docker users:
       ```
       sudo groupadd docker
       sudo usermod -aG docker $USER
       ```
    - for Podman users:
       ```
       sudo ln -s $(which podman) /usr/bin/docker
       ```
7. Try `cargo test --features native --all -- --test-threads=16`.

## Running WASM tests

1. Set up [WASM Build Environment](../docs/WASM_BUILD.md#Setting-up-the-environment)
2. Install Firefox.
3. Download [Gecko driver](https://github.com/mozilla/geckodriver/releases) for your OS
4. Set environment variables required to run WASM tests
   ```shell
   # wasm-bindgen specific variables
   export WASM_BINDGEN_TEST_TIMEOUT=120
   export GECKODRIVER=PATH_TO_GECKO_DRIVER_BIN
   # MarketMaker specific variables
   export BOB_PASSPHRASE="also shoot benefit prefer juice shell elder veteran woman mimic image kidney"
   export ALICE_PASSPHRASE="spice describe gravity federal blast come thank unfair canal monkey style afraid"
   ```
6. Run WASM tests
   - for Linux users:
   ```
   wasm-pack test --firefox --headless
   ```
    - for OSX users (Intel):
   ```
   CC=/usr/local/opt/llvm/bin/clang AR=/usr/local/opt/llvm/bin/llvm-ar wasm-pack test --firefox --headless
   ```
    - for OSX users (M1):
   ```
   CC=/opt/homebrew/opt/llvm/bin/clang AR=/opt/homebrew/opt/llvm/bin/llvm-ar wasm-pack test --firefox --headless
   ```
   Please note `CC` and `AR` must be specified in the same line as `wasm-pack test`.

PS If you notice that this guide is outdated, please submit a PR.

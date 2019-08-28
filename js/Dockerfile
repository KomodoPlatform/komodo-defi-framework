# Demo of driving the peers communication from the portable layer of the code.
#
#     docker build --file js/Dockerfile --tag mm2-peers-demo .
#     docker run -ti mm2-peers-demo

# -- Begins with parts copied from the MM Dockerfile in order to share those filesystem layers --

FROM ubuntu:xenial

RUN \
    apt-get update &&\
    apt-get install -y git build-essential libssl-dev wget &&\
    # https://github.com/rust-lang/rust-bindgen/blob/master/book/src/requirements.md#debian-based-linuxes
    apt-get install -y llvm-3.9-dev libclang-3.9-dev clang-3.9 &&\
    # openssl-sys requirements, cf. https://crates.io/crates/openssl-sys
    apt-get install -y pkg-config libssl-dev &&\
    apt-get clean

RUN wget https://cmake.org/files/v3.12/cmake-3.12.0-Linux-x86_64.sh && \
    sh cmake-3.12.0-Linux-x86_64.sh --skip-license --exclude-subdir --prefix=/usr/local && \
    rm -rf cmake-3.12.0-Linux-x86_64.sh

RUN \
    wget -O- https://sh.rustup.rs > /tmp/rustup-init.sh &&\
    sh /tmp/rustup-init.sh -y --default-toolchain none &&\
    . /root/.cargo/env &&\
    rustup install nightly-2019-08-05 &&\
    rustup default nightly-2019-08-05 &&\
    # It seems that bindgen won't prettify without it:
    rustup component add rustfmt-preview &&\
    rm -rf /root/.rustup/toolchains/nightly-2019-08-05-x86_64-unknown-linux-gnu/share/doc &&\
    rm -f /tmp/rustup-init.sh

ENV PATH="/root/.cargo/bin:${PATH}"

# First 7 characters of the commit ID.
ENV MM_VERSION="f44ffec"

RUN cd /tmp &&\
    wget https://api.github.com/repos/KomodoPlatform/atomicDEX-API/tarball/$MM_VERSION &&\
    tar -xzf $MM_VERSION &&\
    ls &&\
    mv KomodoPlatform-atomicDEX-API-$MM_VERSION /mm2 &&\
    rm $MM_VERSION &&\
    echo $MM_VERSION > /mm2/MM_VERSION

RUN cd /mm2 && cargo fetch

# -- End of the shared parts --

RUN cd /tmp &&\
    wget https://nodejs.org/dist/v12.6.0/node-v12.6.0-linux-x64.tar.xz &&\
    tar -xJf node-v12.6.0-linux-x64.tar.xz &&\
    cd /tmp/node-v12.6.0-linux-x64 &&\
    cp -r * /usr/local/ &&\
    cd /tmp &&\
    rm -rf node* &&\
    node --version

RUN rustup target add wasm32-unknown-unknown

RUN cargo install rustfilt

# This will overwrite the Git version with the local one.
# Only needed when we're developing or changing something locally.
#COPY . /mm2

RUN cd /mm2 &&\
    cargo build --target=wasm32-unknown-unknown --release &&\
    mv target/wasm32-unknown-unknown/release/mm2.wasm js/ &&\
    cargo build --features native --bin mm2 &&\
    cp target/debug/mm2 js/ &&\
    rm -rf target

RUN cd /mm2/js &&\
    npm install > npm-install.log 2>&1

CMD cd /mm2/js && node wasm-run.js 2>&1 | rustfilt

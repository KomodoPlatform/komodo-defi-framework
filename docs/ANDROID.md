## Cross-compiling MM for Android

### Requirements

We need a Unix operating system (the build has been tested on Linux and Mac).

We need a free access to the Docker (`docker run hello-world` should work).

We need the Nightly revision of Rust, such as

    rustup install nightly-2019-03-10
    rustup default nightly-2019-03-10

### Install cross

    git clone --depth=1 git@github.com:ArtemGr/cross.git
    (cd cross && cargo install -f --path .) && rm -rf cross

### Install extra packages into the Docker image

The [Docker image](https://github.com/rust-embedded/cross/tree/master/docker/armv7-linux-androideabi) used by `cross` for the cross-compilation is [missing](https://github.com/rust-embedded/cross/issues/174) certain things that we need. So we're going to build a patched up image.

    git clone git@github.com:ArtemGr/cross.git aga-cross || git clone https://github.com/ArtemGr/cross.git aga-cross
    (cd aga-cross/docker && docker build --tag armv7-linux-androideabi-aga -f armv7-linux-androideabi/Dockerfile .)

### Get the source code

    git clone --depth=1 git@gitlab.com:artemciy/supernet.git -b mm2-cross
    cd supernet
    git log --pretty=format:'%h' -n 1 > MM_VERSION

### Setup the NDK_HOME variable

The Docker image used by `cross` contains the NDK under /android-ndk,
but we need to point some of the dependencies to that location
by setting the NDK_HOME variable.

    export NDK_HOME=/android-ndk

### Build

    cross build --features native --target=armv7-linux-androideabi -vv

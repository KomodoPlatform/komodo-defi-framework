## Cross-compiling MM for Android

### Requirements

We need a Unix operating system (the build has been tested on Linux and Mac).

We need a free access to the Docker (`docker run hello-world` should work).

We need the Nightly revision of Rust, such as

    rustup default nightly-2021-05-17

### Install cross

    cargo install cross

### Get the source code

    git clone --depth=1 git@gitlab.com:KomodoPlatform/supernet.git -b mm2.1-cross
    cd supernet
    git log --pretty=format:'%h' -n 1 > MM_VERSION
    git log --pretty=format:'%cI' -n 1 > MM_DATETIME

### Install extra packages into the Docker image

The [Android NDK installer](https://github.com/rust-embedded/cross/tree/master/docker/android-ndk.sh) used by the ['cross' docker image](https://github.com/rust-embedded/cross/tree/master/docker/armv7-linux-androideabi) for the cross-compilation uses out-of-date NDK version. So we're going to build a patched up image.

#### armeabi-v7a ABI Docker image

    (cd supernet && docker build --tag armv7-linux-androideabi-aga -f .docker/Dockerfile.armv7-linux-androideabi .)

#### arm64-v8a ABI Docker image

    (cd supernet && docker build --tag aarch64-linux-android-aga -f .docker/Dockerfile.aarch64-linux-android .)

### x86 ABI Docker image

    (cd supernet && docker build --tag i686-linux-android-aga -f .docker/Dockerfile.i686-linux-android .)

### x86_64 ABI Docker image

    (cd supernet && docker build --tag x86_64-linux-android-aga -f .docker/Dockerfile.x86_64-linux-android .)

### Setup the NDK_HOME variable

The Docker image used by `cross` contains the NDK under /android-ndk,
but we need to point some of the dependencies to that location
by setting the NDK_HOME variable.

    export NDK_HOME=/android-ndk

### Build

#### armeabi-v7a

    cross build --target=armv7-linux-androideabi --release --lib

#### arm64-v8a

    cross build --target=aarch64-linux-android --release --lib

#### x86

    cross build --target=i686-linux-android --release --lib

#### x86_64

    cross build --target=x86_64-linux-android --release --lib

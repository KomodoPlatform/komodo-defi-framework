## Cross-compiling MM for Android

### Requirements

We need a Unix operating system (the build has been tested on Linux and Mac).

We need a free access to the Docker (`docker run hello-world` should work).

We need the Nightly revision of Rust, such as

    rustup install nightly-2019-03-10
    rustup default nightly-2019-03-10

### Install cross

    git clone --depth=1 https://github.com/ArtemGr/cross
    (cd cross && cargo install -f --path .) && rm -rf cross

### Install extra packages into the Docker image

The [Docker image](https://github.com/rust-embedded/cross/tree/master/docker/armv7-linux-androideabi) used by `cross` for the cross-compilation is [missing](https://github.com/rust-embedded/cross/issues/174) certain things that we need. We have to add these to the Docker image for the build to work.

To do this we'll need two terminals. In the first terminal we start a container from that image and install the necessary packages there:

    docker run --name cross-upgrade -ti japaric/armv7-linux-androideabi bash
    apt-get update && \
        apt-get install -y llvm-3.9-dev libclang-3.9-dev clang-3.9 && \
        apt-get install -y libc6-dev-i386 && \
        apt-get clean && \
        echo 'All done.'

Having done that, and still keeping the first container open,
we use a second terminal to commit the changes back into the image

    docker commit cross-upgrade japaric/armv7-linux-androideabi
    docker stop cross-upgrade
    docker rm cross-upgrade

### Get the source code

    git clone --depth=1 https://github.com/artemii235/SuperNET.git -b mm2-cross
    cd SuperNET
    git log --pretty=format:'%h' -n 1 > MM_VERSION

### Setup the NDK_HOME variable

The Docker image used by `cross` contains the NDK under /android-ndk,
but we need to point some of the dependencies to that location
by setting the NDK_HOME variable and letting it into the `cross` build.

    export NDK_HOME=/android-ndk
    printf '[build.env]\npassthrough = [\n "NDK_HOME",\n "RUST_BACKTRACE"\n]\n' > Cross.toml

### Build

    cross build -vv --target=armv7-linux-androideabi

#!/bin/bash

set -ex

NDK_URL=https://dl.google.com/android/repository/android-ndk-r23c-linux.zip
NDK_CHECKSUM=6ce94604b77d28113ecd588d425363624a5228d9662450c48d2e4053f8039242

main() {
    local arch=$1 \
        api=$2

    local dependencies=(
        unzip
        python3
        python3-distutils
        curl
    )

    sudo apt-get update
    local purge_list=()
    for dep in ${dependencies[@]}; do
        if ! dpkg -L $dep; then
            sudo apt-get install --no-install-recommends -y $dep
            purge_list+=($dep)
        fi
    done

    td=$(mktemp -d)

    pushd $td
    curl -O $NDK_URL
    if ! echo "$NDK_CHECKSUM  android-ndk-r23c-linux.zip" | sha256sum -c -; then
        echo "Error: SHA256 sum mismatch for android-ndk-r23c-linux.zip"
        exit 1
    fi
    unzip -q android-ndk-r23c-linux.zip
    pushd android-ndk-*/
    sudo ./build/tools/make_standalone_toolchain.py \
        --install-dir /android-ndk \
        --arch $arch \
        --api $api

    # clean up
    sudo apt-get purge --auto-remove -y ${purge_list[@]}

    popd
    popd

    rm -rf $td
    rm $0
}

main "${@}"

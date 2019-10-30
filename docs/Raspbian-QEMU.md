
It should be possible to build Market Maker under [Raspbian](https://en.wikipedia.org/wiki/Raspbian) in order to run it on Raspberry Pi. I don't own the Rasperry Pi and was building on Windows under QEMU instead. This document describes a way to do that (Windows/QEMU/Raspbian/MM2).

[MSYS](https://msys2.github.io/) can be used to install the QEMU:

    pacman -S mingw-w64-x86_64-qemu
    pacman -Ql mingw-w64-x86_64-qemu

[Wim Vanderbauwhede's tutorial](https://github.com/wimvanderbauwhede/limited-systems/wiki/Raspbian-%22stretch%22-for-Raspberry-Pi-3-on-QEMU) points at the files we need. It is the kernel files in his [raspbian_bootpart](https://github.com/wimvanderbauwhede/limited-systems/tree/master/Raspberry-Pi-3-QEMU/Raspbian/raspbian_bootpart) directory plus the Raspbian image obtained from https://downloads.raspberrypi.org/raspbian_lite_latest.

We'll also need a ramdisk in order to place the swap file there. (Raspbian under QEMU-arm can only use 256 MiB of RAM so a lot of swap will be used during the build and placing the swap on a physical drive will make the build much slower).  
I've used [ImDisk](https://sourceforge.net/projects/imdisk-toolkit/files/20190924/ImDiskTk-x64.exe/download)'s `RamDisk Configuration` to mount 1111 MiB of exFAT on R.  

The swap file is then created [with](https://www.raspberrypi.org/forums/viewtopic.php?p=109287#p109287)

    qemu-img create -f raw /r/swap 1G

Now we can start the Raspbian with

    qemu-system-arm \
      -kernel kernel-qemu-4.14.50-stretch \
      -dtb versatile-pb.dtb \
      -m 256 -M versatilepb -cpu arm1176 \
      -serial stdio \
      -append "rw console=ttyAMA0 root=/dev/sda2 rootfstype=ext4 loglevel=8 rootwait fsck.repair=yes memtest=1" \
      -drive file=2019-07-10-raspbian-buster-lite.img,format=raw \
      -drive file=/r/swap,format=raw \
      -no-reboot

Raspbian's username is "pi" and password is "raspberry".

QEMU will give us two terminals: the graphical QEMU window and the console terminal under which the QEMU was first started. In my experience only the graphical QEMU window works reliably, whereas the console terminal would lose some keystrokes (it is a known bug, though I don't have a link to it at hand).  
Note also that pressing ^C in the console terminal will terminate the QEMU ([one way to prevent this](https://unix.stackexchange.com/a/171707/42463) might be to remap ^C with `stty intr ^]`).

The Raspbian image thus obtained is too small for us. We ought to resize it by shutting down Raspbian and running

    qemu-img resize -f raw 2019-07-10-raspbian-buster-lite.img +10G

This resize should then be mirrored under Raspbian [with](https://gist.github.com/larsks/3933980)

    sudo fdisk /dev/sda
      d, 2, n, p, 2, 540672, enter, N, w
    sync; sudo reboot -f

And then

    sudo resize2fs /dev/sda2
    sync; sudo reboot -f

Now that we have a larger image, we can prepare the packages and the source code necessary for the build

    sudo apt-get update
    sudo apt install git cmake mc
    sudo apt install llvm-dev libclang-dev clang
    sudo apt install libssl-dev

    git clone --depth=1 https://github.com/KomodoPlatform/atomicDEX-API.git
    cd atomicDEX-API/
    wget -Orustup http://sh.rustup.rs
    sh rustup -y --default-toolchain none
    . /home/pi/.cargo/env

    rustup set profile minimal
    # cf. https://rust-lang.github.io/rustup-components-history/arm-unknown-linux-gnueabihf.html
    rustup install nightly-2019-10-24
    rustup default nightly-2019-10-24
    rustup component add rustfmt-preview

And we should uncomment the `codegen-units = 1` in `Cargo.toml`

    mcedit Cargo.toml
      # https://github.com/rust-lang/rust/issues/62896
      codegen-units = 1

Now let's add the swap memory

    sudo mkswap /dev/sdb
    sudo swapon /dev/sdb

And start the build with

    cargo build --features native

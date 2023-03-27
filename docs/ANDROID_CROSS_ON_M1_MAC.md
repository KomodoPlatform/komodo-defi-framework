## Cross-compiling MM for Android on M1 Mac

1. Ensure that your terminal is added to `Developer tools` in MacOS Security & Privacy settings.
2. The cross-compilation requires Android NDK version 21. Custom brew cask file is located at the root of this repo.
3. Install android-ndk by `brew install --cask ./scripts/ci/android-ndk.rb` or other preferred way.
4. Add Android targets via rustup
```shell
rustup target add armv7-linux-androideabi
rustup target add aarch64-linux-android
rustup target add i686-linux-android
rustup target add x86_64-linux-android
```
5. The following build commands presume that NDK was installed using brew. If it was installed other way, you have to change PATH ENV accordingly. 

   Setting PATH
   ```shell
   export PATH=$PATH:/opt/homebrew/share/android-ndk/toolchains/llvm/prebuilt/darwin-x86_64/bin
   ```
   Build armv7-linux-androideabi target
   ```shell
   CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER=armv7a-linux-androideabi21-clang CC_armv7_linux_androideabi=armv7a-linux-androideabi21-clang AR_armv7_linux_androideabi=llvm-ar CXX_armv7_linux_androideabi=armv7a-linux-androideabi21-clang++ ANDROID_NDK_HOME="/opt/homebrew/share/android-ndk" cargo rustc --target=armv7-linux-androideabi --release --lib --crate-type=staticlib --package mm2_bin_lib
   ```
   Build aarch64-linux-android target
   ```shell
   CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android21-clang CC_aarch64_linux_android=aarch64-linux-android21-clang AR_aarch64_linux_android=llvm-ar CXX_aarch64_linux_android=aarch64-linux-android21-clang++ ANDROID_NDK_HOME="/opt/homebrew/share/android-ndk" cargo rustc --target=aarch64-linux-android --release --lib --crate-type=staticlib --package mm2_bin_lib
   ```
   Build i686-linux-android target
   ```shell
   CARGO_TARGET_I686_LINUX_ANDROID_LINKER=i686-linux-android21-clang CC_i686_linux_android=i686-linux-android21-clang AR_i686_linux_android=llvm-ar CXX_i686_linux_android=i686-linux-android21-clang++ ANDROID_NDK_HOME="/opt/homebrew/share/android-ndk" cargo rustc --target=i686-linux-android --release --lib --crate-type=staticlib --package mm2_bin_lib
   ```
   Build x86_64-linux-android target
   ```shell
   CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER=x86_64-linux-android21-clang CC_x86_64_linux_android=x86_64-linux-android21-clang CXX_x86_64_linux_android=x86_64-linux-android21-clang++ AR_x86_64_linux_android=llvm-ar ANDROID_NDK_HOME="/opt/homebrew/share/android-ndk" cargo rustc --target=x86_64-linux-android --release --lib --crate-type=staticlib --package mm2_bin_lib
   ```

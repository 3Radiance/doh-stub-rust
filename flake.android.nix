{
  description = "Rust Android Cross-Compilation Flake with BoringSSL & NDK";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
          config.allowUnfree = true;
        };

        androidComposition = pkgs.androidenv.composeAndroidPackages {
          includeNDK = true;
          ndkVersion = "26.1.10909125";
        };
        ndk = androidComposition.ndk-bundle;

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "aarch64-linux-android" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustToolchain
            pkg-config
            cmake
            ninja
            go

            llvmPackages.clang
            llvmPackages.bintools
          ];

          shellHook = ''
            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
            export ALL_PROXY="socks5://127.0.0.1:10808"

          
            export ANDROID_NDK_HOME="${ndk}/libexec/android-sdk/ndk-bundle"
            if [ ! -d "$ANDROID_NDK_HOME" ]; then
              export ANDROID_NDK_HOME="${ndk}/libexec/android-sdk/ndk/26.1.10909125"
            fi
            if [ ! -d "$ANDROID_NDK_HOME" ]; then
              export ANDROID_NDK_HOME="${ndk}"
            fi

            NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
            NDK_SYSROOT="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"

    
            TOOLCHAIN_WRAPPER="/tmp/android_aarch64_toolchain.cmake"
            cat <<EOF > "$TOOLCHAIN_WRAPPER"
set(ANDROID_ABI "arm64-v8a" CACHE STRING "" FORCE)
set(ANDROID_PLATFORM "android-29" CACHE STRING "" FORCE)
set(ANDROID_STL "c++_static" CACHE STRING "" FORCE)
include("$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake")
EOF

            export CMAKE_TOOLCHAIN_FILE="$TOOLCHAIN_WRAPPER"
            export CMAKE_TOOLCHAIN_FILE_aarch64_linux_android="$TOOLCHAIN_WRAPPER"

           
            unset CPATH
            unset C_INCLUDE_PATH
            unset CPLUS_INCLUDE_PATH
            unset NIX_CFLAGS_COMPILE
            unset NIX_LDFLAGS

            
            BINDGEN_FLAGS="--target=aarch64-linux-android --sysroot=$NDK_SYSROOT -isystem $NDK_SYSROOT/usr/include -isystem $NDK_SYSROOT/usr/include/aarch64-linux-android"
            export BINDGEN_EXTRA_CLANG_ARGS="$BINDGEN_FLAGS"
            export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="$BINDGEN_FLAGS"

            
            export CC_aarch64_linux_android="$NDK_BIN/aarch64-linux-android29-clang"
            export CXX_aarch64_linux_android="$NDK_BIN/aarch64-linux-android29-clang++"
            export AR_aarch64_linux_android="$NDK_BIN/llvm-ar"

            export CFLAGS_aarch64_linux_android="--target=aarch64-linux-android29"
            export CXXFLAGS_aarch64_linux_android="--target=aarch64-linux-android29"

            export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$NDK_BIN/aarch64-linux-android29-clang"
            export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-fuse-ld=lld -C link-arg=-static-libstdc++"
          '';
        };
      }
    );
}
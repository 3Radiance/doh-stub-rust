{
  description = "Windows MSVC Cross-compilation environment using LLVM and xwin";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        proxyUrl = "http://127.0.0.1:10808";

      in {
        devShells.default = (pkgs.buildFHSEnv {
          name = "rust-doh-msvc-env";
          targetPkgs = pkgs: with pkgs; [
            rustup
            cargo-xwin
            xwin

            clang
            lld
            llvm               
            llvmPackages.libclang
            gcc

            cmake
            ninja
            gnumake
            pkg-config
            nasm
            yasm
          ];
          profile = ''
            export PATH="$HOME/.cargo/bin:$PATH"

            mkdir -p /tmp/llvm-bin
            ln -sf $(which llvm-ar) /tmp/llvm-bin/llvm-lib
            export PATH="/tmp/llvm-bin:$PATH"

            export HTTP_PROXY="${proxyUrl}"
            export HTTPS_PROXY="${proxyUrl}"
            export http_proxy="${proxyUrl}"
            export https_proxy="${proxyUrl}"
            export ALL_PROXY="${proxyUrl}"
            export all_proxy="${proxyUrl}"

            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
            export XWIN_DIR="$HOME/.xwin"

            export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="gcc"
            export CC="gcc"
            export CXX="g++"

            export CC_x86_64_pc_windows_msvc="clang-cl"
            export CXX_x86_64_pc_windows_msvc="clang-cl"
            export AR_x86_64_pc_windows_msvc="llvm-lib"
            export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER="lld-link"

            export CMAKE_SYSTEM_NAME="Windows"
            export CMAKE_SYSTEM_PROCESSOR="AMD64"
          '';
        }).env;
      }
    );
}

{
  description = "FHS Rust dev + static musl release builds";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        proxyUrl = "http://127.0.0.1:10808";
        fhs = pkgs.buildFHSEnv {
          name = "rust-doh-env";
          targetPkgs = pkgs: with pkgs; [
            rustup gcc clang llvmPackages.libclang
            cmake gnumake pkg-config
            glibc glibc.dev openssl libunwind zlib
            pkgsCross.musl64.stdenv.cc
          ];
          profile = ''
            export HTTP_PROXY="${proxyUrl}"
            export HTTPS_PROXY="${proxyUrl}"
            export http_proxy="${proxyUrl}"
            export https_proxy="${proxyUrl}"
            export ALL_PROXY="${proxyUrl}"
            export all_proxy="${proxyUrl}"
            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
            export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="gcc"
            export CC_x86_64_unknown_linux_musl="x86_64-unknown-linux-musl-gcc"
            export CXX_x86_64_unknown_linux_musl="x86_64-unknown-linux-musl-g++"
            export AR_x86_64_unknown_linux_musl="x86_64-unknown-linux-musl-ar"
            export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="x86_64-unknown-linux-musl-gcc"
          '';
          runScript = "zsh";
        };

        muslPkgs = pkgs.pkgsCross.musl64.pkgsStatic;

        doh-stub-static = muslPkgs.rustPlatform.buildRustPackage {
          pname = "doh-stub-rust";
          version = "1.3.0";
          src = pkgs.fetchFromGitHub {
            owner = "3Radiance";
            repo = "doh-stub-rust";
            rev = "master"; # pin to a tag/commit for reproducibility
            sha256 = pkgs.lib.fakeSha256; # nix will tell you the real hash on first build
          };
          cargoLock.lockFile = ./Cargo.lock; # or fetch alongside src
          # Fully static binary: no dynamic linker, no RPATH, runs on any
          # x86_64 Linux kernel new enough (basically anything from the
          # last decade), NixOS or not.
        };

      in
      {
        devShells.default = fhs.env;

        packages.doh-stub-static = doh-stub-static;
      }
    );
}

 {

  description = "FHS Rust development environment";


  inputs = {

    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    flake-utils.url = "github:numtide/flake-utils";

  };


  outputs = { self, nixpkgs, flake-utils, ... }:

    flake-utils.lib.eachDefaultSystem (system:

      let

        pkgs = import nixpkgs { inherit system; };

        

        fhs = pkgs.buildFHSEnv {

          name = "rust-doh-env";


          targetPkgs = pkgs: with pkgs; [

            # Rust & Cargo

            rustup

            

            # C/C++ & LLVM / Clang

            gcc

            clang

            llvmPackages.libclang

            

            # Build tools

            cmake

            gnumake

            pkg-config

            

            # System libs

            glibc

            glibc.dev

            openssl

            libunwind

            zlib

          ];


          profile = ''

            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"

            export ALL_PROXY="socks5://127.0.0.1:10808"

           '';


          runScript = "bash";

        };

      in

      {

        devShells.default = fhs.env;

      }

    );

} 
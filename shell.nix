let
    rust_overlay = import (builtins.fetchTarball "https://github.com/oxalica/rust-overlay/archive/master.tar.gz");
    pkgs = import <nixpkgs> { overlays = [ rust_overlay ]; };
    rust = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
    llvm = pkgs.llvmPackages_11;
    aarch64 = import <nixpkgs> {
      crossSystem = { config = "aarch64-none-elf"; };
    };
in
  pkgs.mkShell {
    buildInputs = with pkgs.buildPackages; [
        rust
        gnumake
        llvm.clang
        llvm.lld
        llvm.libllvm
        llvm.libclang
        aarch64.buildPackages.gcc10
    ];
    hardeningDisable = [ "all" ];
    # Need to specify this when using Rust with bindgen
    LIBCLANG_PATH = "${llvm.libclang.lib}/lib";
}


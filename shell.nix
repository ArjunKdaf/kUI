{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  # rustup manages the actual Rust toolchain — rust-toolchain.toml pins the
  # channel to stable, so `cargo` here is always the latest stable release.
  buildInputs = with pkgs; [
    rustup
    pkg-config
    # desktop hal backend (dev window on the host)
    SDL2
    # cross-linking to the device is done against the tg5040 sysroot (M0)
  ];

  shellHook = ''
    export RUSTUP_HOME="$PWD/.rustup"
    export CARGO_HOME="$PWD/.cargo"
    export PATH="$CARGO_HOME/bin:$PATH"
  '';
}

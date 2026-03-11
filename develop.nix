{ pkgs }:

let
  rustToolchain = pkgs.rust-bin.stable.latest.default.override {
    extensions = [ "rust-src" "rust-analyzer" ];
  };

  nixPackages = with pkgs; [
    nil
    nixd
    alejandra
    statix
    nix-tree
    nix-output-monitor
    vscode-json-languageserver
  ];

  rustPackages = with pkgs; [
    rustToolchain
    pkg-config
    openssl
    cargo-watch
    cargo-edit
    cargo-outdated
    gcc
    gnumake
    cmake
  ];

  devPackages = with pkgs; [
    git
    jq
    shellcheck
    shfmt
    bash-language-server
    package-version-server
    go gopls
  ];

  shell = pkgs.mkShell {
    packages = nixPackages ++ rustPackages ++ devPackages;

    buildInputs = with pkgs; [
      openssl
    ];

    RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
  };

in
{
  inherit shell;
}

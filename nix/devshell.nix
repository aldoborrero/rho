{
  pkgs,
  inputs,
  system,
  ...
}:
let
  # Rust nightly toolchain via fenix (required for edition 2024)
  fenixPkgs = inputs.fenix.packages.${system};
  rustToolchain = fenixPkgs.complete.withComponents [
    "cargo"
    "rustc"
    "rust-src"
    "rust-analyzer"
    "clippy"
    "rustfmt"
  ];
in
pkgs.mkShellNoCC {
  packages = [
    rustToolchain
    pkgs.pkg-config
  ]
  ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
    pkgs.libx11
  ];

  shellHook = ''
    export PRJ_ROOT=$PWD
    ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [pkgs.stdenv.cc.cc.lib]}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    ''}
  '';
}

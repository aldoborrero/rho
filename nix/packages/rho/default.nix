{
  pkgs,
  inputs,
  system,
  ...
}:
let
  inherit (pkgs) lib stdenv;

  # Rust nightly toolchain via fenix (required for edition 2024)
  toolchain = inputs.fenix.packages.${system}.complete.withComponents [
    "cargo"
    "rustc"
    "rust-src"
  ];

  craneLib = (inputs.crane.mkLib pkgs).overrideToolchain toolchain;

  # Source filtering — keep Rust files, Cargo manifests, vendored crates,
  # and .md files (embedded via include_str! for tool/system prompts)
  rawSrc = lib.cleanSource ../../..;
  src = lib.cleanSourceWith {
    src = rawSrc;
    filter = path: type:
      (builtins.match ".*\\.md$" path != null)
      || (craneLib.filterCargoSources path type);
  };

  # Common arguments for crane builds
  commonArgs = {
    inherit src;
    strictDeps = true;
    pname = "rho";
    version = "0.1.0";

    nativeBuildInputs = [
      pkgs.pkg-config
      pkgs.git # needed by tests that create temporary git repos
    ];

    # Runtime dependencies for arboard (clipboard support on Linux)
    buildInputs = lib.optionals stdenv.hostPlatform.isLinux [ pkgs.libx11 ];
  };

  # Build dependencies first (for caching)
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    # Only build the rho binary crate
    cargoExtraArgs = "-p rho";

    meta = {
      description = "A terminal-based AI coding agent written in Rust";
      homepage = "https://github.com/aldoborrero/rho";
      license = lib.licenses.mit;
      sourceProvenance = with lib.sourceTypes; [ fromSource ];
      maintainers = with lib.maintainers; [ aldoborrero ];
      platforms = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      mainProgram = "rho";
    };
  }
)

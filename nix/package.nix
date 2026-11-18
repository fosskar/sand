{
  lib,
  rustPlatform,
  protobuf,
  pkg-config,
  oniguruma,
  sqlite,
  zstd,
}:
# the check phase runs the tests that work offline; the ones that need the embedding model are #[ignore]
rustPlatform.buildRustPackage {
  pname = "sediment";
  version = (lib.importTOML ../Cargo.toml).package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../benches
      ../tests
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  # cargoSetupHook makes zstd-sys, onig_sys and libsqlite3-sys link the system libraries
  nativeBuildInputs = [
    protobuf
    pkg-config
  ];
  buildInputs = [
    oniguruma
    sqlite
    zstd
  ];

  env = {
    PROTOC = "${protobuf}/bin/protoc";
    PROTOC_INCLUDE = "${protobuf}/include";
  };

  meta = {
    description = "Semantic memory for AI agents - local-first, MCP-native";
    homepage = "https://github.com/fosskar/sand";
    license = lib.licenses.mit;
    mainProgram = "sediment";
  };
}

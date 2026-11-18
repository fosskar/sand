{
  mkShell,
  cargo,
  rustc,
  clippy,
  rustfmt,
  rust-analyzer,
  protobuf,
  pkg-config,
  oniguruma,
  sqlite,
  zstd,
  treefmt,
}:
mkShell {
  packages = [
    cargo
    rustc
    clippy
    rustfmt
    rust-analyzer
    protobuf
    pkg-config
    oniguruma
    sqlite
    zstd
    treefmt
  ];
  PROTOC = "${protobuf}/bin/protoc";
  PROTOC_INCLUDE = "${protobuf}/include";
}

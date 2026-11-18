{
  description = "Semantic memory for AI agents - local-first, MCP-native";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: {
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.sediment;
        # the check phase runs the tests that work offline; the ones that need the embedding model are #[ignore]
        sediment = pkgs.rustPlatform.buildRustPackage {
          pname = "sediment";
          version = (lib.importTOML ./Cargo.toml).package.version;

          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./src
              ./benches
              ./tests
            ];
          };

          cargoLock.lockFile = ./Cargo.lock;

          # cargoSetupHook makes zstd-sys, onig_sys and libsqlite3-sys link the system libraries
          nativeBuildInputs = [
            pkgs.protobuf
            pkgs.pkg-config
          ];
          buildInputs = [
            pkgs.oniguruma
            pkgs.sqlite
            pkgs.zstd
          ];

          env = {
            PROTOC = "${pkgs.protobuf}/bin/protoc";
            PROTOC_INCLUDE = "${pkgs.protobuf}/include";
          };

          meta = {
            description = "Semantic memory for AI agents - local-first, MCP-native";
            homepage = "https://github.com/fosskar/sand";
            license = lib.licenses.mit;
            mainProgram = "sediment";
          };
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.clippy
            pkgs.rustfmt
            pkgs.rust-analyzer
            pkgs.protobuf
            pkgs.pkg-config
            pkgs.oniguruma
            pkgs.sqlite
            pkgs.zstd
          ];
          PROTOC = "${pkgs.protobuf}/bin/protoc";
          PROTOC_INCLUDE = "${pkgs.protobuf}/include";
        };
      });

      checks = forAllSystems (pkgs: {
        package = self.packages.${pkgs.stdenv.hostPlatform.system}.sediment;
      });
    };
}

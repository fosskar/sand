{
  description = "Semantic memory for AI agents - local-first, MCP-native";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixbot = {
      url = "github:Mic92/nixbot";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.treefmt-nix.follows = "treefmt-nix";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      treefmt-nix,
      nixbot,
    }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      treefmtFor = forAllSystems (pkgs: treefmt-nix.lib.evalModule pkgs ./treefmt.nix);
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
            treefmtFor.${pkgs.stdenv.hostPlatform.system}.config.build.wrapper
          ];
          PROTOC = "${pkgs.protobuf}/bin/protoc";
          PROTOC_INCLUDE = "${pkgs.protobuf}/include";
        };
      });

      herculesCI = import ./effects.nix {
        pkgs = nixpkgs.legacyPackages.x86_64-linux;
        inherit nixbot;
      };

      formatter = forAllSystems (
        pkgs: treefmtFor.${pkgs.stdenv.hostPlatform.system}.config.build.wrapper
      );

      checks = forAllSystems (
        pkgs:
        let
          inherit (pkgs.stdenv.hostPlatform) system;
        in
        {
          package = self.packages.${system}.sediment;

          formatting = treefmtFor.${system}.config.build.check self;

          clippy = self.packages.${system}.sediment.overrideAttrs (old: {
            pname = "sediment-clippy";
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clippy ];
            buildPhase = ''
              runHook preBuild
              cargo clippy --all-targets --offline -- -D warnings
              runHook postBuild
            '';
            doCheck = false;
            installPhase = "touch $out";
            dontFixup = true;
          });
        }
      );
    };
}

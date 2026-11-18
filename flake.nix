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
      treefmtFor = forAllSystems (pkgs: treefmt-nix.lib.evalModule pkgs ./nix/treefmt.nix);
    in
    {
      packages = forAllSystems (pkgs: {
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.sand;
        sand = pkgs.callPackage ./nix/package.nix { };
        sediment = self.packages.${pkgs.stdenv.hostPlatform.system}.sand;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.callPackage ./nix/devshell.nix {
          treefmt = treefmtFor.${pkgs.stdenv.hostPlatform.system}.config.build.wrapper;
        };
      });

      herculesCI = import ./nix/effects.nix {
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
          package = self.packages.${system}.sand;

          formatting = treefmtFor.${system}.config.build.check self;

          clippy = self.packages.${system}.sand.overrideAttrs (old: {
            pname = "sand-clippy";
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

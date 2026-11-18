{
  projectRootFile = "flake.nix";

  programs = {
    nixfmt.enable = true;
    rustfmt = {
      enable = true;
      # match Cargo.toml so treefmt orders imports the way cargo fmt does
      edition = "2024";
    };
  };
}

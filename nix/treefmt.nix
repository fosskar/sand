{
  projectRootFile = "flake.nix";

  programs = {
    nixfmt.enable = true;
    rustfmt = {
      enable = true;
      # match Cargo.toml so treefmt orders imports the way cargo fmt does
      edition = "2024";
    };
    shellcheck.enable = true;
    shfmt = {
      enable = true;
      # treefmt-nix no longer defaults shfmt to 2 spaces; without this it indents with tabs
      indent_size = 2;
    };
  };
}

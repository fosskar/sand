# nixbot effects. the GitToken comes from nixbot at runtime (a github app
# installation token on github repos)
{ pkgs, nixbot }:
let
  inherit ((pkgs.lib.importTOML ../Cargo.toml).package) version;
  inherit (nixbot.lib.effects { inherit pkgs; }) mkEffect;
in
{ primaryRepo, ... }:
{
  # push effects run only after all checks passed on the commit. the tag push
  # starts .github/workflows/release.yml, which builds and publishes the release
  onPush.default.outputs.effects = pkgs.lib.optionalAttrs (primaryRepo.branch or null == "main") {
    release-tag = mkEffect {
      name = "effect-release-tag";
      inputs = [ pkgs.gh ];
      secretsMap.git.type = "GitToken";
      effectScript = ''
        GH_TOKEN=$(readSecretString git .token)
        export GH_TOKEN
        if response=$(gh api repos/fosskar/sand/git/ref/tags/v${version} 2>&1); then
          echo "v${version} exists"
        elif [[ $response == *"HTTP 404"* ]]; then
          gh api repos/fosskar/sand/git/refs -f ref=refs/tags/v${version} -f sha=${primaryRepo.rev}
        else
          echo "$response" >&2
          exit 1
        fi
      '';
    };
  };

  onSchedule.update-flake-inputs = {
    when = {
      hour = 2;
      minute = 30;
    };
    # nixbot mounts a pushable clone of the effect's commit at
    # $NIXBOT_EFFECT_CHECKOUT, which is also the working directory. the updater
    # comes from nixfiles, which needs no flake input to run
    outputs.effects.update-flake-inputs = mkEffect {
      name = "effect-update-flake-inputs";
      checkout = true;
      inputs = [ pkgs.nix ];
      secretsMap.git.type = "GitToken";
      effectScript = ''
        nix --extra-experimental-features 'nix-command flakes' \
          run github:fosskar/nixfiles#updater-effect -- flake-inputs
      '';
    };
  };
}

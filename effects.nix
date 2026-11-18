# nixbot effects. the GitToken comes from nixbot at runtime (a github app
# installation token on github repos)
{ pkgs, nixbot }:
let
  inherit (nixbot.lib.effects { inherit pkgs; }) mkEffect;
in
_args: {
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

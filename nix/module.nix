{
  config,
  lib,
  pkgs,
  utils,
  ...
}: let
  cfg = config.services.rustbg;
in {
  options.services.rustbg = {
    enable = lib.mkEnableOption "static Wayland wallpapers";
    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix {};
      defaultText = lib.literalExpression "pkgs.callPackage ./nix/package.nix {}";
      description = "The rustbg package to use.";
    };
    wallpapers = lib.mkOption {
      default = {};
      description = "Named wallpaper instances.";
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          image = lib.mkOption {
            type = lib.types.path;
            description = "Wallpaper image.";
          };
          fill = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Fill and center-crop instead of fitting the image.";
          };
          blur = lib.mkOption {
            type = lib.types.ints.between 0 1024;
            default = 0;
            description = "Gaussian blur sigma in output pixels.";
          };
        };
      });
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [cfg.package];
    systemd.user.services = lib.mapAttrs' (name: wallpaper:
      lib.nameValuePair "rustbg-${name}" {
        description = "Static ${name}";
        wantedBy = ["graphical-session.target"];
        after = ["graphical-session.target"];
        partOf = ["graphical-session.target"];
        serviceConfig = {
          ExecStart = utils.escapeSystemdExecArgs (
            ["${lib.getExe cfg.package}" "--namespace" "rustbg-${name}"]
            ++ lib.optional wallpaper.fill "--fill"
            ++ lib.optionals (wallpaper.blur != 0) ["--blur" (toString wallpaper.blur)]
            ++ [wallpaper.image]
          );
          Restart = "on-failure";
        };
      })
    cfg.wallpapers;
  };
}

# Rustbg

Minimal static wallpapers for Wayland. Forked from [rusty-wl-utils](https://github.com/spaceavenue/rusty-wl-utils) by [spaceavenue](https://github.com/spaceavenue).

Supports x86-64 Linux and compositors with `wlr-layer-shell`.

For NixOS, add `github:greyxp1/rustbg` as a flake input:

```nix
{inputs, ...}: {
  imports = [inputs.rustbg.nixosModules.default];
  services.rustbg = {
    enable = true;
    wallpapers = {
      wallpaper.image = ./wallpaper.jpg; # Namespace: rustbg-wallpaper
      backdrop = {                       # Namespace: rustbg-backdrop
        image = ./wallpaper.jpg;
        blur = 7;                        # Blur strength, 0–1024.
      };
    };
  };
}
```

For a blurred Niri overview, match `rustbg-backdrop` in a layer rule with `place-within-backdrop true`.

Source builds require nightly Rust, `rust-src` and FFmpeg.

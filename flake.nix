{
  description = "Minimal static Wayland wallpaper client";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/f9a75f41be3a74eab55d3a7679d34ad6cc0da977";

  outputs = {nixpkgs, ...}: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {inherit system;};
    rustbg = pkgs.callPackage ./nix/package.nix {};
  in {
    packages.${system} = {
      inherit rustbg;
      default = rustbg;
    };
    devShells.${system}.default = pkgs.mkShell {
      inputsFrom = [rustbg];
      inherit (rustbg) RUSTC_BOOTSTRAP RUSTFLAGS;
    };
    nixosModules.default = ./nix/module.nix;
  };
}

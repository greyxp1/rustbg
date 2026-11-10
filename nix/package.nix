{
  lib,
  rustc,
  cargo,
  rustPlatform,
  makeRustPlatform,
  symlinkJoin,
  makeWrapper,
  installShellFiles,
  ffmpeg-headless,
}: let
  sysroot = symlinkJoin {
    name = "rustbg-rust-sysroot";
    paths = [rustc.unwrapped];
    postBuild = ''
      mkdir -p $out/lib/rustlib/src/rust
      ln -s ${rustPlatform.rustLibSrc} $out/lib/rustlib/src/rust/library
    '';
  };
  platform = makeRustPlatform {
    inherit cargo;
    rustc = rustc.override {inherit sysroot;};
  };
in
  platform.buildRustPackage {
    pname = "rustbg";
    version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.version;
    src = lib.cleanSource ../.;
    cargoDeps = symlinkJoin {
      name = "cargo-vendor-dir";
      paths = [
        (platform.importCargoLock {
          lockFile = ../Cargo.lock;
          outputHashes."wllib-0.1.0" = "sha256-3ujv1uZZ4Je+xDCheSS2nTBGYytzqV7FIoNHHxkCVB8=";
        })
        "${rustPlatform.rustLibSrc}/vendor"
      ];
    };
    doCheck = false;
    env = {
      RUSTC_BOOTSTRAP = "1";
      RUSTFLAGS = lib.concatStringsSep " " (builtins.fromTOML (builtins.readFile ../.cargo/config.toml)).build.rustflags;
    };
    nativeBuildInputs = [makeWrapper installShellFiles];
    postInstall = ''
      installManPage rustbg.1
    '';
    postFixup = ''
      wrapProgram $out/bin/rustbg --prefix PATH : ${lib.makeBinPath [ffmpeg-headless]}
    '';
    meta = {
      description = "Minimal static Wayland wallpaper client";
      homepage = "https://github.com/greyxp1/rustbg";
      mainProgram = "rustbg";
      platforms = ["x86_64-linux"];
    };
  }

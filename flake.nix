{
  description = "Hestia Flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        linuxGui = with pkgs; lib.optionals stdenv.hostPlatform.isLinux [
          wayland
          libxkbcommon
          xorg.libxcb
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          vulkan-loader
          fontconfig
          freetype
          xdg-utils
        ];
      in
      {
        devShells.default = with pkgs; mkShell {
          name = "hestia";
          packages = [
            (rust-bin.stable.latest.default.override { extensions = [ "rust-src" ]; })
            just
            sea-orm-cli
            sqlite
            openssl
            pkg-config
            cmake
            libiconv
          ] ++ linuxGui;
          NIX_SHELL = "hestia";
          LD_LIBRARY_PATH = lib.makeLibraryPath linuxGui;
        };
      }
    );
}

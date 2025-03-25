{pkgs ? import <nixpkgs-unstable> {}}: 
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ rustc rustfmt clippy cargo openssl xorg.libX11 xdotool atk webkitgtk_4_1 sqlite sqlite.dev openssl.dev pkg-config];
  PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
  RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
}

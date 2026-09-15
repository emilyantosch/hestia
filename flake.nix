{
  description = "Hestia Flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    nixpkgs,
    rust-overlay,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        qt = pkgs.symlinkJoin {
          name = "hestia-qt";
          paths = [pkgs.qt6.qtbase pkgs.qt6.qtdeclarative];
        };
        qmake = pkgs.writeShellScript "hestia-qmake" ''
          if [ "$1" = "-query" ]; then
            case "$2" in
              QT_INSTALL_PREFIX|QT_HOST_PREFIX|QT_INSTALL_ARCHDATA|QT_HOST_DATA) echo "${qt}"; exit ;;
              QT_INSTALL_HEADERS) echo "${qt}/include"; exit ;;
              QT_INSTALL_LIBS|QT_HOST_LIBS) echo "${qt}/lib"; exit ;;
              QT_INSTALL_PLUGINS) echo "${qt}/lib/qt-6/plugins"; exit ;;
              QT_INSTALL_QML) echo "${qt}/lib/qt-6/qml"; exit ;;
              QT_INSTALL_LIBEXECS|QT_HOST_LIBEXECS) echo "${qt}/libexec"; exit ;;
              QT_INSTALL_BINS|QT_HOST_BINS) echo "${qt}/bin"; exit ;;
            esac
          fi
          exec ${pkgs.qt6.qtbase}/bin/qmake "$@"
        '';
      in {
        devShells.default = with pkgs;
          mkShell {
            name = "hestia";
            buildInputs = [
              (rust-bin.stable.latest.default.override {extensions = ["rust-src"];})
              sea-orm-cli
              sqlite
              sqlite.dev
              openssl.dev
              pkg-config
              libiconv
              libglvnd
              just
              qt
            ];
            nativeBuildInputs = [
              pkg-config
              openssl.dev
            ];
            NIX_SHELL = "hestia";
            QMAKE = qmake;
            # cxx-qt needs the framework search path for Qt headers on macOS.
            CXXFLAGS = lib.optionalString stdenv.hostPlatform.isDarwin "-F${qt}/lib";
            QML_IMPORT_PATH = "${qt}/lib/qt-6/qml";
            QML2_IMPORT_PATH = "${qt}/lib/qt-6/qml";
          };
      }
    );
}

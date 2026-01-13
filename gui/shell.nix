{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  packages = [
    pkgs.rustc
    pkgs.cargo
    pkgs.pkg-config
    pkgs.libxkbcommon
    pkgs.wayland
    pkgs.wayland-protocols
    pkgs.xorg.libX11
    pkgs.xorg.libXcursor
    pkgs.xorg.libXi
    pkgs.xorg.libXrandr
    pkgs.xorg.libXinerama
  ];
}

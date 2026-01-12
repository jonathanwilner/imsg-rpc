{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  packages = [
    pkgs.rust-bin.stable.latest.default
    pkgs.pkg-config
  ];
}

{
  description = "imsg-gui development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" "x86_64-darwin" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "imsg-gui";
            version = "0.1.0";
            src = ./.;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            nativeBuildInputs = [ pkgs.makeWrapper pkgs.pkg-config ];
            buildInputs = [
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.wayland-protocols
              pkgs.mesa
              pkgs.xorg.libX11
              pkgs.xorg.libXcursor
              pkgs.xorg.libXi
              pkgs.xorg.libXrandr
              pkgs.xorg.libXinerama
            ];
            postInstall = ''
              install -Dm755 nixos/imsg-gui-net $out/bin/imsg-gui-net
              install -Dm644 nixos/imsg-gui.desktop $out/share/applications/imsg-gui.desktop
              wrapProgram $out/bin/imsg-gui \
                --set WINIT_UNIX_BACKEND wayland \
                --set WGPU_BACKEND gl \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [
                  pkgs.libxkbcommon
                  pkgs.wayland
                  pkgs.mesa
                  pkgs.xorg.libX11
                  pkgs.xorg.libXcursor
                  pkgs.xorg.libXi
                  pkgs.xorg.libXrandr
                  pkgs.xorg.libXinerama
                ]}
            '';
          };
        });

      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.rustc
              pkgs.cargo
              pkgs.pkg-config
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.wayland-protocols
              pkgs.mesa
              pkgs.xorg.libX11
              pkgs.xorg.libXcursor
              pkgs.xorg.libXi
              pkgs.xorg.libXrandr
              pkgs.xorg.libXinerama
            ];
          };
        });
    };
}

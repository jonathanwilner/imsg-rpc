{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  packages = [
    pkgs.rustc
    pkgs.cargo
    pkgs.pkg-config
    pkgs.libnotify
    pkgs.xdg-utils
  ];
  shellHook = ''
    export IMSG_TUI_TRANSPORT=''${IMSG_TUI_TRANSPORT:-tcp}
    export IMSG_TUI_HOST=''${IMSG_TUI_HOST:-192.168.2.186}
    export IMSG_TUI_PORT=''${IMSG_TUI_PORT:-57999}
    export COLORTERM=''${COLORTERM:-truecolor}
    cargo build --release >/dev/null
    echo "Starting imsg-tui (transport=$IMSG_TUI_TRANSPORT host=$IMSG_TUI_HOST port=$IMSG_TUI_PORT)"
    exec ./target/release/imsg-tui --transport "$IMSG_TUI_TRANSPORT" --host "$IMSG_TUI_HOST" --port "$IMSG_TUI_PORT"
  '';
}

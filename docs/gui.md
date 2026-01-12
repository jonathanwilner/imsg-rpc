# imsg GUI (COSMIC + iced)

This is a Rust GUI client for `imsg` using the iced toolkit with a
COSMIC-inspired layout. It supports local subprocess and TCP (socat)
transport, matching the Emacs/TUI clients.

## Build

```
cd gui
cargo build --release
```

Binary:

```
./gui/target/release/imsg-gui
```

## Usage

Local (spawns `imsg rpc`):

```
./gui/target/release/imsg-gui --transport local --imsg-bin imsg
```

TCP (connect to socat wrapper):

```
./gui/target/release/imsg-gui --transport tcp --host 192.168.2.186 --port 57999
```

Optional database path:

```
./gui/target/release/imsg-gui --transport local --db ~/Library/Messages/chat.db
```

## Nix build (flake)

From `gui/`:

```
nix build .#
```

Run the built binary:

```
./result/bin/imsg-gui
```

Dev shell:

```
nix develop
```

This uses `rustc`/`cargo` from `nixpkgs` (no `rust-bin` overlay required).

## Nix build (shell.nix)

From `gui/`:

```
nix-shell
```

## Controls

- Refresh: reload chat list
- History: load history for selected chat
- Watch: toggle watch for selected chat
- Send: send to selected chat
- Direct: enter recipient, then message
- React: send a reaction for the selected message
- Cancel: clear current input
- Click a message bubble to select it
- Click URL chips under a message to open in your browser (`xdg-open` on Linux, `open` on macOS)

## Notifications

Desktop notifications are best-effort via `notify-rust`. Disable with:

```
./gui/target/release/imsg-gui --notify false
```

## Transport notes

- Local mode uses `imsg rpc` as a subprocess.
- TCP mode expects the macOS `socat` wrapper listening on `57999`
  (see `docs/remote-emacs.md` for setup).

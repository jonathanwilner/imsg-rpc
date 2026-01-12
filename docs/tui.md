# imsg TUI (Ratatui)

This is a Rust TUI client for `imsg` that mirrors the Emacs client:
- list chats
- view message history
- watch new messages
- send messages
- supports local subprocess or TCP socket (socat) transport

## Build

```
cd tui
cargo build --release
```

Binary:

```
./tui/target/release/imsg-tui
```

## Usage

Local (spawns `imsg rpc`):

```
./tui/target/release/imsg-tui --transport local --imsg-bin imsg
```

TCP (connect to socat wrapper):

```
./tui/target/release/imsg-tui --transport tcp --host 192.168.2.186 --port 57999
```

Optional database path:

```
./tui/target/release/imsg-tui --transport local --db ~/Library/Messages/chat.db
```

## Nix build (flake)

From `tui/`:

```
nix build .#
```

Run the built binary:

```
./result/bin/imsg-tui
```

Dev shell:

```
nix develop
```

## Nix build (shell.nix)

From `tui/`:

```
nix-shell
```

## Keys

- `q` quit
- `r` refresh chats
- `Up/Down` select chat
- `Enter` load history for selected chat
- `w` toggle watch for selected chat
- `s` send message to selected chat
- `n` new direct message (prompt for recipient + text)
- `Esc` cancel input

## Notifications

Desktop notifications are best-effort via `notify-rust`.
Disable with `--notify false`.

## Transport notes

- Local mode uses `imsg rpc` as a subprocess.
- TCP mode expects the macOS `socat` wrapper listening on `57999`
  (see `docs/remote-emacs.md` for setup).

## Requirements (Linux)

- Rust stable or Nix dev shell/flake.
- A notification daemon if you want popups (e.g., `dunst`, GNOME).

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

## Controls

- Refresh: reload chat list
- History: load history for selected chat
- Watch: toggle watch for selected chat
- Send: send to selected chat
- Direct: enter recipient, then message
- Cancel: clear current input

## Notifications

Desktop notifications are best-effort via `notify-rust`. Disable with:

```
./gui/target/release/imsg-gui --notify false
```

## Transport notes

- Local mode uses `imsg rpc` as a subprocess.
- TCP mode expects the macOS `socat` wrapper listening on `57999`
  (see `docs/remote-emacs.md` for setup).

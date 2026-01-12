# Remote Emacs Client (Linux/Emacs 30+) + macOS RPC

This document explains how to run `imsg rpc` on a Mac host and use Emacs
from a remote Linux machine (including NixOS with emacs-pgtk 30.1+) to
send/receive messages via TRAMP over SSH.

## Overview

`imsg` provides a JSON-RPC 2.0 interface over stdin/stdout. The Emacs client
starts an `imsg rpc` process on the Mac host via TRAMP, then exchanges
JSON lines over that SSH session. For always-on connections, a `socat`
TCP wrapper can provide a long-lived socket service.

ASCII flow, high level:

```
Linux Emacs (TRAMP/SSH)          macOS host
----------------------          -----------
imsg.el                          imsg rpc
  |  JSON-RPC over stdio  --->     |  reads stdin
  |  <--- JSON-RPC responses       |  writes stdout
```

Alternate flow (socket wrapper):

```
Linux Emacs (TCP)                 macOS host
------------------                -----------
imsg.el (network)  ---> TCP --->  socat  ---> imsg rpc
```

## Step-by-step: start the macOS RPC at boot

We ship two LaunchAgent plists:
- `imsg rpc` (on-demand; exits when stdin closes)
- `socat` wrapper (persistent TCP listener)

ASCII steps:

```
[1] Copy plist    [2] Load     [3] Verify
    |                 |             |
    v                 v             v
~/Library/LaunchAgents  launchctl    log file
```

1) Install the LaunchAgent plist:

```
cp launchctl/com.jonathan.imsg.rpc.plist ~/Library/LaunchAgents/
```

2) Load it:

```
launchctl load -w ~/Library/LaunchAgents/com.jonathan.imsg.rpc.plist
```

3) Check logs:

```
tail -f ~/Library/Logs/imsg-rpc.out.log
tail -f ~/Library/Logs/imsg-rpc.err.log
```

Notes:
- The plist assumes the binary lives at `/Users/jonathan/bin/imsg`.
  If you install it elsewhere, update `ProgramArguments` accordingly.
- If you prefer a custom DB path, add `--db /path/to/chat.db` in `ProgramArguments`.
- `imsg rpc` exits when stdin closes. Use the `socat` wrapper for a
  persistent listener.

### Persistent socket mode (socat)

This keeps a TCP port open and spawns `imsg rpc` per connection.

1) Install the plist:

```
cp launchctl/com.jonathan.imsg.rpc.socat.plist ~/Library/LaunchAgents/
```

2) Load it:

```
launchctl load -w ~/Library/LaunchAgents/com.jonathan.imsg.rpc.socat.plist
```

3) Check logs:

```
tail -f ~/Library/Logs/imsg-rpc-socat.out.log
tail -f ~/Library/Logs/imsg-rpc-socat.err.log
```

Default port is `57999`. Override via:

```
IMSG_PORT=58000 /Users/jonathan/src/imsg/scripts/start-rpc-socat.sh
```

Bind to localhost only (recommended with Cloudflare Tunnel):

```
IMSG_BIND=127.0.0.1 /Users/jonathan/src/imsg/scripts/start-rpc-socat.sh
```

Security note: keep this behind a firewall or bind only on localhost
if you proxy over SSH.

## Sign-in and permissions (macOS)

You must be signed in to Messages on the Mac host. This cannot be automated
by the LaunchAgent. Ensure:

1) Messages.app is signed in.
2) Terminal (or the agent) has Full Disk Access for `~/Library/Messages/chat.db`.
3) Automation permission is granted if you use `send`.

ASCII checklist:

```
[Messages signed-in] -> [Full Disk Access] -> [Automation permission]
```

### Full Disk Access for LaunchAgent (no CLI bypass)

macOS TCC does not allow granting Full Disk Access from the command line.
You must approve it manually in System Settings.

Recommended steps:

1) Run once in Terminal to trigger the prompt:

```
~/bin/imsg rpc
```

2) Approve Full Disk Access for the Terminal (or for the `imsg` binary).

3) Reload the LaunchAgent:

```
launchctl unload ~/Library/LaunchAgents/com.jonathan.imsg.rpc.plist
launchctl load -w ~/Library/LaunchAgents/com.jonathan.imsg.rpc.plist
```

ASCII flow:

```
run once -> approve FDA -> reload LaunchAgent
```

## Emacs client configuration (Linux)

The Emacs client is in `emacs/imsg.el`. It supports local, TRAMP, or TCP
network mode.

Minimal setup:

```elisp
(add-to-list 'load-path "/path/to/imsg/emacs")
(require 'imsg)

(setq imsg-remote-host "192.168.2.186")
(setq imsg-remote-user "jonathan")
(setq imsg-remote-method "ssh")

(imsg-use-remote)
```

Socket setup:

```elisp
(setq imsg-network-host "192.168.2.186")
(setq imsg-network-port 57999)
(imsg-use-network imsg-network-host imsg-network-port)
```

Open the transient menu:

```
M-x imsg-transient
```

ASCII flow for TRAMP:

```
Emacs --> /ssh:jonathan@192.168.2.186: --> start-file-process --> imsg rpc
```

ASCII flow for socket mode:

```
Emacs --> TCP:57999 --> socat --> imsg rpc
```

## Notifications

Incoming messages trigger Emacs notifications by default.
You can customize or disable them:

```elisp
(setq imsg-notify-enabled t)
(setq imsg-notify-function
      (lambda (message)
        (notifications-notify
         :title (alist-get 'sender message)
         :body (alist-get 'text message)
         :app-name "imsg")))
```

To disable:

```elisp
(setq imsg-notify-enabled nil)
```

## Remote tests

There are two remote test layers:

1) RPC smoke test (SSH + JSON-RPC):

```
make remote-test
```

2) Emacs SSH auth test (ERT):

This is run automatically by `make remote-test` if Emacs is installed.

Environment variables:

- `IMSG_REMOTE_HOST` (default `192.168.2.186`)
- `IMSG_REMOTE_USER` (default `jonathan`)
- `IMSG_REMOTE_BIN` (default `~/bin/imsg`)
- `IMSG_REMOTE_SEND_TO` (optional; enables send test)
- `IMSG_REMOTE_SEND_TEXT` (optional text)

ASCII test flow:

```
make remote-test
  |-- SSH auth ok?
  |-- imsg rpc: chats.list, history, watch subscribe/unsubscribe
  '-- optional send if IMSG_REMOTE_SEND_TO is set
```

## Cloudflare Tunnel (optional)

Expose the TCP port safely through an existing Cloudflare Tunnel.

Mac host (`cloudflared` config):

```
tunnel: <your-tunnel-uuid>
credentials-file: /Users/jonathan/.cloudflared/<your-tunnel-uuid>.json
ingress:
  - hostname: imsg-rpc.example.com
    service: tcp://localhost:57999
  - service: http_status:404
```

Client (remote machine):

```
cloudflared access tcp --hostname imsg-rpc.example.com --url 127.0.0.1:57999
```

Then point Emacs at `localhost:57999`:

```elisp
(imsg-use-network "127.0.0.1" 57999)
```

This same TCP tunnel can be used by the Rust TUI client
(`docs/tui.md`) by pointing it at `127.0.0.1:57999`.

## Keeping `imsg rpc` running manually

If you do not want LaunchAgent, you can run manually:

```
~/bin/imsg rpc
```

Leave it running in a terminal or a tmux session.

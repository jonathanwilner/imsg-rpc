{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  packages = with pkgs; [
    netcat
    python3
    (writeShellScriptBin "imsg-rpc-test" ''
      python - <<'PY'
import json, socket, os
host = os.environ.get("IMSG_RPC_HOST", "192.168.2.186")
port = int(os.environ.get("IMSG_RPC_PORT", "57999"))
limit = int(os.environ.get("IMSG_RPC_LIMIT", "1"))
req = {"jsonrpc": "2.0", "id": "1", "method": "chats.list", "params": {"limit": limit}}
blob = json.dumps(req).encode() + b"\\n"
with socket.create_connection((host, port), timeout=3) as s:
    s.sendall(blob)
    s.settimeout(3)
    print(s.recv(4096).decode().strip())
PY
    '')
  ];
}

#!/usr/bin/env bash
set -euo pipefail

HOST="${IMSG_REMOTE_HOST:-192.168.2.186}"
USER="${IMSG_REMOTE_USER:-jonathan}"
REMOTE="${USER}@${HOST}"
REMOTE_BIN="${IMSG_REMOTE_BIN:-~/bin/imsg}"
SSH_OPTS=(
  -o BatchMode=yes
  -o ConnectTimeout=5
)

printf "imsg remote e2e: %s\n" "${REMOTE}"

ssh "${SSH_OPTS[@]}" "${REMOTE}" true
ssh "${SSH_OPTS[@]}" "${REMOTE}" "test -x ${REMOTE_BIN}"

ssh "${SSH_OPTS[@]}" "${REMOTE}" python3 - "${REMOTE_BIN}" <<'PY'
import json
import os
import subprocess
import sys

remote_bin = sys.argv[1]
proc = subprocess.Popen(
    [remote_bin, "rpc"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    text=True,
)

def request(method, params=None, request_id="1"):
    payload = {"jsonrpc": "2.0", "id": request_id, "method": method}
    if params:
        payload["params"] = params
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()
    while True:
        line = proc.stdout.readline()
        if not line:
            raise RuntimeError("rpc closed")
        data = json.loads(line)
        if data.get("id") == request_id:
            if "error" in data:
                raise RuntimeError(data["error"])
            return data.get("result")

result = request("chats.list", {"limit": 1}, "1")
chats = result.get("chats", [])
if not chats:
    raise RuntimeError("no chats returned")

chat_id = chats[0].get("id")
if not chat_id:
    raise RuntimeError("missing chat id")

request("messages.history", {"chat_id": chat_id, "limit": 1}, "2")
watch = request("watch.subscribe", {"chat_id": chat_id}, "3")
subscription = watch.get("subscription")
if not subscription:
    raise RuntimeError("missing subscription")
request("watch.unsubscribe", {"subscription": subscription}, "4")

send_to = None
try:
    send_to = os.environ.get("IMSG_REMOTE_SEND_TO")
except Exception:
    send_to = None

if send_to:
    text = os.environ.get("IMSG_REMOTE_SEND_TEXT", "imsg remote e2e test")
    request("send", {"to": send_to, "text": text}, "5")

proc.stdin.close()
proc.stdout.close()
proc.terminate()
print("ok")
PY

if command -v emacs >/dev/null 2>&1; then
  IMSG_REMOTE_E2E=1 \
    IMSG_REMOTE_HOST="${HOST}" \
    IMSG_REMOTE_USER="${USER}" \
    emacs --batch -Q -L emacs -l emacs/imsg-test.el -f ert-run-tests-batch-and-exit
else
  printf "emacs not found; skipping emacs remote ssh test\n"
fi

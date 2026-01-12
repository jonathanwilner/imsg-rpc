#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  exit 0
fi

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT}/bin}"
IMSG_BIN="${IMSG_BIN:-${OUTPUT_DIR}/imsg}"
APP_PATH="${IMSG_RPC_APP_PATH:-/Applications/IMsgRPC.app}"
FALLBACK_PATH="${HOME}/Applications/IMsgRPC.app"

if [[ "${IMSG_RPC_APP_SKIP:-}" == "1" ]]; then
  exit 0
fi

if [[ ! -x "${IMSG_BIN}" ]]; then
  printf "imsg binary not found at %s\n" "${IMSG_BIN}" >&2
  exit 1
fi

SOCAT_BIN=""
if [[ -x /opt/homebrew/bin/socat ]]; then
  SOCAT_BIN="/opt/homebrew/bin/socat"
elif [[ -x /usr/local/bin/socat ]]; then
  SOCAT_BIN="/usr/local/bin/socat"
elif command -v socat >/dev/null 2>&1; then
  SOCAT_BIN="$(command -v socat)"
fi

update_app() {
  local target="$1"
  local app_bin="${target}/Contents/MacOS/imsg"
  local app_socat="${target}/Contents/MacOS/socat"

  mkdir -p "${target}/Contents/MacOS" || return 1

  cat > "${target}/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>CFBundleExecutable</key>
    <string>imsg</string>
    <key>CFBundleIdentifier</key>
    <string>com.jonathan.imsg.rpc</string>
    <key>CFBundleName</key>
    <string>IMsgRPC</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSUIElement</key>
    <true/>
  </dict>
</plist>
PLIST

  cp "${IMSG_BIN}" "${app_bin}" || return 1
  chmod +x "${app_bin}" || return 1

  if [[ -n "${SOCAT_BIN}" && -x "${SOCAT_BIN}" ]]; then
    cp "${SOCAT_BIN}" "${app_socat}" || return 1
    chmod +x "${app_socat}" || return 1
  else
    printf "socat not found; skipping bundled socat\n" >&2
  fi

codesign --force --deep --sign - "${target}" || return 1
printf "Updated %s\n" "${target}"
if [[ "${IMSG_RPC_APP_RELOAD:-}" == "1" ]]; then
  if [[ -f "${HOME}/Library/LaunchAgents/com.jonathan.imsg.rpc.socat.plist" ]]; then
    launchctl unload "${HOME}/Library/LaunchAgents/com.jonathan.imsg.rpc.socat.plist" || true
    launchctl load "${HOME}/Library/LaunchAgents/com.jonathan.imsg.rpc.socat.plist"
  fi
  if [[ -f "${HOME}/Library/LaunchAgents/com.jonathan.imsg.rpc.plist" ]]; then
    launchctl unload "${HOME}/Library/LaunchAgents/com.jonathan.imsg.rpc.plist" || true
    launchctl load "${HOME}/Library/LaunchAgents/com.jonathan.imsg.rpc.plist"
  fi
fi
  return 0
}

if ! update_app "${APP_PATH}"; then
  printf "Using fallback app path: %s\n" "${FALLBACK_PATH}" >&2
  printf "Update launchctl plists if you want launchd to use the fallback app\n" >&2
  update_app "${FALLBACK_PATH}"
fi

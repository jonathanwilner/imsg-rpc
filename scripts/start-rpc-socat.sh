#!/usr/bin/env bash
set -euo pipefail

IMSG_BIN="${IMSG_BIN:-/Users/jonathan/bin/imsg}"
IMSG_PORT="${IMSG_PORT:-57999}"
IMSG_DB="${IMSG_DB:-}"
IMSG_BIND="${IMSG_BIND:-}"

SOCAT_BIN="${SOCAT_BIN:-}"
if [[ -z "${SOCAT_BIN}" ]]; then
  if [[ -x /opt/homebrew/bin/socat ]]; then
    SOCAT_BIN="/opt/homebrew/bin/socat"
  elif [[ -x /usr/local/bin/socat ]]; then
    SOCAT_BIN="/usr/local/bin/socat"
  elif command -v socat >/dev/null 2>&1; then
    SOCAT_BIN="$(command -v socat)"
  fi
fi

if [[ ! -x "${IMSG_BIN}" ]]; then
  printf "imsg binary not found: %s\n" "${IMSG_BIN}" >&2
  exit 1
fi
if [[ -z "${SOCAT_BIN}" || ! -x "${SOCAT_BIN}" ]]; then
  printf "socat not found; set SOCAT_BIN or install via brew\n" >&2
  exit 127
fi

ARGS=(rpc)
if [[ -n "${IMSG_DB}" ]]; then
  ARGS+=(--db "${IMSG_DB}")
fi

LISTEN_OPTS="TCP-LISTEN:${IMSG_PORT},fork,reuseaddr"
if [[ -n "${IMSG_BIND}" ]]; then
  LISTEN_OPTS="${LISTEN_OPTS},bind=${IMSG_BIND}"
fi

exec "${SOCAT_BIN}" "${LISTEN_OPTS}" "EXEC:${IMSG_BIN} ${ARGS[*]}"

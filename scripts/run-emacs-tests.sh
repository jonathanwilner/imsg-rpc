#!/usr/bin/env bash
set -euo pipefail

if ! command -v emacs >/dev/null 2>&1; then
  printf "emacs not found; skipping emacs tests\n"
  exit 0
fi

emacs --batch -Q -L emacs -l emacs/imsg-test.el -f ert-run-tests-batch-and-exit

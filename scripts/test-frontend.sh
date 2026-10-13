#!/usr/bin/env bash
#
# React frontend tests (`tasks/027.md` §31, §32). Runs the TypeScript type
# check and the Vitest suite for the Tauri desktop webview against mocked
# desktop API responses.
#
# Usage:
#   ./scripts/test-frontend.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root/apps/desktop"

if [[ ! -d node_modules ]]; then
  echo "ERROR: apps/desktop/node_modules is missing; run 'npm install' first" >&2
  exit 1
fi

echo "==> Frontend type check"
npm run typecheck

echo "==> Frontend unit tests (Vitest)"
npm run test

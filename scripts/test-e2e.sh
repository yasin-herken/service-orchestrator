#!/usr/bin/env bash
#
# End-to-end tests (`tasks/027.md` §33, §34). These drive the real binaries and
# the real daemon over their real transports:
#
#   * the `service-orchestrator mcp` stdio server, end to end;
#   * the shared local daemon over the Unix-domain-socket IPC protocol;
#   * the Tauri desktop adapter against a real daemon (opt-in, see below).
#
# The desktop adapter lives in its own toolchain-facing workspace and is the
# slowest to build, so it runs only when `RUN_DESKTOP_TESTS=1` is set.
#
# The opt-in `#[ignore]`d performance/scale harnesses run only when
# `INCLUDE_PERFORMANCE=1` is set (see `docs/performance-report.md`).
#
# Usage:
#   ./scripts/test-e2e.sh
#   RUN_DESKTOP_TESTS=1 ./scripts/test-e2e.sh
#   INCLUDE_PERFORMANCE=1 ./scripts/test-e2e.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if [[ "${RUN_DESKTOP_TESTS:-0}" == "1" ]]; then
  echo "==> Desktop adapter tests (Tauri unit + IPC end-to-end)"
  # The desktop crate declares the orchestration binary as a Tauri external
  # binary (bundle.externalBin), so tauri-build requires the triple-named
  # sidecar to exist before the crate compiles (tasks/031.md §12).
  node apps/desktop/scripts/preflight.mjs --profile debug --stage-only
  (cd apps/desktop/src-tauri && cargo test)
fi

echo "==> MCP stdio end-to-end test (real binary)"
cargo test -p service-orchestrator --test mcp_stdio

echo "==> Daemon IPC integration tests (real Unix-domain socket)"
cargo test -p service-orchestrator-daemon --test daemon

if [[ "${INCLUDE_PERFORMANCE:-0}" == "1" ]]; then
  echo "==> Opt-in performance/scale harnesses (--ignored)"
  cargo test --workspace -- --ignored --nocapture --test-threads=1
fi

echo "==> End-to-end tests complete"

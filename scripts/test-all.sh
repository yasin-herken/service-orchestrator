#!/usr/bin/env bash
#
# The complete local validation gate (`tasks/027.md` §40, §50): formatting,
# compilation, the Rust unit + integration + security suites, the lints, and
# the frontend type check, tests, and production build.
#
# The slow end-to-end desktop adapter tests and the opt-in performance
# harnesses are not part of the default gate; run `test-e2e.sh` with
# `RUN_DESKTOP_TESTS=1` / `INCLUDE_PERFORMANCE=1` for those.
#
# Usage:
#   ./scripts/test-all.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> cargo check --workspace"
cargo check --workspace

echo "==> Rust unit tests"
./scripts/test-unit.sh

echo "==> Rust integration tests"
./scripts/test-integration.sh

echo "==> Security regression suite"
./scripts/test-security.sh

echo "==> cargo clippy"
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "==> Frontend type check and tests"
./scripts/test-frontend.sh

echo "==> Frontend production build"
(cd apps/desktop && npm run build)

echo "==> All local gates passed"

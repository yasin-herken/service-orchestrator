#!/usr/bin/env bash
#
# Fast Rust unit tests: every library crate's inline `#[cfg(test)]` modules
# (`tasks/027.md` §6, §7–§24). This is the quickest meaningful category and is
# safe to run on every change.
#
# Usage:
#   ./scripts/test-unit.sh [extra cargo-test args...]

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

echo "==> Rust unit tests (workspace --lib)"
cargo test --workspace --lib "$@"

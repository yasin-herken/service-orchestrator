#!/usr/bin/env bash
#
# Rust integration tests: every `tests/*.rs` target under `crates/*/tests`.
# Each target is discovered and run by name so the category is precise and does
# not silently rely on cargo's broader target selection (`tasks/027.md` §6).
#
# Usage:
#   ./scripts/test-integration.sh [extra cargo-test args...]

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

run_target() {
  local file="$1"
  local tests_dir crate_dir package target
  tests_dir="$(dirname "$file")"
  crate_dir="$(dirname "$tests_dir")"
  package="$(sed -nE 's/^name[[:space:]]*=[[:space:]]*"(.*)"/\1/p' "$crate_dir/Cargo.toml" | head -n1)"
  target="$(basename "$file" .rs)"
  if [[ -z "$package" ]]; then
    echo "ERROR: could not resolve package for $file" >&2
    return 1
  fi
  echo "--> $package :: $target"
  cargo test -p "$package" --test "$target" "$@"
}

# Only the crate integration targets; the root `tests/mcp_stdio.rs` end-to-end
# target belongs to `test-e2e.sh`.
found=0
while IFS= read -r file; do
  run_target "$file"
  found=1
done < <(find crates/*/tests -maxdepth 1 -name '*.rs' 2>/dev/null | sort)

if [[ "$found" -eq 0 ]]; then
  echo "ERROR: no crate integration test targets were found" >&2
  exit 1
fi

echo "==> Rust integration tests complete"

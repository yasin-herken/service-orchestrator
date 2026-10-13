#!/usr/bin/env bash
#
# Lightweight documentation validation (`tasks/029.md`).
#
# For `README.md` and every Markdown file under `docs/` this script checks:
#   1. every relative Markdown link resolves to an existing file or directory;
#   2. every repository path written in backticks (scripts/, examples/, crates/,
#      apps/, src/, tests/, docs/) exists on disk;
#   3. every `scripts/*.sh` is executable;
#   4. `examples/config.toml` exists and is structurally a versioned config.
#
# The authoritative parse of `examples/config.toml` is the Rust test
# `loads_the_full_example_configuration`; pass `--with-config-test` to run it.
# External links (http/https/mailto) and pure in-page anchors are not fetched.
#
# Usage:
#   ./scripts/check-docs.sh [--with-config-test]

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

with_config_test=0
for arg in "$@"; do
  case "$arg" in
    --with-config-test) with_config_test=1 ;;
    -h | --help)
      sed -n '2,19p' "$0"
      exit 0
      ;;
    *)
      echo "unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

failures=0
fail() { echo "  FAIL $*"; failures=$((failures + 1)); }
pass() { echo "  ok   $*"; }

files="$(printf '%s\n' README.md; find docs -type f -name '*.md' | sort)"

check_links() {
  local file="$1" dir target resolved
  dir="$(dirname "$file")"

  # Inline links `[text](target)` and reference definitions `[id]: target`.
  local targets
  targets="$(
    { grep -oE '\]\([^)]*\)' "$file" | sed -E 's/^\]\(//; s/\)$//' || true
      grep -oE '^\[[^]]+\]:[[:space:]]+[^[:space:]]+' "$file" \
        | sed -E 's/^\[[^]]+\]:[[:space:]]+//' || true
    } 2>/dev/null
  )"

  while IFS= read -r target; do
    [ -z "$target" ] && continue
    case "$target" in
      http://* | https://* | mailto:* | \#*) continue ;;
    esac
    # Strip any fragment before resolving the path.
    target="${target%%#*}"
    [ -z "$target" ] && continue
    case "$target" in
      /*) resolved="${target#/}" ;;
      *) resolved="$dir/$target" ;;
    esac
    if [ ! -e "$resolved" ]; then
      fail "$file: broken link '$target'"
    fi
  done <<< "$targets"
}

check_paths() {
  local file="$1" token
  local refs
  refs="$(
    grep -oE '`(scripts|examples|crates|apps|src|tests|docs)/[A-Za-z0-9_./-]+\.[A-Za-z0-9]+`' "$file" \
      | tr -d '`' || true
  )"
  while IFS= read -r token; do
    [ -z "$token" ] && continue
    # A path may be written relative to the repository root, to the document, or
    # to a crate/app directory (for example `src/main.rs` under a crate). Accept
    # it if it resolves at any of those bases.
    if [ -e "$token" ] \
      || [ -e "$(dirname "$file")/$token" ] \
      || [ -n "$(find . \( -name node_modules -o -name target -o -name .git \) -prune \
        -o -path "*/$token" -print -quit)" ]; then
      continue
    fi
    fail "$file: referenced path '$token' does not exist"
  done <<< "$refs"
}

echo "==> Markdown links and referenced paths"
while IFS= read -r file; do
  [ -z "$file" ] && continue
  check_links "$file"
  check_paths "$file"
done <<< "$files"
pass "checked $(printf '%s\n' "$files" | grep -c . ) Markdown files"

echo "==> Script executability"
for script in scripts/*.sh; do
  if [ ! -x "$script" ]; then
    fail "$script is not executable"
  fi
done
pass "scripts/*.sh are executable"

echo "==> Example configuration"
cfg="examples/config.toml"
if [ ! -s "$cfg" ]; then
  fail "$cfg is missing or empty"
else
  grep -qE '^config_version[[:space:]]*=' "$cfg" || fail "$cfg: no config_version"
  grep -qE '^\[\[workspaces\]\]' "$cfg" || fail "$cfg: no [[workspaces]]"
  grep -qE '^\[\[services\]\]' "$cfg" || fail "$cfg: no [[services]]"
  grep -qE '^\[\[profiles\]\]' "$cfg" || fail "$cfg: no [[profiles]]"
  grep -qE '^\[\[groups\]\]' "$cfg" || fail "$cfg: no [[groups]]"
  pass "$cfg is structurally a versioned configuration"
fi

if [ "$with_config_test" -eq 1 ]; then
  echo "==> Example configuration (authoritative parse)"
  cargo test -q -p service-orchestrator-config \
    --test configuration loads_the_full_example_configuration
fi

echo
if [ "$failures" -ne 0 ]; then
  echo "check-docs: $failures problem(s) found"
  exit 1
fi
echo "check-docs: all checks passed"

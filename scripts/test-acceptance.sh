#!/usr/bin/env bash
#
# Task 030 acceptance suite (`tasks/030.md`). Drives the **real**
# `service-orchestrator` binary — a real daemon over the real Unix-domain-socket
# IPC and the real MCP stdio server — against isolated fixtures: local Git
# repositories with a local bare remote, a real Node.js HTTP fixture service,
# real health probes, and cross-client shared state.
#
# The suite is hermetic and fast and is the default. The heavier, toolchain- and
# network-dependent Maven/Liquibase acceptance runs are `#[ignore]`d crate tests
# and run only when `INCLUDE_REAL_TOOLCHAIN=1` is set. The performance/scale
# harnesses run only when `INCLUDE_PERFORMANCE=1` is set.
#
# Usage:
#   ./scripts/test-acceptance.sh
#   INCLUDE_REAL_TOOLCHAIN=1 ./scripts/test-acceptance.sh
#   INCLUDE_PERFORMANCE=1 ./scripts/test-acceptance.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

echo "==> Acceptance suite (real binary + real daemon over IPC)"
cargo test -p service-orchestrator --test acceptance -- --test-threads=1

if [[ "${INCLUDE_REAL_TOOLCHAIN:-0}" == "1" ]]; then
  echo "==> Real Maven library acceptance (--ignored)"
  cargo test -p service-orchestrator-build --test maven_library -- \
    --ignored --nocapture --test-threads=1

  # The Liquibase fixture has no Maven Wrapper and runs the provider's default
  # Maven goals, which would otherwise read the developer's own
  # `~/.m2/settings.xml` (often mirroring every repository) and download into the
  # real `~/.m2/repository`. Isolate it: a throwaway settings file with no
  # mirrors and an explicit local repository, injected through `MAVEN_ARGS`
  # (Maven 3.9+ prepends it to the command line). Nothing outside the temporary
  # directory is read or written.
  echo "==> Real Liquibase acceptance (--ignored, isolated Maven settings)"
  m2_tmp="$(mktemp -d "${TMPDIR:-/tmp}/so-accept-m2.XXXXXX")"
  m2_repo="$m2_tmp/repository"
  m2_settings="$m2_tmp/settings.xml"
  mkdir -p "$m2_repo"
  cat >"$m2_settings" <<EOF
<settings xmlns="http://maven.apache.org/SETTINGS/1.0.0">
  <localRepository>$m2_repo</localRepository>
</settings>
EOF
  (
    export MAVEN_ARGS="-s $m2_settings -Dmaven.repo.local=$m2_repo"
    cargo test -p service-orchestrator-liquibase --test liquibase_maven -- \
      --ignored --nocapture --test-threads=1
  )
  rm -rf "$m2_tmp"
fi

if [[ "${INCLUDE_PERFORMANCE:-0}" == "1" ]]; then
  echo "==> Opt-in performance/scale harnesses (--ignored)"
  cargo test --workspace -- --ignored --nocapture --test-threads=1
fi

echo "==> Acceptance suite complete"

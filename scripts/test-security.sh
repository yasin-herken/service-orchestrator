#!/usr/bin/env bash
#
# Security regression suite (`tasks/027.md` §35). It pins the invariants
# established by Task 026 by running the security-dense tests already present
# across the workspace, plus the new untrusted-input tests on the MCP surface.
#
# Every filter is verified to match at least one test: an empty match is treated
# as a failure, so a renamed or removed test cannot silently weaken the suite.
#
# Usage:
#   ./scripts/test-security.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# Runs a package's tests, optionally narrowed by a single libtest filter, and
# fails if no test matched.
run_filter() {
  local package="$1"
  local filter="${2:-}"
  local output
  echo "--> $package ${filter:+:: $filter}"
  if [[ -n "$filter" ]]; then
    output="$(cargo test -p "$package" -- "$filter" 2>&1)" || {
      echo "$output"
      return 1
    }
  else
    output="$(cargo test -p "$package" 2>&1)" || {
      echo "$output"
      return 1
    }
  fi
  echo "$output" | grep -E '^test result:' || true
  if ! echo "$output" | grep -qE '[1-9][0-9]* passed'; then
    echo "ERROR: no security tests matched for '$package' filter='$filter'" >&2
    return 1
  fi
}

echo "==> Security regression suite"

# Authorization and permission policy.
run_filter service-orchestrator-policy
run_filter service-orchestrator-application authoriz

# Untrusted input: identifiers, paths, branches, Git options.
run_filter service-orchestrator-domain redact
run_filter service-orchestrator-config shell
run_filter service-orchestrator-git refuses
run_filter service-orchestrator-git outside_the_workspace
run_filter service-orchestrator-mcp path_traversal
run_filter service-orchestrator-mcp branch_name_that_looks
run_filter service-orchestrator-mcp secret

# Migration target authorization and credentials.
run_filter service-orchestrator-liquibase authoriz

# Process ownership, spec validation, and environment redaction.
run_filter service-orchestrator-process reject
run_filter service-orchestrator-logging redact

# IPC authorization and framing bounds.
run_filter service-orchestrator-daemon destructive
run_filter service-orchestrator-daemon oversized

# Network and resource-limit hardening.
run_filter service-orchestrator-health url
run_filter service-orchestrator-client clamp
run_filter service-orchestrator-execution clamp

echo "==> Security regression suite complete"

# Task 027 progress — Automated Testing & Regression Suite

Status: **complete**. See `docs/testing.md` (reference) and
`docs/test-coverage-report.md` (inventory, category mapping, gaps).

## Environment

| Item | Value |
| --- | --- |
| Machine | Apple M2 Pro, 32 GiB RAM |
| OS | macOS (Darwin), Apple Silicon |
| Rust | stable per `rust-toolchain.toml` |
| Node / npm | per `apps/desktop/package.json` |
| Build mode | debug (`cargo test`, `clippy`) |

## Plan

1. Audit the existing unit/integration/frontend/desktop suites; build a
   coverage-gap inventory against `tasks/027.md` §7–§34.
2. Add deterministic regression tests only where a real gap exists; reuse the
   existing harnesses and fixtures.
3. Add simple, correct category scripts that fail on non-zero results.
4. Document the system (`docs/testing.md`) and the coverage
   (`docs/test-coverage-report.md`).
5. Run the full validation gate.

All steps done.

## Audit findings

The platform already carried a large, healthy suite (≈966 Rust tests plus 219
frontend and 131 desktop tests before this task). The audit found **no second
framework, no duplicate harness, and no flaky test**. Genuine gaps against the
task's category list were narrow and are now closed:

| Gap | Before | Now |
| --- | --- | --- |
| Scheduler stress (10/100/1000 tasks, dependency-heavy, cancellation storm) runnable by default | only `#[ignore]`d measurements | `crates/execution/tests/scheduler_stress.rs` (default) |
| MCP-layer untrusted input (path traversal, Git flag injection, log limits) | identifier validation only | `crates/mcp/src/tests.rs` security section |
| Process-group descendant reaping (child cleanup) | asserted only in a comment | `crates/process/tests/process_manager.rs` |
| IPC oversized-frame rejection over the real socket / request correlation | client-level unit test only | `crates/daemon/tests/daemon.rs` |
| One command per test category | none | `scripts/test-*.sh` |
| Test documentation | none | `docs/testing.md`, `docs/test-coverage-report.md` |

## Tests added

- `crates/execution/tests/scheduler_stress.rs` (new, 4 tests):
  `one_hundred_tasks_never_exceed_the_concurrency_limit_or_run_twice`,
  `a_dependency_heavy_graph_never_starts_a_task_early`,
  `a_cancellation_storm_settles_every_task_without_a_hang`,
  `one_thousand_independent_tasks_complete_without_deadlock`. All use fake
  executors and watch-channel gates; no sleeps, no external commands.
- `crates/mcp/src/tests.rs` (4 tests):
  `path_traversal_identifiers_are_rejected_before_dispatch`,
  `a_branch_name_that_looks_like_a_git_flag_is_rejected`,
  `an_enormous_log_limit_is_clamped_not_rejected`,
  `a_negative_limit_is_a_protocol_error`.
- `crates/process/tests/process_manager.rs` (1 test):
  `stopping_a_process_reaps_its_background_children` (POSIX group signalling,
  `#[cfg(unix)]`).
- `crates/daemon/tests/daemon.rs` (2 tests):
  `an_oversized_request_frame_is_rejected_without_crashing_the_daemon`,
  `concurrent_clients_receive_their_own_correlated_responses`.

## Confirmed defects fixed

**None.** The audit and the new tests found no product defect: MCP identifier
and branch validation, oversized-frame handling, and process-group reaping were
already correct. The new tests convert implicit guarantees into enforced
regression coverage. No production code was changed.

## Infrastructure added

- `scripts/test-unit.sh`, `test-integration.sh` (discovers each
  `crates/*/tests/*.rs` target by name), `test-security.sh` (fails on an empty
  filter match), `test-e2e.sh` (with `RUN_DESKTOP_TESTS` / `INCLUDE_PERFORMANCE`
  opt-ins), `test-frontend.sh`, `test-all.sh`. All `set -euo pipefail` and return
  non-zero on failure.
- README and `docs/architecture.md` link the new documentation.

## Final validation

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo check --workspace` | clean |
| `cargo test --workspace` | **977 passed / 0 failed / 17 ignored** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `./scripts/test-unit.sh` | 576 passed / 0 failed |
| `./scripts/test-integration.sh` | 26 targets / 0 failed |
| `./scripts/test-security.sh` | all filters non-empty / 0 failed |
| `RUN_DESKTOP_TESTS=1 ./scripts/test-e2e.sh` | desktop 131, mcp_stdio 1, daemon 25 (+1 ignored) / 0 failed |
| `./scripts/test-frontend.sh` | 24 files / 219 tests passed; typecheck clean |
| `npm run build` (`apps/desktop`) | success |

The 17 ignored tests are the opt-in toolchain and measurement harnesses listed
in `docs/test-coverage-report.md` §6.

Task 028 not started (per `tasks/027.md` §52).

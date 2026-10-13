# Test Coverage Report — Task 027

What the automated suite covers, where each category is tested, what remains
uncovered, and why. Measurements below were produced on the machine in §1 by
running the shipped scripts and `cargo test --workspace`.

See [`testing.md`](testing.md) for how to run each category.

## 1. Environment

| Item | Value |
| --- | --- |
| Machine | MacBook Pro, Apple M2 Pro, 32 GiB RAM |
| OS | macOS (Darwin, Apple Silicon) |
| Rust | stable per `rust-toolchain.toml` |
| Node / npm | per `apps/desktop/package.json` |
| Build mode | debug (`cargo test`) |
| Vitest | 5.0.4, `jsdom` |

## 2. Summary

| Suite | Result | Where |
| --- | ---: | --- |
| Rust workspace `cargo test --workspace` | **977 passed / 0 failed / 17 ignored** | `scripts/test-unit.sh`, `scripts/test-integration.sh` |
| — of which unit tests | 576 passed (+ 1 doctest) | `cargo test --workspace --lib` |
| — of which integration tests | 400 passed / 17 ignored | `crates/*/tests/*.rs` and `tests/mcp_stdio.rs` |
| Tauri desktop adapter (`apps/desktop/src-tauri`) | **131 passed / 0 failed** | `RUN_DESKTOP_TESTS=1 scripts/test-e2e.sh` |
| — desktop unit tests | 91 | typed command/adapter units |
| — desktop IPC integration | 40 | `tests/daemon_ipc.rs` |
| React frontend (Vitest) | **219 passed / 0 failed (24 files)** | `scripts/test-frontend.sh` |
| **Total active tests** | **1 327** | — |

Ignored tests are never counted as passes; they are the opt-in toolchain and
measurement harnesses listed in §6.

## 3. Inventory by crate

Unit = inline `#[cfg(test)]`; Integration = `tests/*.rs` (passed, with ignored
in parentheses).

| Crate | Unit | Integration |
| --- | ---: | --- |
| `service-orchestrator` (root) | 6 | `mcp_stdio` 1 |
| `domain` | 147 | `serialization` 6 |
| `config` | 7 | `configuration` 53, `management` 10 |
| `application` | 51 | `application_core` 82, `library_manager` 22, `workspace_validation` 14 (4 ignored) |
| `execution` | 38 (2 ignored) | `task_engine` 24, `scheduler_stress` 4, `performance` 0 (3 ignored) |
| `process` | 20 | `process_manager` 16 |
| `git` | 32 | `git_manager` 35, `task_engine` 9 |
| `build` | 58 | `providers` 12, `task_engine` 5, `manager` 5, `library` 4, `maven_library` 0 (4 ignored) |
| `liquibase` | 53 | `manager` 10, `task_engine` 6, `liquibase_maven` 0 (2 ignored) |
| `runtime` | 23 | `service_manager` 14, `task_engine` 2 |
| `health` | 40 | `health_manager` 14 |
| `logging` | 24 (1 ignored) | `observability` 10 |
| `daemon` | 4 | `daemon` 25 (1 ignored) |
| `client` | 24 | — |
| `mcp` | 34 | `tests/mcp_stdio.rs` (root) |
| `policy` | 5 | — |
| `tui` | 10 | `tui` 17 |
| doctests | 1 (`application`) | — |

## 4. Category coverage

The task's required categories (`tasks/027.md` §3) and where they are tested:

| # | Category | Primary coverage |
| --- | --- | --- |
| 1 | Domain unit tests | `domain` unit (ids, paths, task/workflow state, permissions, redaction) + `serialization` |
| 2 | Configuration | `config` `configuration` + `management` (valid/invalid TOML, versions, duplicates, references, ports, runtime/build/migration/health, path resolution) |
| 3 | Application core | `application` unit + `application_core` (queries, preconditions, policy, task/workflow submission, error mapping, scoping, idempotency) |
| 4 | Task engine | `execution` unit + `task_engine` (states, dependencies, concurrency, cancellation, timeout, retry, back-pressure, shutdown, events) |
| 5 | Scheduler stress | `execution` `scheduler_stress` (100 tasks, 1 000 tasks, dependency-heavy DAG, cancellation storm; no duplicates, limit respected, no deadlock) |
| 6 | Workflow | `application_core` (`prepare_environment`, build/start/stop ordering, profiles, groups, plan freshness) + `execution` workflow tests |
| 7 | Process manager | `process` unit + `process_manager` (spawn, stdout/stderr, exit, stop/kill, timeout, restart, cancellation, registry, many-concurrent, group reaping) |
| 8 | Git manager | `git` unit + `git_manager` (status/fetch/pull/checkout/clone/branches, dirty tree, detached HEAD, no upstream, diverged, conflict, unsafe refusal, workspace containment) |
| 9 | Build manager | `build` unit + `providers`/`manager`/`library` (provider selection, wrapper/executable detection, plan generation, cancellation, bulk ordering) |
| 10 | Library manager | `application` `library_manager` + `build` `library` (coordinates, ordering, verification, consumers, version alignment) |
| 11 | Migration manager | `liquibase` unit + `manager`/`task_engine` (provider selection, status, preview, target authorization, no auto-retry) |
| 12 | Service runtime | `runtime` unit + `service_manager` (runtime selection, start/stop/restart, duplicate-start, ownership, state reconciliation) |
| 13 | Health manager | `health` unit + `health_manager` (process/TCP/HTTP, actuator, readiness vs liveness, aggregation, timeout, polling cancellation) |
| 14 | Observability | `logging` unit + `observability` (correlation, streaming, historical, filtering, paging, retention, back-pressure, redaction) |
| 15 | IPC & daemon | `daemon` unit + `daemon` integration (startup, single instance, stale socket, shared state, authorization, framing) + `client` unit |
| 16 | TUI | `tui` unit + `tui` integration (navigation, selection, filtering, dialogs, task progress, errors, reconnection) |
| 17 | MCP protocol | `mcp` unit + root `mcp_stdio` end-to-end (handshake, discovery, schemas, structured responses/errors, tasks, resources, prompts) |
| 18 | Tauri adapter | desktop unit (91) + desktop `daemon_ipc` (40) |
| 19 | React frontend | 24 Vitest files (219 tests) across dashboard/services/git/builds/libraries/migrations/tasks/logs/health/workflows/profiles/configuration |
| 20 | Cross-interface | desktop `gui_tui_and_mcp_observe_the_same_authoritative_state`, `every_client_sees_the_same_authoritative_daemon`; daemon `two_clients_observe_the_same_workspace_and_service_state` |
| 21 | Security regression | `scripts/test-security.sh` — see §5 |
| 22 | Performance regression | `execution`/`workspace_validation`/`store`/`hub`/`daemon` `#[ignore]`d measurement harnesses — see §6 |

## 5. Security regression coverage

`scripts/test-security.sh` runs, and verifies non-empty matches for:

| Invariant | Tests |
| --- | --- |
| Operation classification / permission policy | `policy` (all) |
| Operation & migration authorization | `application :: authoriz`, `liquibase :: authoriz` |
| Secret redaction | `domain :: redact`, `logging :: redact` |
| Shell-command rejection in config | `config :: shell` |
| Git unsafe-operation refusal / containment | `git :: refuses`, `git :: outside_the_workspace` |
| MCP path traversal (ids) | `mcp :: path_traversal` |
| MCP Git flag injection (branches) | `mcp :: branch_name_that_looks` |
| MCP secret redaction | `mcp :: secret` |
| Process spec validation | `process :: reject` |
| IPC destructive authorization | `daemon :: destructive` |
| IPC framing bound | `daemon :: oversized` |
| HTTP URL hardening | `health :: url` |
| Resource-limit clamping | `client :: clamp`, `execution :: clamp` |

## 6. Ignored tests

Skipped by default, each for a concrete reason. Run with `--ignored`.

| File | Count | Reason |
| --- | ---: | --- |
| `execution/tests/performance.rs` | 3 | scale/measurement harnesses |
| `execution/src/store.rs` | 2 | store micro-cost measurements |
| `application/tests/workspace_validation.rs` | 4 | 1 scale check + 3 measurement harnesses |
| `daemon/tests/daemon.rs` | 1 | IPC latency measurement |
| `logging/src/hub.rs` | 1 | log paging measurement |
| `build/tests/maven_library.rs` | 4 | requires a real Maven installation |
| `liquibase/tests/liquibase_maven.rs` | 2 | requires Maven, a JVM, and network |

## 7. What Task 027 added

| Change | File |
| --- | --- |
| Deterministic scheduler stress suite (4 tests) | `crates/execution/tests/scheduler_stress.rs` (new) |
| MCP untrusted-input security tests (4 tests) | `crates/mcp/src/tests.rs` |
| Process-group descendant reaping test (1 test) | `crates/process/tests/process_manager.rs` |
| IPC framing-bound and request-correlation tests (2 tests) | `crates/daemon/tests/daemon.rs` |
| Category scripts | `scripts/test-{unit,integration,security,e2e,frontend,all}.sh` (new) |
| Documentation | `docs/testing.md`, `docs/test-coverage-report.md` (new) |

All new tests pass against the existing code. The audit found **no new product
defects**: every behaviour the new tests pin (MCP identifier/branch rejection,
oversized-frame handling, process-group reaping) was already correct. The tests
convert previously implicit guarantees into enforced regression coverage.

## 8. Coverage gaps and known limitations

These are deliberate and honest; none is reported as covered.

1. **Real Maven / JVM / network builds** are exercised only by the `#[ignore]`d
   fixtures, because requiring a toolchain and network on every run would make
   the default suite unreliable and slow. Their planning, provider selection,
   and error handling are covered with fake process managers.
2. **Real database migrations** are not run by the default suite. Migration
   authorization, planning, target classification, no-auto-retry, and status
   are covered without a database. No company database is ever contacted.
3. **Scheduler scale is bounded.** The default suite stresses 100 and 1 000
   tasks (plus a dependency-heavy DAG and a 200-task cancellation storm); the
   multi-thousand-task and long-soak measurements are opt-in.
4. **The Tauri adapter requires an opt-in build.** Its unit and IPC tests pass
   (131), but building the Tauri toolchain is slow, so `RUN_DESKTOP_TESTS=1` is
   required. A headless render of the real webview is out of scope; the adapter
   is covered at the IPC boundary and the UI at the component level.
5. **macOS-specific process behaviour** is POSIX and gated with `#[cfg(unix)]`;
   it does not run on non-Unix hosts (this project targets macOS).
6. **No coverage percentage threshold is enforced.** There is no arbitrary
   target; coverage is judged by category and failure mode. Line-coverage
   tooling (`cargo-llvm-cov`, `tarpaulin`) was not added because it is not
   installed and would be a new, unrequested dependency.
7. **Pre-existing reliability debt** unrelated to testing remains as recorded
   in [`reliability-report.md`](reliability-report.md) §7 and
   [`security-audit.md`](security-audit.md) (e.g. bounded git stderr context);
   it is not a regression and is out of scope here.

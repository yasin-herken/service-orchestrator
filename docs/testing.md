# Testing — Automated Testing & Regression Suite (Task 027)

This document describes how Service Orchestrator is tested: the layers, the
categories, how to run each category locally, the fixture and isolation
strategy, and the known limitations. It is the companion to
[`test-coverage-report.md`](test-coverage-report.md), which records what is
covered and what is not.

Everything here runs **locally**. The suite never requires GitHub Actions,
Jenkins, a cloud service, a production credential, a company repository, or a
company database.

## 1. Goals

- Cover the existing architecture (`AGENTS.md` §5) with behaviour-focused,
  deterministic tests.
- Keep a fast default suite that a developer can run on every change, with
  slower end-to-end and measurement suites explicitly separated.
- Never touch real developer data: repositories, Maven repositories, databases,
  sockets, or processes.
- Provide one obvious command per category.

## 2. Test layers

```text
Unit tests            inline #[cfg(test)] modules in each crate
    │
    ▼
Component tests       fake infrastructure driving one crate's public API
    │
    ▼
Integration tests     crates/*/tests/*.rs — real ports, real transports
    │
    ▼
Cross-interface       GUI / TUI / MCP over one authoritative daemon
    │
    ▼
End-to-end            real binaries and the real Unix-socket daemon
```

Not every test launches the whole application. Unit and component tests use
fake executors and ports; only the integration, cross-interface, and end-to-end
layers wire real infrastructure (a real daemon, real Git fixture repositories,
real child processes).

## 3. Categories and local commands

| Category | Command | Scope |
| --- | --- | --- |
| Unit | `./scripts/test-unit.sh` | Every library crate's inline unit tests (`cargo test --workspace --lib`). Fastest gate. |
| Integration | `./scripts/test-integration.sh` | Every `crates/*/tests/*.rs` target, discovered and run by name. |
| Security | `./scripts/test-security.sh` | Security-dense suites and untrusted-input tests (see §6). |
| Frontend | `./scripts/test-frontend.sh` | TypeScript type check + Vitest. |
| End-to-end | `./scripts/test-e2e.sh` | Real MCP stdio binary, real daemon IPC, and (opt-in) the Tauri desktop adapter. |
| Acceptance | `./scripts/test-acceptance.sh` | The real `service-orchestrator` binary end to end: real daemon over the real Unix-socket IPC, real MCP stdio, real local Git fixtures, a real Node.js service, and real HTTP health checks (`tasks/030.md`). |
| All | `./scripts/test-all.sh` | fmt, check, unit, integration, security, clippy, frontend tests, frontend build. |

Extra switches:

- `RUN_DESKTOP_TESTS=1 ./scripts/test-e2e.sh` — also runs the Tauri desktop
  adapter unit tests and the desktop↔daemon IPC integration tests. This builds
  the separate Tauri toolchain and is the slowest step, so it is opt-in.
- `INCLUDE_PERFORMANCE=1 ./scripts/test-e2e.sh` — also runs the `#[ignore]`d
  scale/measurement harnesses with `--ignored`.
- `INCLUDE_REAL_TOOLCHAIN=1 ./scripts/test-acceptance.sh` — also runs the
  `#[ignore]`d real Maven and real Liquibase acceptance tests. The Liquibase
  branch is wrapped in a throwaway Maven `settings.xml` (no mirrors) and an
  isolated local repository, so the developer's `~/.m2` is never read or written.
- `INCLUDE_PERFORMANCE=1 ./scripts/test-acceptance.sh` — also runs the
  scale/measurement harnesses.

Every script uses `set -euo pipefail` and returns a **non-zero exit code** when
any test fails. The security script additionally fails if a filter matches **no**
test, so a renamed test cannot silently weaken the suite.

The raw commands are unchanged and always valid:

```bash
cargo test --workspace --lib
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
```

## 4. Layout

- **Inline unit tests** live beside the code in a `#[cfg(test)] mod tests` in the
  same file, so they can reach private items such as state machines and command
  builders.
- **Integration tests** live in `crates/<crate>/tests/`. A small
  `tests/support/mod.rs` module is shared where a fixture is reused.
- **Cross-crate end-to-end tests** live at the repository root
  (`tests/mcp_stdio.rs`) and in the daemon and desktop crates.
- **The Task 030 acceptance suite** lives at the repository root
  (`tests/acceptance.rs`). It drives the **real `service-orchestrator` binary**:
  a real daemon over the real Unix-socket IPC, a real MCP stdio server, real
  local Git repositories with a real bare remote, a real Node.js HTTP fixture
  service, and real HTTP health probes. Each scenario owns a private temp root,
  a private socket (pinned through `SERVICE_ORCHESTRATOR_SOCKET`), and its own
  daemon, which is stopped and removed on drop.
- **The Tauri desktop adapter** is its own workspace
  (`apps/desktop/src-tauri`); its unit tests are inline and its IPC integration
  tests live in `apps/desktop/src-tauri/tests/daemon_ipc.rs`.
- **Frontend tests** live beside the source as `src/**/*.test.{ts,tsx}` and run
  under Vitest with `jsdom` and a mocked desktop API.

## 5. Fixtures and isolation

Every layer is isolated by construction (`tasks/027.md` §37, §39):

| Concern | How it is isolated |
| --- | --- |
| Configuration | Parsed from in-memory strings or a fixture `config.toml` in a throwaway temp directory. Real `~/.service-orchestrator` is never read or written. |
| Git | Temporary local repositories and local bare remotes created and torn down by the test. No network, no GitHub, no developer checkout. |
| Maven / npm | Fake process managers for unit tests; the `#[ignore]`d Maven/JVM integration tests are the only ones that touch a real toolchain. Every real Maven build pins its own `settings.xml` and `-Dmaven.repo.local`, and the acceptance script isolates the Liquibase run the same way, so the developer's real `~/.m2` and npm caches are never read or written by any suite. |
| Databases / migrations | Target authorization and planning are tested without a database; the only real-toolchain migration tests are `#[ignore]`d and require an explicit opt-in. |
| Processes | Small, standard helper executables (`/bin/sh`, `sleep`, `printf`) spawned into their own process group and always stopped or killed. |
| Daemon / IPC | A private Unix-domain socket under a per-test temp directory, pinned through `SERVICE_ORCHESTRATOR_SOCKET`; the socket and lock file are removed on drop. |
| Logs | In-memory log hub. |
| Filesystem | `std::env::temp_dir()` subtrees created per test and removed on drop. |

Tests that need the process-global `SERVICE_ORCHESTRATOR_SOCKET` override hold a
process-wide lock for their lifetime and restore the previous value on drop, so
they run one at a time within their binary; other binaries run in separate
processes and are unaffected.

## 6. Security regression suite

`./scripts/test-security.sh` pins the invariants established by Task 026
(see [`security.md`](security.md) and [`security-audit.md`](security-audit.md)).
It runs the security-dense packages and targeted filters, including:

- policy classification (`service-orchestrator-policy`);
- operation authorization and migration-target authorization
  (`service-orchestrator-application`, `service-orchestrator-liquibase`);
- secret redaction (`service-orchestrator-domain`,
  `service-orchestrator-logging`);
- config shell-command rejection (`service-orchestrator-config`);
- Git unsafe-operation refusal and workspace containment
  (`service-orchestrator-git`);
- MCP untrusted-input rejection: path traversal, Git flag injection, oversized
  log requests (`service-orchestrator-mcp`);
- IPC destructive authorization and framing bounds
  (`service-orchestrator-daemon`);
- process spec validation (`service-orchestrator-process`);
- HTTP health URL hardening (`service-orchestrator-health`);
- resource-limit clamping (`service-orchestrator-client`,
  `service-orchestrator-execution`).

## 7. Determinism

The suite avoids arbitrary sleeps (`tasks/027.md` §38). Coordination uses:

- `tokio` synchronization primitives (`watch`, `broadcast`, `Notify`) so waiting
  is race-free rather than poll-and-sleep;
- explicit readiness signals (a log line, a gate snapshot, a registered event
  pump) instead of fixed delays;
- bounded deadlines so a regression fails fast instead of hanging;
- local fixture repositories and in-process fake executors.

A small number of OS-level races are unavoidable and are documented at the test
(for example, the moment a shell installs a signal trap). Where they occur, the
test is written to be deterministic rather than merely tolerant, and the reason
is recorded in a comment.

## 8. Environment requirements

| Tool | Used by |
| --- | --- |
| Rust (stable, per `rust-toolchain.toml`) | all Rust crates |
| `git` | the Git integration tests (local fixtures only) |
| `sh`, `sleep`, `printf` | the process-manager tests |
| Node.js + npm (per `package.json`) | the frontend tests and build |
| Tauri toolchain | only `RUN_DESKTOP_TESTS=1` |
| Maven + JVM + network | only the `#[ignore]`d Maven/Liquibase tests |

No other external service is required.

## 9. Ignored tests

The default suite skips, via `#[ignore]`, only tests that need a real toolchain
or that are scale/measurement harnesses. They are never counted as passes and
are listed with their reason in
[`test-coverage-report.md`](test-coverage-report.md#ignored-tests). Run them
explicitly with `--ignored`.

## 10. Adding a test

1. Put the test in the lowest layer that can exercise the behaviour: unit for a
   pure rule, component for one crate's public API, integration when a real port
   or transport is required.
2. Use the existing fixtures; do not add a second framework for the same
   language.
3. Prefer a fake port or a temp directory over a real toolchain.
4. Assert behaviour and failure modes, not implementation details.
5. Run `cargo fmt` and the focused test target, then `./scripts/test-all.sh`.

## 11. Related documents

- [`test-coverage-report.md`](test-coverage-report.md) — inventory, category
  mapping, and remaining gaps.
- [`security.md`](security.md), [`security-audit.md`](security-audit.md) — the
  security invariants the security suite pins.
- [`performance-report.md`](performance-report.md) — the measurement harnesses.
- [`reliability-report.md`](reliability-report.md) — the reliability findings
  the regression tests lock down.
- [`acceptance-test-matrix.md`](acceptance-test-matrix.md),
  [`final-acceptance-report.md`](final-acceptance-report.md) — the Task 030
  acceptance results.
- [`architecture.md`](architecture.md) — the layers these tests exercise.

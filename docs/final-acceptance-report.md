# Final Acceptance Report — Task 030

Final integration and acceptance testing of Service Orchestrator, performed
against the real shipped binaries with isolated fixtures. This report records
what was executed and what actually happened. It is the companion to
[`acceptance-test-matrix.md`](acceptance-test-matrix.md) (the per-scenario
record) and [`release-blockers.md`](release-blockers.md) (unresolved blockers).

**Result: no Critical or High defect was found; the application passes its core
workflows, daemon lifecycle, cross-interface consistency, security, and
performance acceptance. It is release-ready at the engineering level. macOS
installer packaging (DMG, signing, notarization) is explicitly out of scope for
this task and was not produced.**

---

## 1. Test environment

| Item | Value |
| --- | --- |
| Machine | Apple Silicon (`arm64`), macOS 26.6.2 (Darwin) |
| rustc / cargo | 1.98.1 stable (`rust-toolchain.toml`) |
| Node / npm | v22.18.0 / 11.11.1 |
| Java / Maven | OpenJDK 25.0.4.1 / Apache Maven 3.9.16 |
| Build profile | debug for correctness, `--release` for the Task 025 measurements (unchanged, reused) |

### Isolation

Every scenario runs in throwaway fixtures. No real company repository, database,
credential, developer service, or the developer's real configuration /
`~/.m2/repository` is touched:

- **Acceptance suite** (`tests/acceptance.rs`) — a private temporary root per
  scenario: local bare Git remotes, seed clones, working checkouts, a real
  Node.js HTTP fixture, a private Unix socket pinned through
  `SERVICE_ORCHESTRATOR_SOCKET`, and a captured daemon log. Each scenario owns
  and tears down its own daemon.
- **Real Maven/Liquibase** — an isolated Maven `settings.xml` (no mirrors) and an
  isolated local repository, injected for the Liquibase run through
  `MAVEN_ARGS` in `scripts/test-acceptance.sh`; the Maven library fixture pins
  its own settings. The developer's `~/.m2` is neither read nor written.
- **Daemon / desktop / MCP** — per-test temp sockets and lock files, restored on
  drop.

### How acceptance was run

```bash
./scripts/test-acceptance.sh                                  # hermetic, ~19 s, 9/9
INCLUDE_REAL_TOOLCHAIN=1 ./scripts/test-acceptance.sh         # + real Maven 4/4, Liquibase 2/2
cargo test --workspace                                        # 986 passed / 0 failed / 17 ignored
./scripts/test-security.sh                                    # passed
RUN_DESKTOP_TESTS=1 ./scripts/test-e2e.sh                     # desktop 40 passed, MCP stdio, daemon 25 passed
(cd apps/desktop && npm run typecheck && npm run test && npm run build)   # 231 tests, build OK
```

## 2. Acceptance matrix summary

Full detail, including each scenario's steps, expected/actual result, evidence,
and defects, is in [`acceptance-test-matrix.md`](acceptance-test-matrix.md).

| Status | Count |
| --- | --- |
| Passed | 53 |
| Failed | 0 |
| Blocked | 0 |
| Skipped | 0 |
| Manual (not automatable here; not claimed as pass) | 2 |

The two manual items are the **live TUI visual/interaction pass** (AC-35) and the
**live Tauri window pass** (AC-44). Their data and command layers are automated
(AC-02/AC-33/AC-34 for the TUI; AC-40…AC-43 for the desktop); only a human
visual pass on a real interactive session is outstanding, and it is recorded as
`MANUAL`, **not** as a pass.

### What the acceptance suite actually drives

`tests/acceptance.rs` starts the **real `service-orchestrator` binary**, so the
whole stack is exercised end to end: real CLI argument handling, real
configuration loading, real daemon, real Unix-socket IPC framing, the real task
engine and scheduler, the real Git CLI against local bare remotes, the real
process manager against a real Node.js service, real HTTP health probes, and the
real MCP stdio server. The nine scenarios are listed in the passed-scenarios
section below.

## 3. Passed scenarios

Grouped by the acceptance areas from `tasks/030.md`; each maps to one or more
matrix rows.

1. **Startup** — binary/daemon, TUI (rendered through Ratatui `TestBackend`), MCP
   stdio, and the desktop frontend build + adapter; invalid and missing
   configuration rejected with actionable, field-specific errors.
2. **Daemon lifecycle** — single-instance enforcement, existing-daemon detection,
   stale socket/lock recovery, client connect/disconnect, graceful `daemon stop`
   with socket release, and survival of an interface exiting.
3. **Shared authoritative state** — two clients of one daemon observe identical
   runtime state; a queued task cancelled by one interface settles as `cancelled`
   and is observed by another; the desktop adapter shares the same real daemon.
4. **Workspace loading/validation** — valid load, missing-repository diagnostics,
   unknown-workspace structured error, and 46 config-validation cases (duplicates,
   path escape, unknown deps, invalid groups/profiles, unicode/spaces).
5. **Git** — real status/branches/fetch/pull/checkout against a local bare remote;
   bulk sync that refuses a dirty repository while the clean one succeeds and the
   local change is preserved; clone, diverged-branch refusal, missing upstream,
   timeout, cancellation, and workspace containment.
6. **Maven/npm/library** — real Maven single-module and multi-module install with
   artifact verification; npm provider planning/execution and runtime validation;
   library discovery, ordering, and read-only version alignment.
7. **Migrations** — real Liquibase validate/apply/status/preview against in-memory
   H2; target authorization and safety invariants.
8. **Service runtime and health** — a real Node service starts, reports a pid, is
   `Healthy` over HTTP, ignores a duplicate start, and stops; an unexpected
   `kill -9` is observed as a terminal state without signalling an unrelated
   process; HTTP/TCP/readiness/timeout/transition health cases.
9. **Logs** — live delivery, bounded/filterable history, pagination, per-service
   and total retention caps, secret redaction, and a slow subscriber that lags
   without blocking the producer.
10. **Workflows** — prepare-environment orchestration, dependency ordering,
    failure propagation, blocked dependents, partial success, and cancellation.
11. **TUI** — navigation, listing, filtering, id-typed multi-selection, actions
    dispatched as tasks, error dialogs, destructive confirmation, and terminal
    restoration.
12. **MCP** — initialize, tool/resource/prompt discovery, structured results and
    errors, untrusted-input rejection, authorization and redaction, clean
    shutdown, and stdout carrying only JSON-RPC.
13. **Desktop GUI** — adapter reads/validation/errors over the real IPC protocol,
    service lifecycle, Git, builds, libraries, cancellation, per-item partial
    failure, a stopped-daemon structured error, and all page component suites.
14. **Security** — the full Task 026 regression suite, including non-forgeable
    destructive authorization and input hardening.
15. **Performance** — startup, IPC, read paths at scale, task-engine throughput,
    and configuration loading, all within their documented budgets.
16. **Reliability and recovery** — scheduler stress, stale-socket recovery, an
    unexpected process exit, invalid configuration, IPC interruption, and
    subscription/teardown cleanup.
17. **Cross-interface consistency** — stable ids and consistent runtime, health,
    progress, and cancellation state across interfaces.
18. **Error quality** — structured errors carrying operation, resource, category,
    safe context, and next action, without leaking credentials.

## 4. Failed scenarios

None. No acceptance scenario failed in any run.

## 5. Blocked scenarios

None. No scenario was blocked by an external dependency.

## 6. Skipped scenarios

None were skipped. Two scenarios are recorded as `MANUAL` (not passes) because
they require an interactive desktop/terminal session that is not available to an
automated run: the live TUI visual pass (AC-35) and the live Tauri window pass
(AC-44). Their automatable layers are covered.

## 7. Confirmed defects

### DEF-030-1 — Non-hermetic real-Liquibase acceptance run *(test harness only; Medium)*

- **Symptom.** `cargo test -p service-orchestrator-liquibase --test liquibase_maven -- --ignored`
  failed on first run with `Failed to read artifact descriptor for
  org.liquibase:liquibase-core:jar:4.27.0` — Maven tried to resolve through the
  developer's personal mirror (`https://nexus.bilgem.gov.tr/nexus/...`), which
  was unreachable.
- **Root cause.** The Liquibase fixture has no Maven Wrapper and runs the
  provider's default Maven goals, so Maven read the developer's real
  `~/.m2/settings.xml` and would also download into the real
  `~/.m2/repository`. This violates the Task 030 isolation rule ("do not modify
  the developer's real configuration or Maven repository"), but it is a
  **test-harness** defect, not a product defect: the product's Maven/orchestration
  path is unchanged and correct.
- **Impact.** An opt-in, `#[ignore]`d acceptance run could fail for an
  environmental reason unrelated to the product, and could write to the
  developer's Maven repository. It never affected the default hermetic suite.
- **Status.** **Fixed** (see §8). Re-run green: 2/2.

No product defect (Critical, High, Medium, or Low) was discovered in the shipped
application code during Task 030.

## 8. Fixes implemented

### FIX-030-1 — Isolate the real-toolchain Liquibase acceptance run

`scripts/test-acceptance.sh` now, for the `INCLUDE_REAL_TOOLCHAIN=1` Liquibase
branch only, creates a throwaway directory containing:

- a `settings.xml` with no mirrors and an explicit `<localRepository>`, and
- an empty isolated local repository,

and runs the test with `MAVEN_ARGS="-s <settings> -Dmaven.repo.local=<repo>"`
(Maven 3.9+ prepends `MAVEN_ARGS` to the command line). The directory is removed
when the run completes. The developer's `~/.m2` is now neither read nor written,
and the run is green regardless of any personal mirror. The real Maven library
tests already pinned their own settings and local repository and were left
unchanged.

**Regression evidence:** `INCLUDE_REAL_TOOLCHAIN=1 ./scripts/test-acceptance.sh`
→ real Maven 4/4 and real Liquibase 2/2 passed.

No product source file was modified for Task 030.

## 9. Remaining limitations

These are verified, documented, deliberate/deferred limitations (see
[`known-limitations.md`](known-limitations.md)); none is a Task 030 defect:

- **Task history is retained in memory without eviction.** Listing is bounded;
  retention is future work. This is the largest long-lived memory risk
  ([`performance-report.md`](performance-report.md) §Remaining).
- **Runtime/task/workflow/log state is in memory** and lost on daemon restart; a
  fresh daemon adopts nothing and never signals a process by PID or port
  (safe by design — verified by AC-27).
- **No managed-process recovery after a crash**; **one daemon per config
  directory**; **best-effort event delivery** (lagging subscribers resync).
- **No OS-level supervision** (the auto-started daemon is a detached child, not a
  `launchd` service).
- **Cancelling a migration does not guarantee a rollback**; Liquibase locks are
  never released automatically.
- **Gradle and Flyway are not implemented**; **Docker/Kubernetes are out of
  scope**.
- **Packaging:** `tauri.conf.json` sets `bundle.active = false`; there is **no DMG,
  code signing, notarization, or CI/CD** (explicitly excluded by `tasks/030.md`
  §39).
- **Same-user trust only**; redaction is best-effort; HTTP health URLs are not
  allow-listed ([`security.md`](security.md)).
- **Live TUI/GUI visual passes are manual** (AC-35, AC-44).

## 10. Performance findings

Reused from Task 025 ([`performance-report.md`](performance-report.md)) and
re-verified by the same `#[ignore]`d harnesses during Task 030. The Task 030
re-runs used the **default (debug) profile** — a stricter check than the release
figures in the Task 025 report — and **every budget still held**:

| Metric | Budget | Task 025 release | Task 030 re-check (debug) |
| --- | --- | --- | --- |
| Daemon startup | < 100 ms | ≈1.5 ms median | **1.50 ms median** (5 launches, min 1.37 / max 1.66 ms) |
| IPC `list_services` p95 | < 5 ms | ≈137 µs | **≈380 µs** (≈330 µs median, real socket) |
| `list_services` @ 200 services | < 5 ms | ≈94 µs | **322 µs** |
| `validate_workspace` @ 200 services | < 5 ms | ≈373 µs | **2.51 ms** |
| Configuration load @ 200 services | startup-only | ≈3.6 ms | **38.7 ms** (debug; one-time startup cost) |
| Task submit (20 000) | < 50 µs/task | ≈5.0 µs/task | **15.55 µs/task** (311 ms total) |
| Task drain (steady state) | < 100 µs/task, flat | ≈7.4 µs/task | **20.6–22.2 µs/task, flat** to 4 000 history |
| Deep workflow (100/500/1 000 steps) | linear | linear | **linear** (plan 1.5/6.7/15.1 ms) |
| Idle daemon CPU / RSS | < 1 % / < 16 MiB | ≈0 % / ≈6.1 MiB | unchanged (no code change) |

Every debug figure is within its budget, and the debug numbers are the *worse*
case (release is roughly 7–15× faster — `performance-report.md` §Debug-profile
reference). No performance regression was introduced by Task 030 (no product
code changed).

## 11. Security findings

`./scripts/test-security.sh` passed in full, re-running the Task 026 invariants
([`security.md`](security.md), [`security-audit.md`](security-audit.md)): IPC
peer-UID-verified destructive authorization (non-forgeable), policy
classification, secret redaction, config shell-command rejection, Git
unsafe-operation refusal and workspace containment, MCP untrusted-input
rejection (path traversal, flag injection, oversized requests), IPC framing
bounds, process-spec validation, HTTP health URL hardening, and resource-limit
clamping. **No security policy was weakened to make any acceptance test pass**,
and no new security defect was found.

## 12. Packaging readiness decision

**Ready for macOS application packaging.**

The release-readiness criteria in `tasks/030.md` §33 are met:

- ✅ No unresolved Critical defects. ✅ No unresolved High defects.
- ✅ Core workflows pass (acceptance + application/execution suites).
- ✅ Daemon lifecycle is reliable (single-instance, stale recovery, graceful stop).
- ✅ Cross-interface state is consistent (one authoritative daemon).
- ✅ Security regression tests pass.
- ✅ GUI is usable and its adapter runs against a real daemon; page suites and the
  production build pass.
- ✅ TUI and MCP remain functional.
- ✅ Performance is acceptable for the 40-service (and 100+/200-service) target.
- ✅ Known limitations are documented.

**Caveats, stated precisely:**

1. The **installer artifact** (DMG) and **code signing/notarization** are **not
   produced** — they are explicitly out of scope for Task 030 (`tasks/030.md`
   §39). "Packaging readiness" here means the application is verified and
   releasable at the engineering level; producing a signed/notarized DMG is a
   separate, follow-on task.
2. The two **manual** scenarios (live TUI and live Tauri window visual/
   interaction passes) should be performed by a human on a real session before
   distribution. Their logic is automated; only the visual/interaction pass is
   outstanding.
3. The **test-harness** isolation defect (DEF-030-1) is fixed; no product defect
   remains open.

No Task 031 work was started.

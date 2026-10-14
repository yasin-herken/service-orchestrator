# Release Blockers — Task 030

This page lists issues that block release of Service Orchestrator to users. It
was produced at the close of Task 030 (Final Integration & Acceptance Testing)
from the evidence in [`acceptance-test-matrix.md`](acceptance-test-matrix.md) and
[`final-acceptance-report.md`](final-acceptance-report.md).

## Unresolved release blockers

**None.**

### Evidence

- **No Critical defects.** No data loss, no unauthorized database mutation, no
  signalling of unrelated processes, no credential exposure, and no duplicate
  authoritative daemon was observed. The safety-critical invariants were
  exercised and hold:
  - an unrelated `sleep` process is never signalled while the daemon manages and
    loses a real service (`tests/acceptance.rs::unexpected_service_exit_is_reported`);
  - a fresh daemon adopts nothing and never kills by PID or port
    (`crates/daemon/tests/daemon.rs::a_fresh_daemon_adopts_nothing_and_never_kills_unrelated_processes`);
  - a second daemon on the same socket refuses to become authoritative
    (`tests/acceptance.rs::daemon_lifecycle_status_single_instance_and_stop`);
  - destructive authorization is non-forgeable (peer-UID verified; denied for MCP
    and for non-same-UID peers) — `crates/daemon/tests/daemon.rs`, `./scripts/test-security.sh`.
- **No High defects.** No broken workflow, no incorrect task state, no process
  leak, no deadlock, and no incorrect dependency ordering was found. The
  scheduler-stress, task-engine, workflow, and daemon suites all pass; the
  acceptance daemon is torn down cleanly in every scenario.
- **No Medium defects in product code.** The only Medium finding (DEF-030-1) was
  in the **test harness** and is fixed (§Resolved below).
- **All acceptance scenarios passed** (53/53 automatable + 2 manual); the full
  suite is green: `cargo test --workspace` → 986 passed / 0 failed / 17 ignored;
  `./scripts/test-security.sh` → passed; desktop end-to-end → 40 passed;
  frontend → 231 tests + production build pass.

## Resolved during Task 030

### FIX-030-1 — Real-Liquibase acceptance run was non-hermetic *(test harness; Medium)*

| Field | Detail |
| --- | --- |
| Severity | Medium (test hygiene; could fail for environmental reasons and write to the developer's Maven repository) |
| Description | The `#[ignore]`d real-Liquibase acceptance test ran the provider's default Maven goals with no isolated settings, so Maven read the developer's `~/.m2/settings.xml` (which mirrored to an unreachable Nexus) and would download into the real `~/.m2/repository`. |
| Impact | The opt-in real-toolchain run could fail for a reason unrelated to the product; it could also modify developer-owned Maven state. It never affected the default hermetic suite. |
| Reproduction (before fix) | `cargo test -p service-orchestrator-liquibase --test liquibase_maven -- --ignored` on a machine whose `~/.m2/settings.xml` mirrors every repository → `Failed to read artifact descriptor for org.liquibase:liquibase-core:jar:4.27.0`. |
| Recommended resolution | Run the Liquibase branch with a throwaway `settings.xml` (no mirrors) and an isolated local repository injected via `MAVEN_ARGS`. |
| Current status | **Fixed** in `scripts/test-acceptance.sh`. Re-run: real Maven 4/4 and real Liquibase 2/2 pass with `INCLUDE_REAL_TOOLCHAIN=1`. |

No product source file was changed for Task 030.

## Non-blocking notes (not blockers)

These are recorded for completeness. None blocks packaging.

1. **Two manual acceptance passes.** The live TUI visual/interaction pass
   (AC-35) and the live Tauri window pass (AC-44) are `MANUAL`: their logic is
   automated, but a human visual pass on an interactive session is recommended
   before distribution. This is not a defect.
2. **Installer packaging is out of scope.** DMG creation, code signing, and
   notarization are explicitly excluded by `tasks/030.md` §39. The application is
   release-ready at the engineering level; producing a signed/notarized DMG is a
   follow-on task.
3. **Documented design limitations** (`docs/known-limitations.md`): in-memory
   state lost on daemon restart, no managed-process recovery, no OS-level
   supervision, task-history retention deferred, Gradle/Flyway/Docker/Kubernetes
   out of scope, same-user trust model, best-effort redaction. These are
   deliberate and documented, not blockers.
4. **Largest long-lived memory risk** — terminal-task history is retained for the
   process lifetime (`docs/performance-report.md`). Recommended as a follow-up
   retention policy, not a blocker at the target workload.

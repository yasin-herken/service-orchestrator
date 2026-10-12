# Reliability Report — Task 017

The reliability audit of the Service Orchestrator workspace: what exists, what
was found, what was fixed, how the fixes are pinned by regression tests, and
what risk remains. Findings are listed by severity; every fix below is in the
working tree accompanying this report and every gate is green
(`cargo fmt --check`, `cargo check --workspace`, `cargo test --workspace`,
`cargo clippy --all-targets --all-features -- -D warnings`).

## 1. Capability inventory (what was audited)

| Capability | Implementation | Audit outcome |
| --- | --- | --- |
| Git | `crates/git` — GitManager port, CLI-backed runner, per-repo coordinator, task-engine integration | See D5, plus new reliability fixtures (§4) |
| Build | `crates/build` — provider abstraction (Maven/npm), build plans, artifact inspection | See D5 |
| Libraries | `crates/library` — coordinates, install, version alignment | No defects found |
| Service runtime | `crates/runtime` — service manager, registry, start plans, port/ownership rules | See D1, D10 |
| Migrations | `crates/liquibase` — provider-based execution, database-target safety | See D5 |
| Task engine | `crates/execution` — scheduler, store, state machine, cancellation | See D2, D6, D9, D11 |
| Workflows | `crates/application` — prepare/start/stop environment orchestration | No defects found |
| Process management | `crates/process` — supervision, ports, logs, signals | See D10 |
| Health & observability | `crates/health`, `crates/logging` | No defects found |
| Daemon & IPC | `crates/daemon`, `crates/client` — Unix socket, protocol, authorization | See D7, H-IPC-1, H-IPC-3, G3 |
| Workspace validation | `crates/application` + `src/probe.rs` (new in Task 017) | Read-only by construction; see §5 |

## 2. High-severity findings and fixes

### H-IPC-1 — Destructive authorization was self-asserted *(High)*

**Finding.** The IPC request envelope carried `allow_destructive` as a
client-supplied boolean. Any local process that could connect to the socket
could assert it, bypassing the policy engine for destructive operations —
a violation of `AGENTS.md` §29/§30.

**Fix.** Destructive authorization now requires a **three-signal AND**: the
request asserts `allow_destructive`, the client kind is interactive
(`TUI`/`CLI`, never `MCP`), and the daemon **kernel-verifies the connecting
peer's UID** via `SO_PEERCRED`/`peer_cred()` matches its own
(`crates/daemon/src/server.rs`, `crates/daemon/src/dispatch.rs`). A
same-UID policy decision cannot be forged over the socket.

**Regression tests.** `crates/daemon/tests/daemon.rs` asserts the
`destructive_allowed` matrix: allowed only for interactive clients with
verified same-user identity; denied for MCP clients, for
`allow_destructive: false`, and when the peer UID differs.

### H-IPC-3 — Run-directory permissions were re-applied on every start *(High)*

**Finding.** The daemon re-chmodded a pre-existing run directory on start-up.
If a developer had deliberately tightened permissions on their run directory,
the daemon silently broadened them — an unauthorized mutation of
developer-owned state.

**Fix.** The daemon only sets permissions when **it** creates the run
directory; a pre-existing directory is left exactly as found.

**Regression test.** Daemon start-up against a pre-existing, tightly
permissioned run directory leaves its mode untouched.

### G3 — Protocol violations returned unstructured text *(High)*

**Finding.** Malformed requests (bad JSON, wrong protocol version, unknown
method) could surface as raw strings instead of structured protocol errors,
contradicting `AGENTS.md` §28 and forcing clients to parse prose.

**Fix.** Every protocol-violation path now returns the standard structured
error envelope with `code: INVALID_REQUEST`
(`crates/daemon/src/server.rs`, `dispatch.rs`, `crates/client/src/remote.rs`).

**Regression tests.** Daemon and client tests assert `INVALID_REQUEST` on
malformed frames, version mismatch, and unknown methods.

## 3. Medium-severity findings and fixes

### D1 — Failed start transitions left stale "running" state *(Medium)*

**Finding.** When a start/stop/restart transition failed mid-flight, the
runtime manager kept the previous state, so a service could be reported as
running when its process had already failed.

**Fix.** `reconcile_failed_transition` re-derives the service's true state
from the process manager after every failed transition
(`crates/runtime/src/manager.rs`). Reported state always reflects reality.

**Regression tests.** `crates/runtime/tests/service_manager.rs` covers failed
start, stop, and restart reconciliation.

### D2 — A panicking task executor wedged dependents forever *(Medium)*

**Finding.** If a task executor panicked, the task never reached a terminal
state and every dependent stayed `blocked` indefinitely.

**Fix.** The scheduler wraps executor invocation in `catch_unwind`
(`crates/execution/src/scheduler.rs`) and fails the task with
`TASK_EXECUTOR_PANICKED`, releasing dependents into `blocked`/re-evaluation.

**Regression tests.** `crates/execution/tests/task_engine.rs` panics an
executor and asserts the task fails with `TASK_EXECUTOR_PANICKED` and
dependents do not hang.

### D3/D4 — Default timeouts were defined but never wired *(Medium)*

**Finding.** The configuration's `default_timeout` was parsed and documented
but not applied; operations could wait forever.

**Fix.** The composition root wires `default_timeout` into the runtime and
engine paths (`src/main.rs`); an unset value keeps the previous explicit
defaults.

**Regression tests.** Config-to-runtime wiring test asserts the configured
default reaches spawned operations.

### D5 — Git/build/liquibase runners could miss cancellation *(Medium)*

**Finding.** The runners selected on either the cancellation event stream or
process exit; a race between the two could leave a cancelled operation
running to completion.

**Fix.** All three runners now race `events.recv()` against `handle.wait()`
in one `tokio::select!` (`crates/git/src/runner.rs`,
`crates/build/src/runner.rs`, `crates/liquibase/src/runner.rs`), so
cancellation always wins when both are ready.

**Regression tests.** Per-runner tests cancel during execution and assert the
operation terminates promptly.

### D6 — Timeout did not signal cancellation downstream *(Medium)*

**Finding.** `run_attempt` enforced its timeout but did not propagate a
Cancellation signal, leaving the process manager unaware the task was over.

**Fix.** On timeout, `run_attempt` now signals `Cancellation` so downstream
owners can clean up (`crates/execution`).

**Regression tests.** Timeout-path test asserts the cancellation signal is
emitted.

### D7 — Unbounded outbound IPC queue *(Medium, security-adjacent)*

**Finding.** A slow or malicious client that never read its responses could
grow the daemon's per-connection outbound queue without bound — a local
denial-of-service vector.

**Fix.** The outbound channel is bounded (`OUTBOUND_CAPACITY = 256`,
`crates/daemon/src/server.rs`); `respond` is async and applies backpressure
rather than buffering. (Note: every `respond()` call **must be awaited** — a
missed `.await` here previously caused a test-suite hang.)

**Regression tests.** Daemon tests exercise sustained request bursts and
assert bounded memory and clean behaviour.

### D9 — Cancelling a blocked task left it pending *(Medium)*

**Finding.** Cancelling a task in the `blocked` state recorded the
cancellation but left the task pending forever.

**Fix.** The store settles a cancelled blocked task as `cancelled`
immediately (`crates/execution/src/store.rs`).

**Regression tests.** Store tests cover cancel-from-`blocked` and
cancel-from-`queued`.

### D10 — Process-manager cancellation was not watched *(Medium)*

**Finding.** The process manager did not watch for task cancellation, so a
cancelled start could orphan a half-started child process.

**Fix.** A cancellation watcher terminates the supervised process on cancel
(`crates/process/src/manager.rs`), preserving the "no orphaned children"
rule (`AGENTS.md` §15).

**Regression tests.** Process tests cancel a running start and assert no
orphaned PID remains.

### D11 — Scheduler dispatched tasks that should have been skipped *(Medium)*

**Finding.** After a dependency failed, dependents could still be dispatched
once before being marked failed — a wasted dispatch window in which a
dependent started against a broken prerequisite.

**Fix.** The scheduler checks dependency outcomes **before** dispatch and
skips/continues correctly (`crates/execution/src/scheduler.rs`).

**Regression tests.** Engine tests assert a dependent of a failed task is
never dispatched.

## 4. Reliability hardening added (no defect, better evidence)

Git §9 fixtures were extended to cover structured failure and terminal
states: a repository whose remote disappears reports
`GitError::RemoteUnavailable` / `GIT_REMOTE_UNAVAILABLE`; an already-cancelled
operation reports `GIT_CANCELLED`; an operation exceeding its timeout reports
a timed-out error; branch-with-no-upstream and merge-conflict statuses report
correctly. All use local fixture repositories — no developer checkout is ever
touched.

## 5. Task 017 deliverable — workspace validation

`ValidateWorkspace` (see `docs/workspace-validation.md`,
`docs/adr/017-workspace-validation.md`) was built read-only by construction:
the application core reaches the system only through the `WorkspaceProbe`
port, all real probes are side-effect free, and port occupancy is reported
without ever signalling the occupying process. The MCP, daemon, and stdio
integration tests assert that validation **submits no tasks and starts no
processes**.

## 6. Documentation consistency

- `docs/architecture.md` — new §18 "Where workspace validation belongs";
  subsequent sections renumbered; related-documents list extended.
- `docs/mcp.md` — `validate_workspace` added to the tool table.
- `docs/adr/017-workspace-validation.md` — decision record for the probe port
  and read-only guarantees.
- `docs/performance-report.md`, `docs/workspace-validation.md`,
  `docs/reliability-report.md` — this report's companions.

## 7. Remaining risks and known gaps

These were identified during the audit and deliberately left open; none is a
defect in shipped behaviour, and each is a candidate for a future task:

1. **Git argument validation (Medium).** Branch and path arguments are passed
   to the Git CLI as discrete arguments (never through a shell), but explicit
   validation that a branch name does not begin with `-` is not uniformly
   enforced at every call site. Recommendation: a single `BranchName`
   validator rejecting leading dashes and control characters.
2. **Log retrieval bounds (Low).** `get_logs`/`GetLogs` returns bounded,
   paginated results, but a caller-requested upper bound is not clamped at
   every layer; a very large `limit` could still allocate a large response
   buffer. Recommendation: clamp limits in the query layer.
3. **Error-context gaps (Low).** Some infrastructure errors (git, build)
   carry a code and message but not always the underlying stderr excerpt;
   diagnosis requires fetching task logs. Recommendation: attach a bounded
   stderr tail to structured errors.
4. **Test-hygiene observation (Low, process-level).** During this task two
   daemon processes started by ad-hoc measurement scripts outlived their
   scripts (leaked, then cleaned up manually). The shipped test harnesses
   (`tests/mcp_stdio.rs`, `crates/daemon/tests`) kill their daemons in
   `Drop` and stop them explicitly — no leak reproduces from the test suite;
   the leak was an artifact of hand-rolled scripts, not product code.

## 8. Verification state

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | ✅ clean |
| `cargo check --workspace` | ✅ clean |
| `cargo test --workspace` | ✅ **892 passed / 0 failed / 10 ignored** (baseline before Task 017: 863 passed) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ clean |

The 10 ignored tests are the 6 pre-existing ignored tests (Maven/JVM/network
fixtures) plus the 4 `#[ignore]`d scale/measurement tests in
`crates/application/tests/workspace_validation.rs`
(`a_large_workspace_validates_every_entity_exactly_once` and the three
measurement harnesses), which are ignored because they are opt-in scale and
measurement runs; they run on demand with `--ignored --nocapture` and are
documented in `docs/performance-report.md`.

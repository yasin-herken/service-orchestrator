# Performance Report — Task 025

Measured, not estimated. Every number below was produced on the machine
described below, by running the shipped binaries or the shipped
`#[ignore]`d measurement harnesses. Where a figure is a range it is a spread
of repeated observations; nothing is extrapolated.

This report supersedes the Task 017 report. It keeps the Task 017 read-path,
configuration, and IPC figures (re-measured and unchanged) and adds the Task
025 work: the task-store scheduling index, log-hub paging, the daemon dispatch
clone removal, and the React context split.

## Environment

| Item | Value |
| --- | --- |
| Machine | MacBook Pro, Apple M2 Pro |
| CPU | Apple M2 Pro (10-core: 6P + 4E), arm64 |
| RAM | 32 GiB |
| OS | macOS 26.6.2 (Darwin 25.83), Apple Silicon |
| rustc / cargo | 1.98.1, stable |
| Node / npm | v22.18.0 / 11.11.1 |
| Frontend | React 19.3, Tauri 2, Vite 8, Vitest 5 |
| Profile | `--release` unless explicitly marked *debug* |
| Workspace fixtures | deterministic synthetic configurations generated in-process by the `#[ignore]`d tests; no network, no real repositories |

## Methodology

All Rust measurements are taken by the shipped `#[ignore]`d harnesses and call
the real production code paths. Commands are listed in
[How to reproduce](#how-to-reproduce).

- **In-process use-case latency** (`list_services`, `get_workspace_status`,
  `validate_workspace`) is measured with `ScriptedProbe`, which answers every
  environment probe from memory. These figures therefore isolate the
  application core's own cost — no filesystem, `PATH`, or socket time.
  Per workload: one warm-up call, then a fixed number of samples averaged.
- **Configuration loading** writes a synthetic TOML document to a string and
  parses it with the real loader (`load_from_str`): one warm-up load, then a
  fixed number of timed loads.
- **Task-engine** measurements use a fake in-process executor (`CountingExecutor`)
  so the numbers reflect scheduling and bookkeeping only — never Maven, Git, or
  process speed. `next_ready`/`cancel_queued_signalled` micro-costs are measured
  in the store's own unit tests over a seeded terminal-task history.
- **Log-hub paging** records a fixed volume of synthetic entries, then pages
  repeatedly from several cursor depths with and without a text filter.
- **Daemon startup and IPC latency** launch the real daemon over a real Unix
  domain socket, wait for reachability with a 1 ms poll, then issue 500
  `list_services` round trips over one persistent client and report median,
  p95, and max.
- **Idle CPU and memory** sample `ps -o time=` / `ps -o rss=` against the
  release binary while it sits idle (daemon: fixture config, no client;
  TUI: attached to that daemon through a pseudo-terminal).
- **React rendering** uses a deterministic render-count probe in the existing
  Vitest suite (a `useTaskRevision` consumer) rather than a wall-clock profile:
  the change is exactly "how many times does this component render per event",
  which a counter measures precisely and a timer does not.
- Single-run figures are given where run-to-run variance was small (the
  microsecond-scale CPU-bound workloads varied under ~10 %); ranges/percentiles
  are given where the measurement is noisier (IPC over a real socket).

## Baseline measurements (before optimization)

Recorded against the pre-change code with the same harnesses. The
micro-costs are per call at the stated retained-history depth.

| Measurement | Baseline (before) | Context |
| --- | --- | --- |
| `TaskStore::next_ready` | **1.79 ms / call** | 20 000 retained terminal tasks |
| `TaskStore::cancel_queued_signalled` | **1.82 ms / call** | 20 000 retained terminal tasks (each scheduler wake) |
| Engine submit, 20 000 tasks | **19.8 s (989.9 µs/task)** | fake executor, concurrency 4 |
| Engine drain (round 1 → round 3) | **84.9 µs/task → 809 µs/task** | grew as terminal history accumulated (2 000 → 6 000) |
| Deep workflow, 1 000-step chain | **~190 ms** | submit + run, fake executor |
| `LogHub::page` deep cursor | **47.9 µs / call** | 20 000 retained entries, cursor at the tail |
| `LogHub::page` deep cursor + text filter | **50.3 µs / call** | same, `matching("CONNECTION")` |
| Idle daemon RSS (Task 017) | ~5.4 MiB | flat |
| Idle TUI RSS (Task 017) | ~4.8 MiB | flat |

The two store micro-costs and the engine submit/drain figures are the same
problem seen from different angles. `next_ready` scanned the full `order`
index from the start on every scheduler dispatch, and
`cancel_queued_signalled` scanned it on every wake. Because submission and
completion each trigger a wake, per-op cost grew with retained history, so an
N-task run cost O(N²). At N = 20 000 that is ~2×10⁸ map-entry visits — which,
at the ~90 ns measured per scanned entry, is exactly the ~18 s observed for
submission and the 1.8 ms per store scan. The baselines are internally
consistent with the post-change per-entry cost.

## Confirmed bottlenecks

Only issues demonstrated by the measurements above were changed.

1. **`TaskStore::next_ready` was O(retained history) per dispatch.** It iterated
   `order` from the beginning, checking each task's status and dependency
   readiness, returning the first ready one. Every dispatch paid this even when
   the ready task was near the end.
2. **`TaskStore::cancel_queued_signalled` was O(retained history) per wake.** It
   scanned every task to find signalled queued/blocked ones, even when nothing
   had been cancelled.
3. **`TaskStore::running()` cloned every task before filtering by status.**
4. **`EngineInner::publish_status_change` cloned the whole `Task`** (dependency
   list, progress, error) just to read its `kind` for the `TaskStarted` event.
5. **Workflow creation listed the entire task store** (`store.list()`) and
   filtered by workflow, instead of reading the workflow's own tasks.
6. **`LogHub::page` scanned from the start of the retained buffer on every
   page**, discarding everything at or before the cursor.
7. **`LogHub::contains_ignore_case` allocated two lowercased `String`s per
   retained entry per query.**
8. **The daemon cloned the `Application` twice per read-only request** — once
   to take a consistent snapshot, once to apply authorization.
9. **React: `terminalRevision` shared one context value with per-event task
   data**, so every consumer re-rendered on every progress/message frame; and
   **`LogViewer` recreated its follow interval on every poll** because `load`
   depended on the cursor state.

## Optimizations implemented

### Task store: incremental scheduling index (`crates/execution/src/store.rs`)

- Added a ready-set index `queued: BTreeSet<u64>` holding the sequences of tasks
  that are `Queued` and therefore dependency-satisfied. A task enters `Queued`
  only once its dependencies are met, and dependencies never un-satisfy, so the
  lowest sequence in the set is always the FIFO-ready task. `next_ready` is now
  an O(log n) minimum lookup.
- Added a cancellation-request index `cancel_requests: BTreeSet<u64>`; it is
  populated by `signal_cancel` and consumed on the next
  `cancel_queued_signalled`, so that step visits only signalled tasks. The
  request is consumed whether or not it settles a task (a running task observes
  cancellation cooperatively), exactly as the old scan did.
- `running()` now filters the index in place and clones only running tasks.
- A new `reconcile_ready` helper keeps the ready-set index consistent on every
  status change; the same bookkeeping is applied on initial insert and on
  dependency settlement.

FIFO order is preserved: selection is still by *submission* sequence, not by
readiness time. Regression tests pin this (see below).

### Task engine (`crates/execution/src/engine.rs`)

- Added `TaskStore::task_kind`, and `publish_status_change` uses it for the
  `TaskStarted` event instead of cloning the whole task.
- Workflow creation now reads only the workflow's own tasks
  (`store.workflow_tasks(&workflow_id)`) rather than listing and filtering the
  whole store. The set of published `TaskCreated` events is identical; the order
  is the workflow's own step order, which for a freshly created workflow is the
  same contiguous sequence order the old code produced.

### Log hub (`crates/logging/src/hub.rs`)

- `page()` seeks from the cursor with
  `BTreeMap::range((Excluded(after), Unbounded))` instead of scanning and
  skipping older entries.
- `contains_ignore_case` is now an allocation-free windowed ASCII
  case-insensitive comparison (`windows().any(eq_ignore_ascii_case)`). Non-ASCII
  bytes compare exactly, as before.

### Daemon (`crates/daemon/src/dispatch.rs`, `crates/daemon/src/server.rs`)

- `dispatch::route` now takes `Application` by value and applies authorization
  in place, removing the second clone per read-only request. The caller already
  holds an owned snapshot taken under the application read lock, so the
  consistency guarantee is unchanged.

### React desktop (`apps/desktop/src/…`)

- Split `terminalRevision` into its own context (`TaskRevisionContext`) with a
  dedicated `useTaskRevision()` hook. `useTaskEvents()` composes the data
  context and the revision context, so existing consumers are unchanged.
  `LogsPage` and `WorkflowsPage` — which only reload on a terminal settle —
  now subscribe to the revision alone and no longer re-render on every progress
  frame.
- `LogViewer` keeps its pagination cursor in a ref, so its `load` callback is
  stable across polls and the follow interval is never torn down and recreated.

## Post-optimization measurements (after)

### Task store and engine (release)

| Measurement | Before | After | Improvement |
| --- | --- | --- | --- |
| `next_ready` @ 20 000 history | 1.79 ms | **~90 ns** | ~20 000× (flat in history) |
| `cancel_queued_signalled` @ 20 000 history | 1.82 ms | **~9 ns** | ~200 000× (flat in history) |
| Submit 20 000 tasks | 19.8 s (989.9 µs/task) | **99.8 ms (4.99 µs/task)** | ~198× |
| Drain, round 1 (0 history) | 84.9 µs/task | **7.12 µs/task** | ~12× |
| Drain, round 3 (4 000 history) | 809 µs/task | **7.43 µs/task** | ~109× (now flat) |
| Deep chain workflow, 1 000 steps | ~190 ms | **plan 6.65 ms + run 5.29 ms** | ~16× |

`next_ready` per call by retained history: 93 ns (0) / 80 ns (1 000) / 104 ns
(5 000) / 90 ns (20 000) — flat. `cancel_queued_signalled`: 23 ns (0) /
17 ns (1 000) / 9 ns (5 000) / 9 ns (20 000) — flat. Deep-workflow scaling:
100 steps plan 1.36 ms / run 2.42 ms; 500 steps plan 4.79 ms / run 5.34 ms;
1 000 steps plan 6.65 ms / run 5.29 ms — linear in step count.

### Log hub (release)

20 000 retained entries, page limit 50, 2 000 pages per point:

| Cursor | No text filter | Text filter |
| --- | --- | --- |
| `after = 0` | 4.06 µs | 3.50 µs |
| `after = 10 000` | 3.08 µs | 4.03 µs |
| `after = 19 950` (tail) | **3.06 µs** | **3.30 µs** |

Against the 47.9 µs / 50.3 µs deep-cursor baselines this is ~16× / ~15×, and
the cost is now independent of cursor depth (it was previously linear in the
number of entries skipped).

### Daemon startup and IPC (release, real Unix socket)

Over 5 fresh launches:

| Metric | min | median | max |
| --- | --- | --- | --- |
| Daemon startup (config → reachable) | 1.43 ms | **1.47 ms** | 2.35 ms |
| `list_services` round trip, median per launch | 54.1 µs | **~55 µs** | 59.3 µs |
| `list_services` p95 per launch | 86.9 µs | ~137 µs | 174.0 µs |
| `list_services` max per launch | 215 µs | — | 405 µs |

500 round trips per launch. The dominant cost is the loopback socket round trip
plus request/response framing; the dispatch clone removal is a small,
consistent saving on top (the `Application` is a clone over `Arc`s plus `Copy`
authorization/policy fields, so the removed clone was cheap but non-zero).

### Read-path use cases at documented scales (release, in-process)

`ScriptedProbe` answers all probes from memory; no filesystem or socket time is
included.

| Scale | `list_services` | `get_workspace_status` | `validate_workspace` |
| --- | --- | --- | --- |
| 10 services + 2 libraries | 5.50 µs | 1.56 µs | 15.15 µs |
| 40 services + 10 libraries | 19.56 µs | 4.50 µs | 59.87 µs |
| 100 services + 25 libraries | 47.52 µs | 10.62 µs | 160.49 µs |
| 200 services + 50 libraries | 94.13 µs | 22.57 µs | 372.62 µs |

All three are linear in entity count. Even at 200 services `validate_workspace`
completes in ~0.37 ms. Ten consecutive `validate_workspace` calls over
100 services + 25 libraries took **1.812 ms total (181.2 µs per call)** —
no per-call growth, no accumulating state.

### Configuration loading (release, real TOML)

| Scale | Document size | Load time |
| --- | --- | --- |
| 10 services | 5 019 B | 186 µs |
| 40 services | 20 579 B | 752 µs |
| 100 services | 51 464 B | 1.74 ms |
| 200 services | 103 389 B | 3.59 ms |

Loading is a start-up-only cost (once per process start) and is the largest
single per-operation latency in the product; at 200 services it is still well
under 4 ms.

### Debug-profile reference

For developers running `cargo run` without `--release`, the same
`validate_workspace` scale test measured ~1.35 ms at 100 services and ~5.37 ms
at 200 services in the Task 017 report — roughly 7–15× the release figures.
Debug builds remain usable; they are simply not the numbers to quote. All
figures in this report are release unless marked otherwise.

## CPU and memory results (release)

| Process | Idle CPU (Δ CPU time over 15 s) | Idle RSS (constant over sampling window) |
| --- | --- | --- |
| `daemon` (fixture config, no client) | **0.0 %** (0.00 s delta; 0.01 s total since start) | **6 208 KiB ≈ 6.06 MiB** |
| `tui` (attached via pseudo-terminal) | **0.0 %** (0.00 s delta; 0.01 s total since start) | **~2.7 MiB** |

Both processes are event-driven and block on I/O when idle; neither spins on a
timer. RSS is flat across the sampling window (no drift). The daemon sits at a
fixed baseline because its task and log stores are bounded
(`docs/observability.md` retention limits) and it holds no per-request state
between calls.

The only long-lived unbounded structure is the terminal-task history in the task
store (see [Remaining risks](#remaining-bottlenecks-known-limitations-and-deferred-work)).

## Task Engine performance

- Submission: 20 000 tasks in 99.8 ms (**4.99 µs/task**) — down from 989.9 µs.
- Steady-state drain: **7.1–7.5 µs/task, flat** across accumulating history
  (was 84.9 → 809 µs and rising). This is the key change: scheduling cost no
  longer depends on how many tasks the daemon has already run.
- Dependency resolution, ready-set maintenance, and cancellation indexing are
  all O(log n) in retained history.
- Cancellation: the per-wake cancel step is now O(signalled) instead of
  O(retained). A signalled queued/blocked task settles on the next scheduler
  wake exactly as before; a signalled running task observes cancellation
  cooperatively and its request is consumed once (regression-tested).
- Concurrency limit is respected unchanged; the fake executor completes
  immediately, so the scheduler's own overhead is what is measured.

## IPC performance

- Daemon startup to reachability: **~1.47 ms median** (5 launches).
- Steady-state `list_services` round trip: **~55 µs median**, p95 ~137 µs, max
  ~405 µs over 500 calls/launch. Real socket, real serialization, real framing.
- Payloads are unchanged; no protocol or framing change was made. Only the
  redundant `Application` clone per read-only request was removed.

## Log streaming performance

- Historical paging cost is now independent of cursor depth: ~3.1 µs to reach
  the tail of 20 000 retained entries (was 47.9 µs), ~3.3 µs with a text filter
  (was 50.3 µs).
- Text matching no longer allocates per retained entry per query.
- Buffering and retention are unchanged and remain bounded; the hub still
  paginates and truncates exactly as before (no log-completeness or
  retention-policy change).

## React GUI performance

- **Revision-only views no longer re-render on progress/message frames.** The
  deterministic probe test emits a `TaskCreated`, then 20 `TaskProgress` frames,
  then one `TaskSucceeded`. The `useTaskRevision` consumer renders **0 times**
  across the 20 progress frames and exactly **once** on the terminal settle.
  Before the split, `terminalRevision` shared one context value with the
  per-event data, so such views re-rendered on every frame.
- `LogViewer`'s follow interval is stable across polls (cursor moved to a ref),
  so a live-following viewer no longer tears down and recreates its polling
  interval on every page.
- No virtualization or blanket memoization was added: no profiler evidence
  justified it at realistic service/log volumes, and the client log buffer is
  already bounded.

## Scale test results

Deterministic fixtures at 10 / 40 / 100 / 200 services (plus services/4
libraries):

| Scale | Config load | `list_services` | `get_workspace_status` | `validate_workspace` |
| --- | --- | --- | --- | --- |
| 10 | 186 µs | 5.50 µs | 1.56 µs | 15.15 µs |
| 40 | 752 µs | 19.56 µs | 4.50 µs | 59.87 µs |
| 100 | 1.74 ms | 47.52 µs | 10.62 µs | 160.49 µs |
| 200 | 3.59 ms | 94.13 µs | 22.57 µs | 372.62 µs |

All read paths scale linearly. A 120-service + 30-library workspace is also
validated entity-exactly-once by `a_large_workspace_validates_every_entity_exactly_once`
(run with `--ignored`). Large-scale fixtures are lightweight synthetic
configurations — no Spring Boot applications are started and no repositories
are cloned.

## Performance budgets

Budgets are set with roughly 10–50× headroom over the measured release figures,
so they tolerate a slower machine and real I/O while still catching a
regression that reintroduces superlinear scanning. They are expectations for
regression detection, not hard pass/fail gates.

| Metric | Budget | Measured (release) |
| --- | --- | --- |
| Idle daemon CPU | < 1 % | ~0 % |
| Idle daemon RSS | < 16 MiB | ~6.1 MiB |
| `list_services` @ 200 services | < 5 ms | 94 µs |
| `validate_workspace` @ 200 services | < 5 ms | 373 µs |
| IPC `list_services` p95 | < 5 ms | < 0.2 ms |
| Daemon startup | < 100 ms | ~1.5 ms |
| Task submit | < 50 µs/task | 5.0 µs/task |
| Task drain (steady state) | < 100 µs/task and flat | 7.4 µs/task, flat |
| Store `next_ready` | O(log n), flat in history | flat to 20 000 |
| Log page at any cursor depth | < 50 µs | ~3.3 µs |

## Regression safety

Every optimization preserves the guarantees in `tasks/025.md` §36:

- **Task state correctness** — the ready-set index only mirrors the existing
  `Queued` status; transitions and the state machine are untouched.
- **FIFO scheduling order** — preserved and pinned by
  `ready_selection_is_fifo_by_submission_not_readiness`.
- **Cancellation behaviour** — pinned by
  `cancelling_a_queued_task_removes_it_from_readiness`,
  `cancelling_a_blocked_task_settles_it_and_skips_dependents`, and
  `a_cancellation_signal_for_a_running_task_is_consumed_once`.
- **Dependency ordering / failure propagation** — pinned by
  `settling_a_failed_dependency_does_not_mark_it_ready`; the settled-dependents
  path is otherwise unchanged.
- **Log completeness** — paging, limits, and truncation are unchanged; only the
  seek and the case-insensitive match were re-implemented, pinned by
  `page_seeks_from_the_cursor_over_large_history` and
  `text_filter_matches_ignoring_ascii_case`.
- **Cross-interface consistency** — no interface-specific logic was added; the
  TUI, MCP, and desktop all keep calling the same use cases.
- **Security** — no authorization, policy, or redaction path was changed.

## Files modified

Tracked changes:

- `crates/execution/src/store.rs` — scheduling/cancellation indexes, `task_kind`,
  `running()` filter, `reconcile_ready`, regression tests and store benchmarks.
- `crates/execution/src/engine.rs` — `task_kind` use; workflow-create reads only
  its own tasks.
- `crates/logging/src/hub.rs` — cursor range seek, allocation-free filter, tests
  and benchmark.
- `crates/daemon/src/dispatch.rs`, `crates/daemon/src/server.rs` — `route`
  takes `Application` by value.
- `crates/daemon/tests/daemon.rs` — startup/IPC latency benchmark.
- `apps/desktop/src/context/TaskEventsContext.tsx` — revision context split and
  `useTaskRevision`.
- `apps/desktop/src/pages/LogsPage.tsx`,
  `apps/desktop/src/pages/WorkflowsPage.tsx` — use `useTaskRevision`.
- `apps/desktop/src/components/tasks/LogViewer.tsx` — cursor ref / stable
  interval.
- `apps/desktop/src/context/taskEvents.test.tsx` — render-count regression test.

New files:

- `crates/execution/tests/performance.rs` — task-engine scale benchmarks.
- `docs/task-025-progress.md` — progress record.
- `tasks/025.md` — task specification (untracked).

## Tests added

Correctness regressions (run in the normal suite):

- `crates/execution/src/store.rs`:
  `ready_selection_is_fifo_by_submission_not_readiness`,
  `cancelling_a_queued_task_removes_it_from_readiness`,
  `cancelling_a_blocked_task_settles_it_and_skips_dependents`,
  `a_cancellation_signal_for_a_running_task_is_consumed_once`,
  `settling_a_failed_dependency_does_not_mark_it_ready`.
- `crates/logging/src/hub.rs`: `page_seeks_from_the_cursor_over_large_history`,
  `text_filter_matches_ignoring_ascii_case`.
- `apps/desktop/src/context/taskEvents.test.tsx`:
  `does not re-render a revision-only consumer on progress events`.

Measurement harnesses (`#[ignore]`d, run with `--ignored --nocapture`):

- `crates/execution/src/store.rs`: `measure_next_ready_scaling`,
  `measure_cancel_scan_scaling`.
- `crates/execution/tests/performance.rs`: `measure_submission_throughput`,
  `measure_drain_throughput_with_growing_history`,
  `measure_deep_workflow_planning`.
- `crates/logging/src/hub.rs`: `measure_page_seek_scaling`.
- `crates/daemon/tests/daemon.rs`: `measure_daemon_startup_and_ipc_latency`.
- `crates/application/tests/workspace_validation.rs` (existing, reused):
  `read_path_latency_across_documented_scales`,
  `configuration_loading_latency_across_documented_scales`,
  `a_large_workspace_validates_every_entity_exactly_once`.

## Validation results

All commands completed successfully on the machine above.

Rust (`--release` figures; validation matrix run in the default profile):

- `cargo fmt --check` — clean.
- `cargo check --workspace` — clean.
- `cargo test --workspace` — **all green**; execution unit suite 37 passed
  (+2 ignored benches), logging unit suite 29 passed (+1 ignored bench), daemon
  integration 23 passed (+1 ignored bench), execution integration
  `performance` 3 ignored, plus all other crates green.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — clean.

Frontend (`apps/desktop`):

- `npm run build` — success (Vite 8, 111 modules, `index.js` 481.02 kB /
  124.29 kB gzip).
- `npm run typecheck` — clean.
- `npm run test` — **24 files / 219 tests passed**.

## Remaining bottlenecks, known limitations, and deferred work

1. **Terminal-task history is unbounded for the process lifetime.** The task
   store never evicts settled tasks, so a daemon that runs indefinitely under
   continuous churn grows its history (and the per-task `BTreeMap` entries)
   without bound. This is the largest remaining memory risk. It was **not**
   changed here because retention would alter observable semantics (task lookup
   by id, workflow summaries, the activity feed) and needs an explicit,
   documented retention policy — analogous to the log hub's — rather than a
   performance-driven deletion. Recommended as a follow-up.
2. **`LogHub` and the client log buffer remain the reference for bounding
   memory**; the task store is the gap.
3. **React log entry rendering is not virtualized.** `LogViewer` renders the
   bounded client buffer (no leak), but a very long visible log list still has
   many DOM nodes. No profiler evidence justified virtualization at realistic
   volumes, so it was left out per the "measure, then optimize" rule.
4. **Revision-only views refetch authoritative state on every terminal settle**
   and do not coalesce bursts. Left as-is; no measured issue.
5. **Git status is not cached.** `get_git_status` shells out to Git on every
   call. It is only ever invoked on explicit request — a TUI keypress, an MCP
   tool call, or a desktop command — and there is no background scanner, so
   there is no repeated-scan cost to optimize. Caching would add staleness that
   the "do not present stale state as current" rule forbids for no measured
   benefit. Deliberately deferred.
6. **Configuration loading is the largest single startup latency** (~3.6 ms at
   200 services). It is start-up-only and already small; if configuration
   hot-reloading is ever introduced, the loader should be profiled first.
7. **Port probing has a 150 ms timeout per declared port** in production
   (loopback TCP connect). Not observed as a problem — macOS loopback rejects
   immediately — but documented as a worst case for a machine that drops
   rather than rejects loopback connections.
8. **Long-running endurance beyond the sampling window was not run** to
   completion; idle CPU/RSS were verified flat over short windows only. No
   claim of multi-hour stability is made.

## How to reproduce

```bash
# Task-store micro-costs and log-hub paging scaling (release):
cargo test --release -p service-orchestrator-execution --lib  -- --ignored --nocapture --test-threads=1
cargo test --release -p service-orchestrator-logging    --lib  -- --ignored --nocapture --test-threads=1

# Task-engine submission / drain / deep-workflow scaling (release):
cargo test --release -p service-orchestrator-execution --test performance \
  -- --ignored --nocapture --test-threads=1

# Daemon startup + IPC round-trip latency (release, real socket):
cargo test --release -p service-orchestrator-daemon --test daemon \
  -- --ignored --nocapture --test-threads=1

# Read-path + config-load scale measurements (release, in-process):
cargo test --release -p service-orchestrator-application \
  --test workspace_validation -- --ignored --nocapture --test-threads=1

# Idle CPU/memory (release): start the daemon against a fixture config, then
# sample `ps -o rss= -p <pid>` and `ps -o time= -p <pid>` over an interval.
cargo build --release
target/release/service-orchestrator daemon --config <fixture-config> &

# React render-count regression:
cd apps/desktop && npm run test
```

Rerunning these on the same machine reproduces the figures within a few
percent (IPC over the real socket is the noisiest and is reported with
percentiles).

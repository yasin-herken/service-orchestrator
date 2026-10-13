# Task 025 progress — Performance & Resource Optimization

Status: **complete**. See `docs/performance-report.md` for the full report.

## Environment

| Item | Value |
| --- | --- |
| Machine | Apple M2 Pro, 32 GiB RAM |
| OS | macOS 26.6.2 (Darwin 25.83) |
| Arch | arm64 |
| rustc / cargo | 1.98.1 |
| Node / npm | v22.18.0 / 11.11.1 |
| Build mode | `--release` unless marked debug |

## Plan

1. Audit hot paths; record confirmed issues.
2. Add repeatable `#[ignore]` benchmark/scale harnesses; measure baselines.
3. Implement targeted, low-risk optimizations.
4. Re-measure; verify correctness with focused + full test suites.
5. Write `docs/performance-report.md`.
6. Run full validation.

All steps done.

## Confirmed audit targets

- Task store: `next_ready()` full-scan per dispatch (O(N)); `cancel_queued_signalled()`
  full-scan per scheduler wake; unbounded terminal-task history.
- Task store: `running()` clones every task before filtering.
- Task engine: `publish_status_change` clones the whole task just to read its kind.
- Task engine: workflow creation listed the whole store.
- Log hub: `page()` scans from the start on every page; `contains_ignore_case`
  allocates two lowercased strings per entry per query.
- Daemon: two `Application` clones per read-only request.
- React: `terminalRevision` shared one context value with per-event task data, so
  every page re-rendered on every progress frame; `LogViewer` recreated its
  follow interval on every poll because `load` depended on the cursor state.

## Implemented (with measurements)

- Ready-set index (`BTreeSet<u64>`) for `next_ready`; cancellation-request index
  for `cancel_queued_signalled`; `running()` filter-before-clone; `task_kind`
  lookup for the `Running` event; workflow creation publishes one workflow's
  tasks instead of listing the whole store.
  - `next_ready` @ 20k history: 1.79 ms → ~90 ns; `cancel_queued_signalled`
    @ 20k: 1.82 ms → 9 ns.
  - Engine submit 20k tasks: 19.8 s → 99.8 ms; drain @ 4k history:
    809 µs/task → 7.4 µs/task (now flat in history); deep 1000-step workflow:
    ~190 ms → 6.65 ms plan + 5.29 ms run (now linear).
- Log hub: `page()` seeks from the cursor via `BTreeMap::range`; allocation-free
  ASCII-case `contains_ignore_case`.
  - Deep-cursor page (20k retained): 47.9 µs → 3.06 µs; text-filtered page:
    50.3 µs → 3.30 µs.
- Daemon: `dispatch::route` takes `Application` by value, removing one clone per
  read-only request.
- React: split `terminalRevision` into its own context (`useTaskRevision`), so
  revision-only views no longer re-render per progress event; `LogViewer` keeps
  its cursor in a ref so the follow interval is stable.

## Deferred (documented as remaining risks, not implemented)

- Task-history retention/eviction (risky semantics change; needs an explicit
  retention policy).
- React log-entry render batching / virtualization and `terminalRevision`
  refetch coalescing (no profiler evidence; would add test fragility).
- Git status caching (status is on-demand only; no background scanner exists).

## Final validation

- `cargo fmt --check` — clean.
- `cargo check --workspace` — clean.
- `cargo test --workspace` — all green.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — clean.
- `npm run build` — success.
- `npm run typecheck` — clean.
- `npm run test` — 24 files / 219 tests passed.

Task 026 not started (per `tasks/025.md` §46).

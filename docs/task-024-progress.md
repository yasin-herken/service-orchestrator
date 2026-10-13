# Task 024 progress — Desktop UX & Cross-Interface Consistency

Status: **complete**.

(The task body is duplicated internally as Task 025 in places; the branch and
this record use the task number `024`. See `tasks/024.md`.)

## Scope chosen

A coherent, testable slice across the five required areas rather than broad
rewrites, reusing the existing core/IPC/daemon/Tauri/React components and
changing no Rust business logic and no IPC protocol:

1. Task-event terminal-state protection (`TaskEventsContext`).
2. Dialog focus trap and focus restore (`Dialog`).
3. Health detail loading/error/empty states + Retry (`HealthDetailPanel`,
   `HealthPage`).
4. Structured workflow-load error on the task detail (`TaskDetailPanel`,
   `TasksPage`).
5. Stable reveal-row identity in the environment editor (`EnvironmentEditor`).
6. One shared task-status/operation-label vocabulary (`utils/taskStatus.ts`,
   `utils/format.ts`).

## Plan (as executed)

1. `utils/taskStatus.ts`: single source of truth for the terminal task states
   (`TERMINAL_TASK_STATUSES`), `isTerminalTaskStatus`, and `taskStatusLabel`.
   `utils/taskProgress.ts` and `components/tasks/TaskStatusIndicator.tsx`
   delegate to it.
2. `context/TaskEventsContext.tsx`: the tracked-task map and its
   `terminalRevision` are one atomic state object; a settled task is frozen
   against late progress frames, duplicate terminal events, and out-of-order
   updates; a task created already terminal counts once.
3. `components/Dialog.tsx`: focus moves to the first focusable control on open,
   `Tab`/`Shift+Tab` are trapped, and focus returns to the opener on close.
4. `components/health/HealthDetailPanel.tsx` + `pages/HealthPage.tsx`: the
   detail panel renders a spinner while loading, a structured error with a
   working Retry on failure, and an explicit empty state only when nothing was
   observed. `detailError` is a typed `DesktopError` and a retry nonce re-reads
   the daemon.
5. `components/tasks/TaskDetailPanel.tsx` + `pages/TasksPage.tsx`: a failed
   workflow lookup is surfaced as a structured error instead of a silent gap.
6. `components/configuration/EnvironmentEditor.tsx`: each row has a stable id;
   the reveal set and React keys use it, removal cleans revealed state, and the
   reveal control's accessible name follows its action.
7. `utils/format.ts`: one shared `operationLabel`; the duplicated copies in the
   services, Git, builds, and libraries tables were removed.
8. Tests (frontend unit/component) and documentation.

## Decisions

- **No IPC protocol change.** The daemon silently dropping lagged task events for
  a slow subscriber (`crates/daemon/src/server.rs`) remains a documented
  limitation, not a daemon change. Correctness is preserved by terminal-state
  freezing plus the fresh snapshot taken on reconnect (`ConnectionContext.epoch`
  reconciliation already exists). Wrapping the protocol to add sequence numbers
  or a `Lagged` count for the task stream was judged out of proportion to this
  UX/consistency task and would be a protocol change.
- **Frontend-only changes.** No Rust workspace change was needed; the audit
  found no Rust defect to fix. All Rust/adapter validation still runs.
- **Shared helpers, not new abstractions.** Three duplicated terminal-status
  definitions were consolidated into `utils/taskStatus.ts`; the duplicated
  `operationLabel` implementations were consolidated into `utils/format.ts`.

## Verification (actually run)

- Rust workspace: `cargo fmt --check`, `cargo check --workspace`,
  `cargo test --workspace`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings` — all clean; zero test failures.
- Tauri adapter (its own workspace): `cargo fmt --check`,
  `cargo check --all-targets`, `cargo test`, `cargo clippy --all-targets
  --all-features -- -D warnings` — clean; 91 unit + 40 integration tests green.
- Frontend: `npm run build`, `npm run typecheck`, `npm run test` — all green;
  **218 tests across 24 files** (baseline before this task: 202 across 19).

## Known limitations (unchanged, documented)

- Daemon-side task/log event drops are not observable over IPC; the GUI relies on
  terminal-state freezing and reconnect snapshots rather than every intermediate
  event (see `docs/desktop-gui.md` §5, §26).
- Realtime refresh remains task-driven; service runtime, health, and Git
  transitions are refreshed when a task settles or on demand, not from a
  dedicated event stream.

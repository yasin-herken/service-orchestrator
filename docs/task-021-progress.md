# Task 021 progress — Migration, Tasks, Health & Realtime Logs GUI

Status: **complete**. All four screens are implemented on the existing
Application Core through the shared local daemon, all tests pass, and all
validation commands listed in `tasks/021.md` §45 completed successfully.

---

## 1. What was built

### Migration Management (`tasks/021.md` §4–§9)

- `pages/MigrationsPage.tsx` + `components/migration/{MigrationTable,MigrationDetailPanel,SqlPreviewDialog}.tsx`
- Workspace-scoped target list from `list_migrations` (configuration snapshot +
  the real latest migration task), search, multi-select, and a detail panel
  showing the full snapshot and capabilities.
- Operations: validate configuration, inspect status, inspect pending, SQL
  preview, apply (single, confirmed), apply selected (bulk, one coordinated
  `apply_migrations` workflow).
- Unknown `pending`/`locked` render as "Unknown"; a service that never ran
  renders "Never run"; a non-local target carries an explicit
  "authorization required" tag.
- SQL preview reads the settled preview task's own logs
  (`source === "migration"` preferred), with search + highlight, wrap toggle,
  copy (with an honest unavailable fallback), and a 2,000-line truncation.
- Safety: apply always goes through a danger confirmation naming the
  environment and database identity; nothing runs automatically; no retry; no
  Liquibase-lock release; cancellation never claims a rollback (stated in the
  confirmation text).

### Task Management (`tasks/021.md` §10–§16)

- `pages/TasksPage.tsx` + `components/tasks/{TaskTable,TaskDetailPanel}.tsx`
- One bounded `list_tasks(200)` listing (newest first, workspace-tagged) with
  all seven engine states kept distinct.
- Client-side filters: status, workspace, type, service/library, workflow, time
  window (15 min / 1 h / 24 h), free text; the workspace filter defaults to the
  active workspace and resets with it.
- Detail: identity, state, progress (indeterminate when the engine reports no
  total), timestamps, structured failure, and the workflow's declared steps and
  dependency edges from `get_workflow` plus sibling tasks from the event stream.
- Cancellation delegates to `cancel_task`/`cancel_workflow`; the returned task
  state is the authority; a `PermissionDenied` refusal is shown verbatim.

### Health Monitoring (`tasks/021.md` §17–§22)

- `pages/HealthPage.tsx` + `components/health/{HealthTable,HealthDetailPanel}.tsx`
- Runtime state and health state are separate columns; never-observed checks
  render explicit "Not checked" and an `unknown` badge.
- Detail panel lists every observed check (type, requirement, target,
  duration, message, time), diagnostics (managed, PID, ports), the structured
  last failure, recent errors, and reports history as **unavailable** when no
  transition was ever observed.
- Manual `check_service_health` / `check_service_readiness` run as daemon
  tasks; progress comes from the shared `OperationProgressDialog`.

### Realtime & Historical Logs (`tasks/021.md` §23–§28)

- `pages/LogsPage.tsx` + `components/logs/UnifiedLogViewer.tsx` — one viewer
  for every screen.
- Scopes: workspace, one service (push subscription + `daemon://log` events
  filtered by `subscription_id`), one task; scope seeded by cross-screen
  navigation.
- Bounded: 1,000-entry client buffer (oldest dropped, with a notice), 500-entry
  pages, sequence-based de-duplication, explicit "older entries exist" and
  "N entries were dropped" notices.
- Historical filters (level floor, source, free text) are part of the query;
  pagination continues from `next_cursor`; "Clear view" clears only the
  visible buffer.

### Cross-screen navigation (`tasks/021.md` §30)

- `types/nav.ts` `NavIntent { taskId?, serviceId? }`; `AppShell` holds the
  intent and routes `navigate(page, intent)`; each page consumes it once
  (failed migration → its task; failed task → its logs; unhealthy service →
  its health detail / logs).

---

## 2. Core and IPC integration

- The Tauri crate gained `migration.rs`, `health.rs`, `logs.rs`, and
  `log_fanout.rs`, each a thin adapter over the existing `OrchestratorClient`
  use cases. No new migration provider, engine, storage, health manager, task
  engine, or log store was introduced; the TUI and MCP keep working unchanged.
- `log_fanout.rs` multiplexes the single per-IPC-subscription-per-service log
  stream to several frontend subscriptions, with teardown ordered so a
  re-subscribe never races an unsubscribe.
- `crates/client/src/protocol.rs`: added `ClientKind::Desktop`
  (`is_interactive() == true`).
- `crates/daemon/src/dispatch.rs`: `destructive_allowed` requires all three
  trust signals — the request asks for it, the client kind is interactive, and
  the kernel-verified peer UID matches the daemon's. A desktop confirmation is
  therefore honored only for a same-user interactive peer; MCP and CLI clients
  can never self-confirm.
- `submit_apply` forwards the frontend confirmation as the request's
  `allow_destructive` intent; denials surface as `PermissionDenied` with the
  daemon's explanation.
- Task-producing operations: all migration operations except `migration_status`,
  both health checks, and readiness. `list_tasks(limit)` defaults to 200, max
  1000, newest first, workspace-tagged. `get_logs` resolves scope order
  task → service → workspace (config-resolved membership), oldest first, with
  `next_cursor` = newest scanned sequence and `truncated = len >= limit`.

---

## 3. Realtime event handling

- Task events: unchanged `daemon://task` forwarding; `terminalRevision` drives
  one authoritative refresh per settled task (used by Migrations, Health, Logs
  service lists).
- Log events: `daemon://log` with `entry`/`lagged`/`closed`, filtered by
  `subscription_id`; every subscription, listener, and poll timer is cleaned up
  on scope change, toggle, workspace switch, and unmount — no duplicate
  listeners.
- There is **no** `ServerEvent::Health`, so the Health screen refreshes from
  `terminalRevision` (health-kind tasks settling) instead of a dedicated event.

---

## 4. Tests

Frontend (vitest, mocked desktop API) — 4 files, 58 tests:

- `migrations-page.test.tsx` (15): listing, unknown pending, authorization tag,
  empty state, status snapshot + stale error, SQL preview success/failure,
  confirmed apply, cancel-without-apply, bulk coordinated workflow, progress,
  `PermissionDenied` explanation without retry, disconnection disabling actions,
  workspace switching.
- `tasks-page.test.tsx` (14): bounded listing, distinct terminal states, status/
  text filters, clear-all, workspace scoping, details with structured failure,
  workflow steps/siblings, cancellation + refusal, disconnection, workspace
  switching, cross-navigation to logs.
- `health-page.test.tsx` (12): runtime vs health separation, explicit unknowns,
  empty state, per-check detail, unavailable history, last failure + diagnostics,
  manual check and readiness, refusal, event-driven refresh, disconnection,
  workspace switching.
- `logs-page.test.tsx` (17): workspace/service/task scopes, subscription and
  cleanup, streamed entries, `lagged` notice, foreign-event filtering, level/
  source/text filters, cursor pagination, de-duplication, polling follow and
  stop, buffer limit, clear-view-only, workspace switching, disconnection.

Rust adapter (`apps/desktop/src-tauri`): 79 unit tests (migration request
mapping, task query/cancellation mapping, health mapping, log scope/cursor
mapping, input validation, error conversion, authorization propagation) and 26
integration tests in `tests/daemon_ipc.rs`, including 8 that drive the §42
scenarios against a real daemon over a private socket with fixture workspaces
and fake executors — including cross-interface consistency with TUI and MCP
clients (§43).

---

## 5. Validation (all successful)

```text
cargo fmt --check                                  clean (root workspace)
cargo check --workspace                            ok
cargo test --workspace                             ok (all crates)
cargo clippy --workspace --all-targets --all-features -- -D warnings
                                                   clean

cd apps/desktop/src-tauri                          (its own workspace)
cargo fmt --check                                  clean
cargo test                                         79 unit + 26 integration, ok
cargo clippy --all-targets --all-features -- -D warnings
                                                   clean

cd apps/desktop
npm run build                                      ok (tsc + vite)
npm run typecheck                                  ok
npm run test                                       17 files, 170 tests, ok
```

---

## 6. Known limitations (documented, not hidden)

1. **§42 item 5 — isolated real test database: documented skip.** No test in
   this repository provisions a disposable real database. The item is covered
   by `migration_apply_honors_only_the_daemon_verified_confirmation`, which
   applies against a local fixture through a fake migration executor and
   asserts the full authorization boundary (unconfirmed refusal, MCP/CLI
   non-interactive refusal, bulk refusal, desktop confirmation honored, exactly
   two applies reaching the executor). A real Liquibase round trip against an
   isolated database remains a manual verification.
2. **Dropped log/task events are not observable over IPC.** The daemon's event
   bus lags slow subscribers, and the desktop forwarder currently swallows the
   lag notification for those streams (`crates/daemon/src/server.rs`), so an
   end-to-end dropped counter needs a small daemon change and is deferred. A
   `lagged` event that does reach the adapter is rendered verbatim.
3. **Only service logs stream in realtime.** The subscription surface is
   per-service; workspace and task scopes follow by polling the cursor once a
   second — a bounded, explicit fallback, disabled while disconnected.
4. **Restart semantics.** Cancelling a migration task does not roll back
   database changes; the GUI says so in the confirmation and never claims
   otherwise. A failed migration is never auto-retried and a Liquibase lock is
   never auto-released.
5. **Health history is observation-derived** (latest observation + transition
   timestamps), not a time series; it is reported as unavailable when nothing
   was observed.
6. **Pending migration counts stay unknown** until a real inspection task
   answers; the GUI never guesses.

---

## 7. Deliverables

- New: `MigrationsPage`, `TasksPage`, `HealthPage`, `LogsPage`;
  `MigrationTable`/`MigrationDetailPanel`/`SqlPreviewDialog`,
  `TaskTable`/`TaskDetailPanel`, `HealthTable`/`HealthDetailPanel`,
  `UnifiedLogViewer`; 4 test files; adapter modules `migration.rs`,
  `health.rs`, `logs.rs`, `log_fanout.rs`.
- Modified: `desktopApi.ts`, `models.ts`, `nav.ts`, `mocks.ts`, `fixtures.ts`,
  `harness.tsx`, `ErrorDialog.tsx` (optional `hint`), `format.ts`
  (`formatDateTime`/`formatMillis`), `AppShell.tsx`, `global.css`,
  `shell.test.tsx`; adapter `commands.rs`, `dto.rs`, `events.rs`, `lib.rs`,
  `mutations.rs`, `state.rs`, `tasks.rs`, `tests/daemon_ipc.rs`;
  `crates/client/src/protocol.rs`, `crates/daemon/src/dispatch.rs`;
  `docs/desktop-gui.md`, `docs/architecture.md`.
- No ADR was created: no new architectural decision was taken — the screens
  reuse the adapter/core/daemon architecture introduced by `tasks/018.md`–
  `tasks/020.md`, with `ClientKind::Desktop` extending the existing client-kind
  model rather than replacing it.

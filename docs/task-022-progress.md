# Task 022 progress — Desktop Workflow Management & Profiles

Status: **complete**. The Workflows screen is implemented on the existing
Application Core through the shared local daemon, all tests pass, and every
validation command in `tasks/022.md` §44 completed successfully.

---

## 1. What was built

### Workflow Management screen (`tasks/022.md` §4–§27)

- `pages/WorkflowsPage.tsx` with four modes — `configure`, `plan`, `running`,
  `results` — and `components/workflow/`:
  - `WorkflowWizard` — the step container;
  - `ServiceSelection` — searchable, multi-select services (stable ids only);
  - `ProfileSelector` / `GroupSelector` — configured profiles and groups, each
    with the services it selects;
  - `WorkflowOptions` — the request options with `defaultWorkflowOptions()` /
    `toPrepareInput()`;
  - `DependencyView` — the dependencies the daemon added automatically;
  - `WorkflowPreview` — the read-only plan (phases, steps, warnings,
    destructive-authorization flag);
  - `WorkflowProgress` + `WorkflowStateBadge` — aggregate state and per-phase,
    per-service progress;
  - `WorkflowResults` — counts, per-phase breakdown, and structured failures;
  - `WorkflowHistory` — the bounded, workspace-scoped history list;
  - `WorkflowCancellationDialog` — confirmation before cancelling.
- **Selection:** exactly one source (explicit services XOR profile XOR group);
  the adapter rejects anything else with `INVALID_INPUT`. A blank/placeholder
  value can never be read as "the whole workspace".
- **Safe defaults:** `sync`, `include_dependencies`, and `wait_for_health` are on;
  `build_libraries`, `run_liquibase`, `build`, `start`, and `branch` are off.
  No workflow discards Git state, applies migrations, forces a branch, or
  auto-starts the workspace.
- **Planning is read-only:** "Preview plan" calls `plan_prepare_environment` and
  renders the daemon's plan; nothing is scheduled and history is unchanged.
- **Execution:** "Run workflow" submits the same request through
  `prepare_environment`; the daemon re-validates and re-authorizes at execution
  time. Frontend confirmations are never treated as authorization.
- **Monitoring:** while a workflow runs the screen polls `get_workflow_status`
  and `list_tasks` (`POLL_MS = 1500`) and refreshes immediately when a task
  settles via the shared `terminalRevision`. Progress, phase breakdown, and
  partial success are shown exactly as the engine reports them.
- **Cancellation:** `cancel_workflow` behind a confirmation dialog; best-effort,
  never a claimed rollback.
- **Results/history:** bounded `list_workflows` (adapter-clamped to default 20,
  max 100) scoped to the active workspace; partial success is distinguished from
  success.
- **Scope/disconnect:** switching workspaces resets selection, plan, and results;
  while disconnected, execution and cancellation are disabled, the workflow id
  is preserved, and state reloads on reconnect.
- **Entry points:** Services ("Prepare workflow…" carrying `workflowServiceIds`)
  and Dashboard ("Prepare environment…"); `NavIntent` extended with
  `workflowServiceIds` / `workflowProfileId` / `workflowGroupId`, and `workflows`
  added to `PageId`/`PAGES`.

---

## 2. Core and IPC integration

- New Tauri module `apps/desktop/src-tauri/src/workflow.rs` with **8 commands**:
  `list_profiles`, `get_profile`, `list_groups`, `get_group`,
  `plan_prepare_environment`, `prepare_environment`, `get_workflow_status`,
  `list_workflows`. All are thin adapters over the existing `OrchestratorClient`
  use cases — no new engine, scheduler, provider, or storage. The TUI and MCP
  keep working unchanged.
- `PrepareEnvironmentInput` (serde `#[serde(default)]`, custom `Default`) is the
  single request mapping shared by planning and execution, so a preview can never
  drift from what execution schedules. `build_request()` enforces exactly one
  selection source and validates every identifier at the boundary.
- `validation.rs` gained `parse_profile_id` / `parse_group_id`; `dto.rs` gained
  `ProfileView`, `GroupView`, `PlanStepView`, `PlanPhaseView`,
  `EnvironmentPlanView`, `WorkflowPhaseProgressView`, `WorkflowTaskFailureView`,
  `WorkflowStatusView`, and `WorkflowSummaryView`.
- **Workflow history is derived.** The daemon persists no workflow table, so
  `list_workflows` groups the engine's active and retained tasks by workflow id
  and reads each candidate's workspace from the resolved parameters of its steps
  (`get_workflow`), then queries `get_workflow_status` for a bounded window.
  Workflows that aged out of retention are skipped, not errors.

### One small core fix

`list_workflows` (and the integration suite) surfaced a defect in the existing
composer: the in-workflow **health** step was scheduled with empty parameters,
so the `HealthCheck` executor — which decodes its target from parameters, exactly
as the lifecycle executors do — always failed with `HEALTH_TARGET_INVALID`. The
composer now attaches the resolved `service_runtime_target` parameters to the
health step (`crates/application/src/commands/workflow.rs`), and
`crates/application/tests/application_core.rs` asserts the step carries
`service.id` / `service.workspace`. This is a one-line parameter fix, not a new
engine; without it, any workflow that starts services and waits for health would
report a spurious failure.

---

## 3. Frontend integration

- `types/models.ts`: corrected/added profile, group, plan, status, summary, and
  `PrepareEnvironmentInput` shapes plus `WORKFLOW_RUN_STATE_LABELS` and
  `isTerminalWorkflowState()`.
- `services/desktopApi.ts`: the eight new typed calls (plan/execute invoke with
  `{ input }`).
- `test/{mocks,fixtures,harness}.tsx`: new API mocks, workflow fixtures, and
  happy-path defaults.
- `styles/global.css`: workflow/wizard/selection/dependency/preview/progress/
  results styles (light + dark).

---

## 4. Tests

Frontend (vitest, mocked desktop API) — `pages/workflows-page.test.tsx` (**20
tests**): profile/group/service selection, single-source enforcement, workflow
options and safe defaults, read-only plan and dependency preview, plan failure,
execution, realtime phase/per-service progress, cancellation with confirmation,
partial-success results, bounded history, workspace-scope reset, and
daemon-disconnect behaviour that disables execution while preserving the
workflow id. Full suite: **18 files, 190 tests, all passing.**

Rust adapter (`apps/desktop/src-tauri`): **90 unit tests** (including the
workflow request mapping, safe defaults, plan/status/summary view rendering,
history scoping/limits, and deserialization defaults) and **33 integration
tests** in `tests/daemon_ipc.rs`, seven of which cover the `tasks/022.md` §42
scenarios against a real daemon on a private socket with fixture workspaces and
fake executors:

1. `profiles_and_groups_load_and_are_workspace_scoped`
2. `a_plan_resolves_dependencies_without_scheduling_anything`
3. `a_workflow_executes_and_reports_its_results`
4. `a_workflow_reports_partial_failure_per_step`
5. `a_running_workflow_is_observable_and_cancellable`
6. `the_tui_and_mcp_observe_the_same_workflow`
7. `a_reconnecting_gui_reloads_state_after_a_daemon_restart`

No real developer repository, process, or database is touched.

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
cargo test                                         90 unit + 33 integration, ok
cargo clippy --all-targets --all-features -- -D warnings
                                                   clean

cd apps/desktop
npm run build                                      ok (tsc + vite)
npm run typecheck                                  ok
npm run test                                       18 files, 190 tests, ok
```

---

## 6. Known limitations (documented, not hidden)

1. **Workflow history is derived and in-memory.** The daemon persists no
   workflow table, so `list_workflows` reconstructs a bounded window from the
   engine's retained tasks; history is lost on a daemon restart and an aged-out
   workflow is simply absent rather than an error.
2. **Composed workflow failures are identified by task, not by entity.** A
   workflow step is scheduled with a workflow target, so a failed step's task
   carries no service or library binding; Results shows the task id, kind, code,
   and the executor's message (which names the service).
3. **Workflows are consumed, not authored.** Profiles and groups come from
   configuration; the GUI selects and previews them but never creates or edits
   them.

---

## 7. Related documents

- `docs/desktop-gui.md` §27 — the Workflows screen.
- `docs/environment-workflows.md` §12 — desktop usage of the same workflow.
- `docs/architecture.md` §25 — the adapter's workflow commands.

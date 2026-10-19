# Workspace Control Center

The **Workspace Control Center** is the desktop GUI's default landing screen
(`tasks/033.md`). It gives a developer one place to choose workspace services,
configure the existing environment operations, inspect the daemon's real
read-only plan, run one coordinated `prepare_environment` workflow, and follow
its progress, results, tasks, and logs without changing tabs.

The Control Center is an **interface**, not a new engine. It reuses the
application core's environment workflow, the daemon IPC boundary, migration
presets, build-provider capabilities, profiles, groups, workflow status, task
records, and bounded log APIs. There is no second scheduler, process manager,
workflow engine, configuration system, or build logic in React or in the Tauri
adapter.

```text
React Control Center
    │ typed desktop API
    ▼
Tauri commands ──▶ shared daemon ──▶ application core
    │                                  ├─ plan_prepare_environment
    │                                  ├─ prepare_environment
    │                                  └─ workflow / task / log state
    ▼
inline plan · progress · results · task and service logs
```

## 1. The main screen

Going to **Workspace Control Center** in the sidebar shows one page:

| Area | What it contains |
| --- | --- |
| Overview | The active workspace, service count, connection banner when the daemon is not connected, and the quick actions. |
| Selected services | The applied selection with name, kind, process state, and health state. |
| Operation configuration | Inline Git, libraries, Liquibase, build, runtime, and health controls. |
| Plan & run | **Preview plan**, **Prepare & Run**, and the daemon's read-only plan. |
| Progress | Appears after submission: workflow id, current phase, aggregate counts, per-phase and per-service task state, and cancellation. |
| Results | Appears when the daemon reports a terminal state: overall result, per-phase counts, structured failures, per-service tasks, and task ids. |
| Final runtime and health | The selected services re-read from the daemon after the workflow settles. |

The page is scoped to the active workspace. Switching workspaces resets the
selection, options, plan, and observed workflow, because service ids and
configuration belong to one workspace.

Nothing is loaded into the plan or executed by opening the screen. Rendering,
filtering, and restoring a saved selection are inert.

## 2. Selecting services

Press **Select services…** or `⌘K` / `Ctrl+K` to open the picker.

- **Search** matches the service id or display name.
- **Kind filter** narrows to the service kinds present in the workspace
  (for example `backend`, `frontend`, `library`).
- **Select all**, **Select visible**, and **Clear selection** operate on stable
  workspace-scoped service ids.
- **Profiles** replace the selection with the profile's resolved services.
- **Groups** replace the selection with the group's member services.
- Multi-selection is always explicit. The picker never implies "the whole
  workspace".
- Each row shows the service's kind, current branch, runtime, process state, and
  health state.

Selection lives in a local draft until **Apply selection**, so searching,
changing filters, and choosing a profile or group never lose checked services.
Required dependencies are **not** added here; the daemon's plan resolves them
and shows exactly which services were included.

## 3. Inline operations

Every control maps one-to-one onto an existing `PrepareEnvironmentRequest` field.
Controls that are irrelevant to the current operation are hidden rather than
simulated. Service templates are not required here: any defaults they contributed
to the validated service configuration are used as-is, and the Control Center
never duplicates template settings.

### Git sync

**Synchronize repositories** is the Git phase master switch. When it is on:

- **Fetch** schedules a fetch.
- **Checkout** checks out the optional target branch.
- **Fast-forward pull** fast-forwards the current branch.

Leaving the branch empty uses each service's configured default branch — shown
under the input from real configuration. The Control Center never invents
`master`, never runs `git reset` or `git clean`, and never discards local
changes. A dirty working tree is a structured failure with actionable guidance;
the developer decides how to reconcile it.

### Libraries

**Build & install required libraries** turns on the workflow's
`build_libraries` phase, which builds the resolved dependency closure in order.
**Library tools…** opens the existing Library Manager operations — verify an
artifact, inspect consumer version alignment, and install a library locally —
as on-demand actions independent of the coordinated workflow.

### Liquibase

**Run Liquibase migrations** enables the migration phase for services that have
migrations enabled. The optional preset selects one of the daemon's existing
migration presets:

- **Status** — read-only inspection.
- **Update** — apply pending changesets.
- **Reset local database** — a destructive preset allowed only for declared
  local environments.

Contexts and the allowed environments come from the preset, never from the
screen. Selecting a destructive preset shows its warning immediately. The
daemon still verifies every declared target and requires interactive
authorization; destructive migrations are never retried automatically.

### Build services

**Build selected services** uses each service's configured build provider. The
**Build operation** options are the union of operations the selected services'
providers expose (`clean`, `build`, `test`, `install_dependencies`,
`publish_local_artifact` as applicable). The UI never hard-codes Maven or npm,
and the daemon rejects an unsupported override.

### Runtime and health

Runtime actions are exclusive:

- **Do not change runtime state**
- **Start services**
- **Restart services**
- **Stop services**

**Wait for readiness after start** runs the configured health checks after a
start or restart. **Run health checks without starting** adds standalone health
checks for services that declare them. Health checks cannot be combined with
stopping services, because the resulting state would be ambiguous; choosing
stop clears the standalone check.

## 4. Advanced settings

**Advanced settings…** opens a compact dialog for real request overrides:

- include required dependencies;
- wait for readiness after start;
- build operation override;
- migration preset override.

Values owned by service configuration are shown **read-only** with secrets
redacted: working directory, default branch, build system, build and start
commands, migration preset and timeout, health checks, and environment
variables.

Overrides apply to the next plan and run only. They are never written to
service configuration, never carry authorization, and never bypass the daemon.
The selected services and options may be remembered locally as described in
[Saved selection](#10-saved-selection). Use **Open Configuration…** to change
configuration-owned values.

## 5. Previewing the plan

**Preview plan** calls `plan_prepare_environment` and renders the daemon's own
plan:

- selected and resolved services, plus dependencies added automatically;
- required libraries;
- phases in canonical order (`validate`, `git-sync`, `library-prepare`,
  `migration`, `service-build`, `service-stop`, `service-start`,
  `health-check`, with empty phases omitted);
- every step with its task kind and description;
- total expected tasks;
- warnings (for example an uncommitted working tree);
- whether any step requires destructive authorization.

Planning schedules nothing and changes nothing. If the selection or options
change after a plan is generated, the displayed plan is marked stale; preview it
again to inspect the new request. **Prepare & Run** always plans the current
request afresh before submitting it, and the daemon re-validates regardless.

## 6. Running the workflow

**Prepare & Run** performs exactly one coordinated submission:

1. build the request from the selection and options;
2. call `plan_prepare_environment`;
3. if the plan requires destructive authorization, show a confirmation that
   explains the protected operation;
4. call `prepare_environment` once and follow the returned workflow id.

Ordinary actions need no confirmation: the plan is already visible on the same
screen. A protected operation requires the extra acknowledgement, and the daemon
independently verifies the client and user. The dialog is intent, not
authorization; a frontend confirmation cannot substitute for the daemon's
decision.

If the daemon schedules no workflow, the screen says so instead of inventing a
task id.

### Quick actions

Quick actions use the same path — build a request, plan it, confirm only if it is
protected, then submit:

| Quick action | Request composition |
| --- | --- |
| Git Sync Only | Fetch, checkout, and fast-forward pull; no build, migration, or runtime change. |
| Build Only | Build with the configured operation; no Git, migration, or runtime change. |
| Start Only | Start and wait for configured health; no Git or build. |
| Stop Only | Stop in reverse dependency order; no health check. |
| Restart Only | Stop then start; wait for configured health; no rebuild. |
| Health Check Only | Standalone configured health checks; nothing is started. |
| Full Prepare Environment | Sync, build libraries, build services, start, and wait for health. |

Selecting a quick action also updates the inline controls, so the screen always
shows the request that was submitted.

## 7. Progress

After submission the page shows, from the daemon:

- the workflow id, kind, and aggregate state;
- the current phase, derived from the daemon's phase counters;
- aggregate task counts (`succeeded`, `failed`, `cancelled`, `skipped`,
  `pending`) and the total;
- per-phase progress;
- per-service task state with progress messages and durations;
- a **Logs** action per task that opens bounded task logs in a modal.

The GUI polls the daemon while the workflow is active and refreshes on task
events; it never declares a terminal state itself. **Cancel workflow…** opens a
confirmation that states the consequences and then calls the daemon's
cancellation. Completed work is preserved; cancellation does not roll back Git
operations, installed artifacts, or migrations.

Progress stays on the Control Center. No automatic navigation to the Tasks
screen occurs.

## 8. Results

When the daemon reports a terminal state, results render on the same screen:

- the daemon's overall result (`succeeded`, `partially_succeeded`, `failed`,
  `cancelled`);
- total, succeeded, failed, cancelled, skipped, and pending counts;
- per-phase results;
- structured failures with task id, kind, service or library, stable error
  code, and the executor's message;
- per-service tasks with task ids and **Task** / **Logs** actions;
- final runtime state, health state, branch, and port for the selected services,
  re-read from the daemon after the workflow settles.

A partial success is never presented as success. **Plan again from this
configuration** keeps the selection and options but discards the completed run;
nothing is retried automatically.

## 9. Safety rules

- The daemon is authoritative for planning, validation, authorization, and
  execution.
- No Git reset, clean, branch discard, or force operation is available.
- Dirty working trees stop the Git step with an actionable error; local changes
  are preserved.
- A branch override takes effect only when Git checkout is enabled.
- Migrations affect only services with migrations enabled.
- Destructive resets require a verified local target and daemon authorization.
- Health checks cannot be combined with stopping services.
- Closing the app or losing the connection never replays an uncertain
  operation.
- No secret is rendered; configuration environment values marked secret are
  masked in the read-only view.
- Temporary overrides are never written to service configuration, and no
  authorization is stored anywhere.

### Reconnection

When the connection is lost the Control Center:

- shows the connection state and a **Reconnect** action;
- disables **Prepare & Run**, quick actions, cancellation, and other mutations;
- preserves the selected services, options, plan, and workflow id;
- resumes observing the same workflow after a safe reconnect.

It never resubmits a workflow that may have started, and it never guesses a final
state. If no workspace can be read because the daemon is unreachable, the screen
says it is waiting for the connection instead of claiming that no workspace is
configured.

### Actionable errors

Structured failures are shown inline with guidance for the common cases:

| Situation | Guidance |
| --- | --- |
| Dirty repository | Commit, stash, or discard the changes yourself; no automatic reset or clean. |
| Missing branch | Check the spelling or fetch first. |
| Diverged branch | Reconcile manually; the workflow only fast-forwards. |
| Maven/Node toolchain unavailable | Check the runtime toolchain and service configuration. |
| Build failure | Open the failed task logs; nothing is retried automatically. |
| Unauthorized Liquibase target | The daemon refused a protected operation; a frontend confirmation is not authorization. |
| Port conflict | Stop the other process or change the configured port. |
| Health timeout | Inspect the service logs and configured health check, or adjust the timeout. |
| Daemon disconnected | Reconnect; the selection is preserved and nothing is replayed. |

## 10. Saved selection

The Control Center remembers the last active workspace together with that
workspace's selected service ids and options in versioned local storage
(`service-orchestrator.control-center.v1`). On load it switches to the saved
workspace when the daemon still reports it, then restores the selection.
Restoring is inert: it applies local state only and never plans, executes, or
authorizes anything.

It stores no plan, workflow id, credential, secret, or authorization. Unknown
versions, malformed data, and unavailable storage fall back to the safe defaults.

## 11. Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `⌘K` / `Ctrl+K` | Open the service picker. |
| `⌘Enter` / `Ctrl+Enter` | Run the current configuration (equivalent to **Prepare & Run**). |
| `Escape` | Close the open dialog. |
| `Tab` / `Shift+Tab` | Move within a dialog; focus is trapped while it is open. |

Quick actions and the service picker are ordinary buttons and are reachable
without the shortcuts.

## 12. Scale and performance

The screen handles large workspaces (40 repositories, 100+ services) by:

- selecting with stable ids rather than row positions;
- filtering client-side without extra IPC;
- deriving visible build operations in one pass from the daemon's capability
  rows;
- polling one workflow rather than every service continuously;
- loading logs only when a task or service log dialog is opened;
- avoiding duplicate requests when reconnecting or refreshing.

## 13. Related documentation

- [`environment-workflows.md`](environment-workflows.md) — the workflow in depth.
- [`workflows.md`](workflows.md) — user-facing workflow behaviour.
- [`migration-presets.md`](migration-presets.md) — preset contexts and safety.
- [`git-operations.md`](git-operations.md) — Git state and destructive rules.
- [`logs-and-diagnostics.md`](logs-and-diagnostics.md) — bounded task and
  service logs.
- [`desktop-gui.md`](desktop-gui.md) — the rest of the desktop GUI.

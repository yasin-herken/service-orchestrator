# Bulk Operations

The **Bulk Operations** screen is the single place to prepare a development
environment for many services at once. It is intentionally one dense screen:
select, choose the phases, preview the daemon's plan, execute, and watch the
workflow — without navigating between per-service forms or rebuilding a
workflow by hand.

It does not add a second workflow engine. Every operation is composed by the
existing application use case (`plan_prepare_environment` /
`prepare_environment`) and executed by the existing task and workflow engine.

```text
selection + options
      ↓
plan_prepare_environment   (read-only)
      ↓
prepare_environment        (one coordinated workflow)
      ↓
task engine → workflow status → results
```

## 1. Selecting services

Exactly one selection source is active at a time:

- **Services** — explicit service ids.
- **Profile** — the members of a configured profile.
- **Group** — the members of a configured group.

A request carries one of `services`, `profile`, or `group`. Profile and group
membership is expanded by the daemon from the current configuration; the GUI
does not duplicate that logic. Dependencies are included by default
(`include_dependencies = true`) and resolved into a dependency order by the
core, never alphabetically or in configuration order.

The default selection is empty. Nothing is selected, planned, or executed
implicitly when the screen opens.

## 2. Options

| Option | Request field | Default |
| --- | --- | --- |
| Synchronize repositories | `sync` | on |
| Target branch | `branch` | empty (keep each configured branch) |
| Include dependencies | `include_dependencies` | on |
| Build required libraries | `build_libraries` | off |
| Run migrations | `run_liquibase` | off |
| Migration preset | `migration_preset` | each service's configured default |
| Build services | `build` | off |
| Build operation | `build_operation` | each service's configured operation |
| Runtime action | `start` / `restart` / `stop` | none |
| Wait for health | `wait_for_health` | on, and only meaningful for start/restart |

The defaults are safe: synchronize and include dependencies, but do not build,
migrate, start, stop, or restart anything until asked.

When the runtime action is `start`, an already-running service is not restarted
(the daemon reports it and skips it). When it is `restart`, a running service is
stopped and started again.

### Restart ordering

A restart is one coordinated composition, not a frontend loop. Per service the
order is:

```text
stop    the running process (reverse dependency order: dependents first)
build   its replacement (dependency order)
start   the built service (dependency order)
health  after the service starts
```

Each build step depends on that service's stop step, so a restart never rebuilds
on top of a process it is about to replace. Services without a running process
have no stop step and are simply built and started.

The stop half only targets services that declare the run capability and are
actually running; the plan reports the rest as warnings. Services that cannot be
started are skipped with a warning, never silently treated as succeeded.

### Migrations

Selecting **Run migrations** reveals the available migration presets (built-ins
and user-defined). A destructive preset is clearly marked and warned about. The
warning is not authorization: planning reports
`requires_destructive_authorization`, and execution still requires
`Destructive` permission for the concrete target. See
[`migration-presets.md`](migration-presets.md).

## 3. Plan first

**Preview plan…** calls the read-only planner and renders:

- the selected and dependency-resolved services;
- included dependencies and required libraries;
- the ordered phases (`validate`, `git-sync`, `library-prepare`, `migration`,
  `service-build`, `service-stop`, `service-start`, `health-check`);
- step count, target branch, warnings, and whether any step requires destructive
  authorization.

Planning writes nothing, starts nothing, and requires only read permission. The
plan is produced by the same composer that execution uses, so a preview cannot
describe different work than the workflow that runs.

## 4. Execute and observe

**Execute operation…** asks for confirmation, then submits the same request.
Execution re-validates every precondition and re-applies the policy; a plan is
not authorization.

- A single-task operation returns a task id; a composed environment operation
  returns a workflow id.
- The screen polls the daemon's workflow status and shows per-phase progress,
  aggregated counts, and structured failures.
- Terminal state always comes from the daemon. The GUI never invents a
  percentage, marks a task successful, or overrides a settled state.
- **Cancel** requests cooperative cancellation of every still-active task in the
  workflow. Completed results stand, pending tasks stop scheduling, and nothing
  the workflow did not start is stopped.
- There is no automatic retry. After a failure the developer can adjust the
  options and plan again.

Results list failed, cancelled, and blocked tasks with their structured errors,
and link to the task and its logs. A partially succeeded workflow is shown as
partially succeeded, never as success.

## 5. Saved operation presets

A preset stores a commonly used *selection and option set*. It is an ordinary
configuration entity:

```toml
[[operation_presets]]
id = "full-stack-local"
name = "Full Stack (Local)"
description = "Sync, build libraries, migrate, restart, and verify the full stack."
workspace = "acme"
profiles = ["full-stack"]
default = true

[operation_presets.git]
enabled = true

[operation_presets.libraries]
enabled = true

[operation_presets.migration]
enabled = true
preset = "update"

[operation_presets.build]
enabled = true
operation = "build"

[operation_presets.runtime]
action = "restart"

[operation_presets.health]
enabled = true
```

| Field | Meaning |
| --- | --- |
| `id` | Stable identifier, unique among presets. |
| `name` / `description` | Display text. |
| `workspace` | The owning workspace. When omitted it applies anywhere; when members are declared, validation infers the workspace. |
| `default` | At most one default per workspace. |
| `services` / `profiles` / `groups` | The selection. At least one member is required. Profiles and groups are expanded to explicit services when composed. |
| `git.enabled` / `git.branch` | Synchronization and optional branch. |
| `libraries.enabled` | Build required libraries. |
| `migration.enabled` / `migration.preset` | Migration and its preset. |
| `build.enabled` / `build.operation` | Build and its operation. |
| `runtime.action` | `none`, `start`, `restart`, or `stop`. |
| `health.enabled` | Run health checks after startup/restart. |

An omitted phase group is disabled, except that `migration.enabled` defaults to
`preset.is_some()` — naming a preset is an explicit intent to run it. Explicit
`enabled = false` always wins, even when a preset id is named.

### Preset safety

- A preset **never stores credentials** and has no token, password, or secret
  field.
- A preset **never stores or implies authorization**. Destructive operations
  require the normal per-execution authorization; loading a preset that includes
  a reset preset does not grant it.
- **Loading a preset never executes it.** It only pre-fills the screen. Nothing
  is planned or executed until the developer presses Preview or Execute.
- Loading a preset is a read-only query (`operation_preset_request`), which
  expands the members and returns a plain `PrepareEnvironmentRequest` for
  review.
- The default preset is a convenience only; it is not auto-run at startup.

### Managing presets in the GUI

The Bulk Operations screen can save the current selection/options as a preset,
rename it, delete it, or mark it as the workspace default. Every one of those
actions goes through the shared configuration review
(`preview_configuration` → diff/validation → `commit_configuration`) with a
revision check. “Set default” first clears the previous default in the same
change set, so a workspace never ends up with two defaults.

## 6. Permissions

| Operation | Permission |
| --- | --- |
| List presets, compose a preset request | `Read` |
| Plan an environment | `Read` |
| Execute an environment | `SafeWrite`, plus `Destructive` when a migration targets a non-local database or uses a destructive preset |
| Save/rename/delete a preset | `SafeWrite`, or `Destructive` when overwriting another definition |

The GUI forwards a confirmation flag as intent only. The daemon authorizes the
request itself, and the same policy applies to the TUI and MCP.

## 7. IPC and desktop adapter

| IPC operation | Purpose |
| --- | --- |
| `list_operation_presets` | List presets for a workspace (including the composed request). |
| `operation_preset_request` | Expand one preset into a `PrepareEnvironmentRequest`. |
| `plan_prepare_environment` | Read-only plan. |
| `prepare_environment` | Submit one coordinated workflow. |
| `get_workflow_status` | Poll aggregated workflow state. |
| `cancel_workflow` | Request cancellation of active tasks. |

The desktop adapter exposes these as Tauri commands (`list_operation_presets`,
`get_operation_preset_request`, `plan_prepare_environment`,
`prepare_environment`, `get_workflow_status`, `cancel_workflow`). It returns the
application types directly; there is no parallel DTO that could drift.

## 8. Relationship to the Workflows screen

The Workflows screen targets one selection with fine-grained, per-operation
options. Bulk Operations is the compact, preset-driven view for everyday work:
it reuses the same planner and executor, so the two screens cannot disagree.
Use Workflows when you need the detailed phase view or a specific workflow
history; use Bulk Operations when you want to sync, build, migrate, restart, and
verify a familiar set in one action.

## 9. What Bulk Operations does not do

- It does not add new build, migration, process, or Git logic.
- It does not install or publish libraries beyond the existing
  `build_libraries` phase; library verification and version alignment stay on
  the Libraries screen.
- It does not start a workflow on load, retry after failure, or roll back a
  partially completed run.
- It does not bypass destructive-operation policy, even for a saved preset.

# Environment Workflows

Environment preparation is the end-to-end developer workflow: take a set of
services, make their repositories current, prepare the libraries they need, apply
their database migrations, build them, start them, and wait for readiness — as a
single, dependency-aware workflow.

It is implemented once in the **application core** and reused by every interface.
The TUI, the MCP server, and the shared local daemon all observe the same
authoritative workflow state (`docs/architecture.md`, `docs/local-daemon.md`,
`docs/adr/016-environment-workflows.md`).

---

## 1. Architecture

```
                 PrepareEnvironmentRequest
                            │
                            ▼
              ┌─────────────────────────────┐
              │        Application Core      │
              │  plan_prepare_environment    │  read-only
              │  prepare_environment         │  schedules a workflow
              │  get_workflow_status         │  aggregated status
              │  cancel_workflow             │  cooperative cancel
              └──────────────┬──────────────┘
                             │ one WorkflowBuilder
                             ▼
              ┌─────────────────────────────┐
              │          Task Engine         │
              │  dependency graph, scheduler │
              │  concurrency, cancellation   │
              └──────────────┬──────────────┘
                             │ tasks / steps
                             ▼
       Git · Library · Migration · Build · Service Runtime · Health
```

Key properties:

- **One composer.** Planning and execution are produced by the *same*
  composition function, so a plan cannot drift from what executes. The only
  difference is the `EnvironmentMode` (`Plan` vs `Execute`), which changes
  authorization behavior only.
- **No second scheduler.** The workflow is submitted to the existing Task
  Engine, which owns dependencies, concurrency limits, retries, timeouts,
  cancellation, and events (`docs/task-engine.md`). Nothing in the application
  layer runs a Git, Maven, npm, Liquibase, or process command.
- **No second core.** The workflow is a normal application use case: the TUI and
  MCP reach it through the shared `OrchestratorClient`, so the daemon owns all
  runtime state.

---

## 2. The request

`PrepareEnvironmentRequest` is the structured input. Every option has a
documented, safe default.

| Field | Default | Meaning |
| --- | --- | --- |
| `workspace_id` | — | The workspace to prepare. |
| `services` | `[]` | Explicit service selection. Empty falls through to profile → group → whole workspace. |
| `profile` | `None` | Select the services of a development profile. |
| `group` | `None` | Select the members of a service group. |
| `branch` | `None` | A branch to check out before building. `None` keeps each repository's configured branch. |
| `sync` | `true` | Fetch, optionally check out, and fast-forward each repository. |
| `include_dependencies` | `true` | Add the transitive closure of required service and library dependencies. |
| `build_libraries` | `false` | Build the required libraries. |
| `run_liquibase` | `false` | Apply migrations for migration-enabled services. **A database mutation.** |
| `build` | `false` | Build the selected services. |
| `start` | `false` | Start the selected services. |
| `wait_for_health` | `true` | Run configured health checks after starting (only meaningful when `start` is set). |

Defaults are deliberately conservative: a plain request **synchronizes Git and
nothing else**. Database mutations (`run_liquibase`) and process startup
(`start`) are opt-in, and no default performs a destructive Git operation.

Builder convenience: `build()` enables both library and service builds (the
common case); `build_libraries()` builds libraries without building services.

### Selection precedence

1. Explicit `services`, if non-empty.
2. Otherwise `profile`, if set.
3. Otherwise `group`, if set.
4. Otherwise every service in the workspace.

A profile or group that resolves to **no** services yields an empty selection —
it does not silently fall back to the whole workspace. An empty selection is
reported as “no services were selected to prepare”, and the workflow is a no-op
(`AlreadyInDesiredState`).

---

## 3. Dependency resolution

Before execution the selection is expanded into a closure:

- **Selected services** — what the caller asked for.
- **Included dependencies** — services and libraries pulled in because a
  selected service requires them (`include_dependencies = true`).
- **Libraries** — every required library, in dependency order.
- **Runtime prerequisites** — service startup is ordered after the services a
  service depends on.

The closure reuses the existing workspace dependency graph; there is no
competing graph implementation. Services are ordered **dependency-first** with a
Kahn topological sort (deterministic, because the frontier is a `BTreeSet`). A
cycle is reported as `ServiceDependencyCycle` rather than producing an arbitrary
order.

Required service-to-service edges order both **builds** and **starts**;
service-to-library edges order **builds** after the library build.

---

## 4. Planning

`plan_prepare_environment` is read-only and requires only `Permission::Read`.

The plan (`EnvironmentPlan`) reports:

- the workspace and the target branch (if any);
- the selected services;
- every resolved service, and which of them were included automatically;
- the required libraries, in order;
- the phases and steps, in execution order;
- the total step count;
- `requires_destructive_authorization` — `true` when any migration targets a
  non-local database;
- non-fatal **warnings** (see §5).

Planning **never** mutates repositories, databases, or processes, and it never
touches the filesystem through Git. Because it is computed from the same
composer as execution, the plan is the authoritative description of what would
run.

```
$ service-orchestrator mcp   # then:
plan_prepare_environment { "workspaceId": "project-x", "serviceIds": ["payment-service"],
                           "build": true, "runLiquibase": true, "start": true }
```

```json
{
  "workspace_id": "project-x",
  "target_branch": null,
  "selected_services": ["payment-service"],
  "resolved_services": ["auth-service", "payment-service"],
  "included_dependencies": ["auth-service"],
  "required_libraries": ["common-core"],
  "phases": [
    { "kind": "git_sync",        "steps": [ … ] },
    { "kind": "library_prepare", "steps": [ … ] },
    { "kind": "migration",       "steps": [ … ] },
    { "kind": "service_build",   "steps": [ … ] },
    { "kind": "service_start",   "steps": [ … ] },
    { "kind": "health_check",    "steps": [ … ] }
  ],
  "step_count": 9,
  "requires_destructive_authorization": false,
  "warnings": []
}
```

---

## 5. Warnings

The plan is explicit about work it will *not* do, rather than silently skipping:

- a migration-enabled phase requested for a service whose migrations are
  disabled (`run_liquibase` set, service not configured) — the service is
  **skipped explicitly**;
- a library that has no build command when `build_libraries` is set;
- a service that cannot be started when `start` is set (no run capability or no
  start command);
- a service that is **already running** — it is not restarted.

A service that is `process.is_active()` but not `Running` (for example
`Starting` or `Stopping`) is an **error**, not a warning: the environment is in an
ambiguous state and the workflow refuses to race it.

---

## 6. Execution phases

The workflow is a dependency graph, not a fixed pipeline. Independent steps in a
phase run concurrently, bounded by the engine's concurrency limit
(`execution.max_parallel_tasks`). Actual ordering is always determined by
dependencies.

1. **Git synchronization.** For every resolved service and library with a
   repository: a `GitSync` step (fetch + fast-forward). If a branch was
   requested, a `GitCheckout` step depends on the sync. Repositories are
   synchronized independently and therefore in parallel.
2. **Library preparation.** `BuildLibrary` for each required library, depending
   on its sync/checkout and on any library it depends on.
3. **Migrations.** `Liquibase` (`MigrationApply`) for each migration-enabled
   service, depending on its own sync/checkout and on the libraries it needs. A
   migration failure blocks the build and start of that service.
4. **Service builds.** `BuildService` for each buildable service, depending on
   its sync/checkout, its migration step (if any), the libraries it needs, and
   the builds of the services it depends on.
5. **Startup.** `StartService` for each startable service, depending on its
   build step (if any), its migration step (if any), and — for required runtime
   dependencies — the startup of the services it depends on.
6. **Health checks.** `HealthCheck` for each started service that declares health
   checks, depending on the corresponding `StartService` step.

### Git synchronization

The Git Manager is reused (`docs/git-manager.md`). The workflow:
fetches, checks out the configured target branch (never a hard-coded `master`),
and fast-forwards. **Local changes are never discarded**: the workflow schedules
no `reset`, no `clean`, and no forced checkout. If a repository cannot
fast-forward safely, that step fails and the failure propagates.

### Library preparation

The Library Manager is reused (`docs/library-manager.md`). Builds use the
configured build command and verify the artifact via the existing build target.
The workflow **never rewrites a POM** to change a version; a version mismatch is
reported by the dedicated `inspect_library_version_alignment` query, not
silently fixed. When `sync = false` (local library development mode), no
checkout is scheduled, so a developer's local library working tree is left
untouched.

### Migrations

The Migration Manager is reused (`docs/migration-manager.md`). Migrations run
only for services whose Liquibase configuration is `enabled`. The database target
and its safety classification are resolved exactly as for a direct
`apply_migration`: a non-local target requires `Permission::Destructive`.
Migration tasks are **never retried automatically** — applying a migration is a
database mutation that may be partially applied. Migration steps respect the
configured timeout and the existing migration coordination lock.

### Builds

The Build Manager is reused (`docs/build-manager.md`). Service builds depend on
their migrations so a schema change lands before the application is built, and on
the builds of services they depend on.

### Startup and health

The Service Runtime is reused (`docs/service-runtime.md`). The workflow never
spawns a process directly. A `StartService` **step** is distinct from the
long-running service **lifecycle**: the step completes when the process has been
started and registered; the process then runs independently of the workflow.
Readiness is bounded by the configured health-check timeouts — the workflow does
not wait forever.

Each of these steps carries its resolved service target as task parameters (the
owning workspace, the service id, the working directory, and the workspace
root), because the lifecycle and health executors decode their target from the
parameters exactly as the standalone operations do. The in-workflow health step
therefore schedules the same `HealthCheck` task a manual health check does.

---

## 7. Cancellation and failure handling

**Failure policy.** The engine's default dependency policy is
`AllSucceeded`: when a step fails or is skipped, its dependents settle as
**skipped**, transitively, so a failure never leaves a downstream step blocked
forever. A migration failure therefore blocks the dependent build and start.

**Cancellation.** `cancel_workflow` cancels every still-active task in a
workflow:

- queued tasks transition `queued → cancelled` without occupying a slot;
- running tasks are asked to cancel cooperatively;
- completed results stand;
- dependents of a cancelled task settle as skipped.

Cancellation is a `SafeWrite` policy operation with the same authorization as any
other write. It never stops a service the workflow did not start, and performs no
destructive cleanup. Cancelling a workflow whose tasks are all terminal is a
no-op and reports `already_complete`.

---

## 8. Workflow state and aggregated results

`get_workflow_status` returns a `WorkflowStatus` correlated by workflow id:

- `state` — `queued`, `running`, `succeeded`, `partially_succeeded`, `failed`,
  or `cancelled`, classified from the task counts;
- counts — `total`, `succeeded`, `failed`, `cancelled`, `skipped`, `pending`;
- timestamps — first start and last finish (displayed in the caller's timezone);
- per-phase progress — `total`, `succeeded`, `failed`, `pending` for each phase;
- structured `failures` — task id, kind, service or library, and a stable
  `code` + `message`. Raw logs are never embedded; a failure is described
  structurally and the detailed log is fetched separately.

---

## 9. Revalidation, idempotency, and coordination

- **Plan freshness.** A plan is a snapshot. Execution **revalidates every
  precondition** — the workspace, the services, capabilities, configured
  commands, migration enablement, database targets, and current process state —
  immediately before scheduling. A plan is not authorization to execute it.
- **Idempotency.** The workflow skips work that is already done where a reliable
  state check exists: already-running services are not restarted; repositories
  converge to the configured branch (re-syncing an up-to-date repository is
  cheap); Liquibase applies only pending changesets; library installs and builds
  are repeatable through their providers.
- **Operation coordination.** All existing locks are preserved — one Git
  operation per repository, one migration per service, one process per service —
  so an environment workflow never races a concurrent direct operation from
  another client of the daemon.

---

## 10. TUI usage

From the Dashboard or Services screen, press **`P`** to open *Prepare
environment — depth*:

```
Prepare environment — depth
  Synchronize repositories only (safe default)
  Synchronize and build required libraries
  Synchronize and build libraries and services
  Full: sync, migrate, build, start, health
```

Choose a depth, then optionally enter a branch (leave it empty to keep each
repository's configured branch). The TUI calls the **same** application workflow
plan and shows it for review:

```
Review environment plan
  Workspace: project-x
  Selected services: payment-service
  Resolved services: auth-service, payment-service
  Required libraries: common-core
  Branch: master
  Planned steps: 9
    git-sync         3 step(s)
    library-prepare  1 step(s)
    migration        1 step(s)
    service-build    2 step(s)
    service-start    2 step(s)
    health-check     1 step(s)

  [Enter] Run   [Esc] Cancel
```

If a step requires destructive authorization (a non-local migration), the review
says so, and confirming runs the workflow through the trusted interactive
authorization path — there is no `confirmed: true` flag. Progress and failures
are observed on the **Tasks** screen (the workflow shares the engine with every
other task). On the Tasks screen, **`W`** opens a chooser to cancel an active
workflow.

The preview is built from the real `EnvironmentPlan`; the UI hard-codes no
workflow steps.

---

## 11. MCP usage

The same workflow is exposed through thin MCP tools (`docs/mcp.md`):

| Tool | Class | Purpose |
| --- | --- | --- |
| `plan_prepare_environment` | read-only | Return the resolved plan without changing anything. |
| `prepare_environment` | write | Schedule the workflow; returns a `workflowId`. |
| `get_workflow_status` | read-only | Aggregated state, per-phase progress, structured failures. |
| `cancel_workflow` | write | Cancel every still-active task in a workflow. |

`prepare_environment` accepts the full request (selection, profile, group,
branch, `sync`, `includeDependencies`, `buildLibraries`, `runLiquibase`,
`build`, `start`, `waitForHealth`) and returns the workflow id immediately.
Long-running execution is always queryable.

An agent preparing a payment environment:

```
list_profiles
  → plan_prepare_environment { profile: "payment-development", branch: "master",
                               build: true, runLiquibase: true, start: true }
  → review the plan (resolved services, required libraries, authorization)
  → prepare_environment { … }
  → get_workflow_status { workflowId }
  → inspect aggregated results; fetch logs only on failure
```

MCP is a non-interactive client: it never requests destructive authorization, so
a plan that mutates a non-local database is denied with `CONFIRMATION_REQUIRED`
at execution time — a human must approve it interactively. Planning still
*succeeds* and reports `requires_destructive_authorization: true`, so the agent
can surface the need for confirmation.

---

## 12. Desktop GUI usage

The desktop GUI (`docs/desktop-gui.md` §27) consumes the same workflow through
thin Tauri commands that map one-to-one onto the operations above:

| Command | Maps to |
| --- | --- |
| `list_profiles` / `get_profile` | the profile reads behind profile selection |
| `list_groups` / `get_group` | the group reads behind group selection |
| `plan_prepare_environment` | the read-only plan |
| `prepare_environment` | scheduling (returns a workflow id) |
| `get_workflow_status` | aggregated state, per-phase progress, structured failures |
| `list_workflows` | a bounded, workspace-scoped history window |
| `cancel_workflow` (existing) | cancellation of every still-active task |

The screen selects exactly one source (explicit services, a profile, or a
group), configures the request with **safe defaults** (sync on, dependencies on,
health wait on; builds, migrations, startup, and branch off), previews the plan,
executes it, and then monitors and cancels it through the same engine operations
the TUI and MCP use. Planning writes nothing, and the daemon re-validates and
re-authorizes at execution time — a frontend confirmation is never treated as
authorization.

Because the daemon keeps no persisted workflow table, `list_workflows` is
derived: the adapter groups the engine's active and retained tasks by workflow
id and reads each workflow's workspace from the resolved parameters of its
steps, so history is bounded and disappears with the daemon's in-memory state.

---

## 13. Example workflows

**Backend development.** `{ workspaceId, profile: "backend-development",
build: true, runLiquibase: true, start: true }` — sync every backend repository,
build shared libraries, apply local migrations, build and start the services, and
wait for actuator health.

**Frontend development.** `{ workspaceId, serviceIds: ["web-ui"], build: true,
start: true }` — sync `web-ui`, build it, and start it. Its required
`auth-service` dependency is pulled in automatically and started first.

**Sync only (Scenario A).** `{ workspaceId, branch: "master" }` — synchronize
every repository to `master` and stop. No builds, no migrations, no startup.

**Local library development.** `{ workspaceId, serviceIds: ["auth-service"],
sync: false, build_libraries: true, build: true, start: true }` — work against a
locally modified library without checking it out, then build and run the
consuming service.

---

## 14. Related documents

- `docs/adr/016-environment-workflows.md` — the decision record.
- `docs/application-core.md` — the application core and its request/response
  models.
- `docs/task-engine.md` — dependencies, concurrency, cancellation, retry.
- `docs/local-daemon.md` — how TUI and MCP share one authoritative workflow.
- `docs/git-manager.md`, `docs/library-manager.md`, `docs/migration-manager.md`,
  `docs/build-manager.md`, `docs/service-runtime.md`, `docs/health-manager.md` —
  the capabilities the workflow orchestrates.

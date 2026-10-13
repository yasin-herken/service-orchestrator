# Workflows and Profiles

An environment **workflow** takes a set of services and prepares them end to end:
synchronize their repositories, build the libraries they need, apply their
migrations, build them, start them, and wait for readiness — as one
dependency-aware unit. **Profiles** and **groups** name reusable selections.

The workflow is implemented once in the application core and reused by every
interface. The full model — request fields, phases, warnings, and semantics — is
in [`environment-workflows.md`](environment-workflows.md); this guide is the
practical tour.

## 1. Selecting services

A request selects exactly one source:

1. **Explicit services** (`services`), if non-empty;
2. otherwise a **profile**;
3. otherwise a **group**;
4. otherwise every service in the workspace.

A profile or group that resolves to **no** services yields an empty selection and
a no-op (`already_in_desired_state`); it does not silently fall back to the whole
workspace.

## 2. Profiles and groups

**Groups** are named sets of services:

```toml
[[groups]]
id = "customer"
name = "Customer Management"
services = ["auth-service", "web-ui"]
```

**Profiles** are commonly used developer environments, referencing services and
groups:

```toml
[[profiles]]
id = "full-stack"
name = "Full Stack"
services = ["auth-service"]
groups = ["customer"]
```

Both are validated: a group must contain at least one service, a profile must
reference at least one entity, and every reference must resolve in the same
workspace. Actions: `start_group`/`stop_group` and `start_profile`/`stop_profile`
start or stop the members; a profile/group can also be used as a workflow
selection.

## 3. Options

| Option | Default | Meaning |
| --- | --- | --- |
| `sync` | `true` | Fetch and fast-forward each repository. |
| `includeDependencies` | `true` | Add required service and library dependencies. |
| `buildLibraries` | `false` | Build required libraries. |
| `runLiquibase` | `false` | Apply migrations (a **database mutation**). |
| `build` | `false` | Build the selected services. |
| `start` | `false` | Start the selected services. |
| `waitForHealth` | `true` | Run health checks after starting. |
| `branch` | none | Check out this branch before building. |

Defaults are deliberately conservative: a plain request **synchronizes Git and
nothing else**. Database mutation and process startup are opt-in.

## 4. Dependency resolution

Before execution, the selection is expanded into a closure:

- **Included dependencies** — services/libraries pulled in because a selected
  service requires them.
- **Libraries** — every required library, in dependency order.
- **Ordering** — services are ordered dependency-first (a deterministic
  topological sort). A cycle is reported as `ServiceDependencyCycle`.

Required service-to-service edges order both builds and starts; service-to-library
edges order builds after the library build.

## 5. Plan, then execute

`plan_prepare_environment` is **read-only** and requires only `Permission::Read`.
Plan before you execute: the plan is computed by the *same* composer that
executes, so it cannot drift from what will run.

The plan reports the workspace and target branch, the selected and resolved
services, the automatically included dependencies, the required libraries in
order, the phases and steps, the total step count,
`requires_destructive_authorization`, and non-fatal warnings (for example, a
service whose migrations are disabled, or a service already running).

### Phases

The workflow is a dependency graph, not a fixed pipeline. Independent steps run
concurrently, bounded by `execution.max_parallel_tasks`.

1. **Git synchronization** — fetch + fast-forward; an optional checkout.
2. **Library preparation** — build required libraries in dependency order.
3. **Migrations** — apply for migration-enabled services only.
4. **Service builds** — after migrations and dependency builds.
5. **Startup** — after builds and required dependencies are running.
6. **Health checks** — after startup.

Plus a leading **Validate** phase.

### Interface usage

- **GUI** — the **Workflows** screen: select by Services/Profile/Group, set
  options (safe defaults), **Preview plan**, **Run workflow**, watch live
  progress, cancel, and review results. History is a bounded, workspace-scoped
  window.
- **TUI** — press `P` (Dashboard or Services) to choose a depth
  (*synchronize only*, *+ libraries*, *+ services*, *full*), optionally enter a
  branch, review the plan, and confirm.
- **MCP** — `plan_prepare_environment` → review → `prepare_environment` (returns a
  `workflowId`) → poll `get_workflow_status` → `cancel_workflow` if needed.

## 6. Cancellation and partial failures

- `cancel_workflow` cancels every still-active task: queued tasks transition to
  `cancelled`, running tasks are asked to cancel cooperatively, completed results
  stand, and dependents settle as **skipped**.
- Cancellation never stops a service the workflow did not start and performs no
  destructive cleanup. Cancelling a completed workflow is a no-op.
- A workflow that partly succeeds is reported as `partially_succeeded`, never as
  a success. Failures are structured — task id, kind, service/library, and a
  stable `code` + `message` — with raw logs fetched separately.
- The default dependency policy is `AllSucceeded`: a failed step causes its
  dependents to be skipped, so a failure never leaves a downstream step blocked.

## 7. Authorization

- The workflow is `SafeWrite` under normal authorization.
- A migration targeting a **non-local** database makes the workflow require
  **destructive** authorization. Planning still succeeds and reports
  `requires_destructive_authorization: true`.
- **MCP cannot self-authorize**: a workflow that would mutate a non-local
  database is denied with `CONFIRMATION_REQUIRED` at execution; a human must run
  it interactively. See [`security.md`](security.md) §3.

## 8. A complete example

Backend development, applying local migrations, building, starting, and waiting
for health:

```text
plan_prepare_environment {
  "workspaceId": "acme",
  "profile": "backend-development",
  "build": true,
  "runLiquibase": true,
  "start": true
}
→ review resolved_services, required_libraries, phases, warnings

prepare_environment { …same request… }
→ { "workflowId": "7" }

get_workflow_status { "workflowId": "7" }
→ state: running → succeeded | partially_succeeded | failed
```

Other useful shapes:

- **Sync only:** `{ workspaceId, branch: "main" }`.
- **Frontend:** `{ workspaceId, services: ["web-ui"], build: true, start: true }`
  — its `auth-service` dependency is pulled in and started first.
- **Local library development:**
  `{ workspaceId, services: ["auth-service"], sync: false, buildLibraries: true, build: true, start: true }`.

## 9. Revalidation and idempotency

- A plan is a **snapshot**. Execution revalidates every precondition (workspace,
  services, capabilities, commands, migration enablement, targets, process state)
  before scheduling. A plan is not authorization to execute.
- The workflow skips work that is already done where a reliable check exists:
  already-running services are not restarted, an up-to-date repository re-syncs
  cheaply, and Liquibase applies only pending changesets.
- Existing locks are preserved (one Git operation per repository, one migration
  per service, one process per service), so a workflow never races a concurrent
  direct operation.

## 10. Limitations

- Workflow history is **in-memory and derived**; it is lost on a daemon restart
  ([`known-limitations.md`](known-limitations.md)).
- The **GUI** consumes profiles/groups but does not create or edit them — use the
  **Configuration** screen for that.
- Composed workflow failures are identified by **task**, not by entity.

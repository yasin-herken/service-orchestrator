# Application Core

The `service-orchestrator-application` crate is the shared application core. It
is the **single API** every interface calls — the Ratatui TUI, the MCP server,
and any future CLI or HTTP API — so that business behavior lives in exactly one
place.

```text
Human developer              AI agent
      │                          │
      ▼                          ▼
    tui                        mcp
      │                          │
      └────────────┬─────────────┘
                   ▼
          ┌─────────────────┐
          │  Application    │   ← this crate
          │  core           │
          └────────┬────────┘
                   │
     ┌─────────────┼─────────────┐
     ▼             ▼             ▼
 execution      policy       ports (runtime state, logs)
     │
     ▼
  domain
```

The application core does **not** depend on `ratatui`, `rmcp`, or any
interface type, and it never executes infrastructure directly. It validates
preconditions, applies policy, describes work as tasks/workflows, and submits
that work to the task engine.

---

## 1. Responsibilities

- **Expose use cases.** One method per operation (`start_service`,
  `prepare_environment`, `get_service_status`, ...).
- **Validate preconditions.** Resolve entities, check configuration, and reject
  operations that cannot proceed before any work is scheduled.
- **Apply policy.** Every mutating command passes through the policy engine
  first; destructive operations require explicit authorization.
- **Describe work.** Build `TaskDefinition` / `WorkflowDefinition` values and
  submit them to the task engine.
- **Expose structured state.** Queries return application/domain values, never
  pre-formatted strings.
- **Own the ports.** Boundaries that the application needs but does not own
  (runtime state, logs) are defined here; infrastructure implements them later.

It does **not** schedule tasks, run Git/Maven/npm/Liquibase, spawn processes,
perform health checks, parse configuration, or render anything.

---

## 2. The `Application` container

`Application` is a cloneable struct holding the already-validated `Config` and
its boundaries:

```rust
pub struct Application {
    config: Config,
    tasks: Arc<dyn TaskManager>,
    runtime: Arc<dyn RuntimeStateStore>,
    logs: Arc<dyn LogService>,
    policy: PolicyEngine,
    authorization: Authorization,
}
```

Dependencies are injected through the constructor; there is no framework:

```rust
Application::new(config, task_manager, runtime_store, log_service)
```

Because the boundaries are shared behind `Arc`, the TUI and MCP adapters can
each hold an `Application` that observes the same runtime state and submits to
the same task engine. `with_policy` and `with_authorization` replace the policy
engine and the caller's authorization.

### Why an application core exists

Without it, each interface would re-implement preconditions, task construction,
and policy. Behavior would drift, destructive operations could be bypassed, and
every capability would have to be built twice. The core is the one source of
truth (see `docs/adr/004-application-core.md`).

---

## 3. Commands and queries

Use cases are split conceptually (not into a distributed CQRS system):

- **Queries** are side-effect free. They read validated configuration and last
  observed runtime state. They never fetch, check out, build, probe, or start
  anything.
- **Commands** validate preconditions, apply policy, build a task/workflow
  definition, and submit it. They never execute infrastructure directly.

### Queries

| Query | Input | Result |
| --- | --- | --- |
| `list_workspaces` | — | `Vec<Workspace>` |
| `get_workspace` | `WorkspaceId` | `Workspace` |
| `get_workspace_status` | `WorkspaceId` | `WorkspaceStatus` |
| `list_services` | `WorkspaceId` | `Vec<Service>` |
| `get_service` | `ServiceId` | `Service` |
| `get_service_status` | `ServiceId` | `ServiceStatus` |
| `list_libraries` / `get_library` | `WorkspaceId` / `LibraryId` | `Vec<Library>` / `Library` |
| `list_groups` / `get_group` | `WorkspaceId` / `GroupId` | `Vec<ServiceGroup>` / `ServiceGroup` |
| `list_profiles` / `get_profile` | `WorkspaceId` / `ProfileId` | `Vec<Profile>` / `Profile` |
| `get_task` | `TaskId` | `Task` |
| `get_workflow` | `WorkflowId` | `Workflow` |
| `get_logs` | `LogRequest` | `Vec<LogEntry>` |

### Commands

| Command | Input | Result |
| --- | --- | --- |
| `sync_service` | `ServiceId` | `OperationResult` |
| `build_service` | `ServiceId` | `OperationResult` |
| `build_library` | `LibraryId` | `OperationResult` |
| `run_liquibase` | `ServiceId` | `OperationResult` |
| `start_service` | `ServiceId` | `OperationResult` |
| `stop_service` | `ServiceId` | `OperationResult` |
| `restart_service` | `ServiceId` | `OperationResult` |
| `health_check` | `ServiceId` | `OperationResult` |
| `sync_workspace` | `WorkspaceId` | `OperationResult` |
| `build_workspace` | `WorkspaceId` | `OperationResult` |
| `prepare_environment` | `PrepareEnvironmentRequest` | `OperationResult` |
| `start_group` / `stop_group` | `WorkspaceId`, `GroupId` | `OperationResult` |
| `start_profile` / `stop_profile` | `WorkspaceId`, `ProfileId` | `OperationResult` |
| `cancel_task` | `TaskId` | `()` |

Commands return quickly. Long-running work is represented by the returned task
or workflow identifier rather than blocking the interface.

---

## 4. Request and response models

All result types are structured and derive `serde` so interfaces can render or
serialize them independently.

### `OperationResult`

```rust
pub enum OperationResult {
    TaskScheduled { task_id: TaskId, kind: TaskKind },
    WorkflowScheduled { workflow_id: WorkflowId, kind: WorkflowKind },
    AlreadyInDesiredState { kind: TaskKind },
}
```

`AlreadyInDesiredState` is how idempotency is expressed: a command that finds
the target already in the requested state returns it instead of scheduling
duplicate work.

### `ServiceStatus` / `WorkspaceStatus`

`ServiceStatus` combines configuration (id, name, kind, ports, capabilities)
with the last observed runtime state (`process_state`, `health_state`,
`git_state`, `branch`). `WorkspaceStatus` aggregates the status of every
service in a workspace. Neither contains formatted text.

### `PrepareEnvironmentRequest`

A structured request object rather than a long positional argument list:

```rust
pub struct PrepareEnvironmentRequest {
    pub workspace_id: WorkspaceId,
    pub services: Vec<ServiceId>,   // empty = every service in the workspace
    pub branch: Option<Branch>,
    pub run_liquibase: bool,
    pub build: bool,
    pub start: bool,
}
```

Builders (`with_services`, `with_branch`, `build`, `run_liquibase`, `start`)
keep call sites readable.

---

## 5. Task submission

The application requests work; the task engine executes it. The boundary
contract lives in the `execution` crate (`TaskManager`), so the engine owns its
API and there is no dependency back-edge from `execution` to `application`.

```text
StartService
      │ validate + policy
      ▼
TaskDefinition { kind: StartService, target: Service(auth-service), timeout }
      │
      ▼
TaskManager::submit ──> TaskId
```

A composed operation is submitted as a `WorkflowDefinition`: the application
builds steps with their prerequisites (inside a private `WorkflowBuilder`) and
the engine schedules them. Dependency information is preserved as step
prerequisites, but dependency-aware scheduling, concurrency, retry, and timeout
enforcement remain the engine's responsibility (`docs/adr/004-application-core.md`).

Workflows are produced by:

- `sync_workspace` → `WorkflowKind::SyncWorkspace`
- `build_workspace` → `WorkflowKind::BuildWorkspace`
- `prepare_environment` → `WorkflowKind::PrepareEnvironment`
- `start_profile` / `stop_profile` → `WorkflowKind::StartProfile` / `StopProfile`
- `start_group` / `stop_group` → `WorkflowKind::Custom("start_group" | "stop_group")`

`prepare_environment` wires synchronization, an optional checkout, library and
service builds, Liquibase, start, and health-check steps into one
dependency-ordered workflow.

---

## 6. Policy checks

Every mutating command begins with a policy check:

```rust
self.authorize(Permission::SafeWrite)?;
```

`Application::authorize` runs the pure `PolicyEngine` (from the `policy` crate)
against the caller's `Authorization`:

- `Read` and `SafeWrite` are always allowed;
- `Destructive` requires `Authorization::allow_destructive()`, typically after
  a user has confirmed the action.

A denied check becomes `ApplicationError::PermissionDenied`. The check lives
*inside* the use case, so neither the TUI nor MCP can bypass it by calling an
operation directly. Interfaces may also call `authorize` themselves to decide
whether to prompt for confirmation before retrying.

`health_check` is classified `Read` (it observes state), while starting,
stopping, building, synchronizing, and migrating are `SafeWrite`.

---

## 7. Runtime state

Configuration and runtime state are deliberately separate:

- **Persistent state** — workspaces, services, libraries, groups, profiles,
  commands, dependencies, ports, health-check definitions. This is the
  validated `Config` loaded once; the application never parses files.
- **Runtime state** — last observed process, health, and Git state, plus the
  checked-out branch. This changes constantly and lives in a focused
  `RuntimeStateStore` port.

The MVP implementation, `InMemoryRuntimeStateStore`, is dependency-free and
does not survive a restart. There is no database. The port keeps a future,
process-manager-backed store local to infrastructure.

Commands optimistically transition runtime state after submitting work (for
example, marking a service `Starting`), while queries only *read* the last
observed state. A service whose process is running is not automatically
healthy: process and health are independent.

---

## 8. Ports

Ports are declared only where the application currently needs a boundary.

| Port | Purpose | MVP implementation |
| --- | --- | --- |
| `TaskManager` (in `execution`) | Submit and observe tasks/workflows. | Implemented by the future engine; fakes in tests. |
| `RuntimeStateStore` | Read/write transient service runtime state. | `InMemoryRuntimeStateStore`. |
| `LogService` | Fetch bounded, structured service logs. | `NullLogService` (returns nothing). |

There is deliberately **no** `GitService`, `BuildService`, `ProcessService`,
`LiquibaseService`, or `HealthService` trait yet: the application does not call
those subsystems directly. It requests tasks, and the execution engine reaches
the adapters. Ports are added when a real boundary appears, not because a noun
exists (`AGENTS.md` §25).

---

## 9. Error strategy

`ApplicationError` is one typed enum, independent of any interface. Variants
fall into three groups:

- **not found** — `WorkspaceNotFound`, `ServiceNotFound`, `LibraryNotFound`,
  `ProfileNotFound`, `GroupNotFound`, `TaskNotFound`, `WorkflowNotFound`;
- **precondition / validation** — `InvalidOperation`, `DependencyNotFound`,
  `ConfigurationInvalid`, `WorkflowInvalid`;
- **boundary** — `PermissionDenied`, `TaskManager`, `LogService`.

Boundary errors preserve context (the engine's message) without leaking
scheduler internals, and there is no enormous catch-all variant. Interfaces map
the enum into their own response shapes.

---

## 10. Preconditions and idempotency

Commands validate before scheduling:

- **`start_service`** — service and workspace exist, dependencies resolve, the
  service declares `Run`, a start command exists, and the service is not
  already running or transitioning. A running service returns
  `AlreadyInDesiredState`; a transitioning one is a precondition error.
- **`stop_service`** — a service that is not running returns
  `AlreadyInDesiredState`.
- **`build_service`** — declares `Build`, has a build command and a path.
- **`sync_service`** — has a repository.
- **`run_liquibase`** — Liquibase is configured, enabled, and has a command.
- **`health_check`** — has at least one configured health check.
- **`start_group` / `start_profile`** — every selected service can be started;
  already-running services are skipped, and if nothing remains the result is
  `AlreadyInDesiredState`.
- **`prepare_environment`** — the workspace and selected services exist, and
  each requested phase is valid for every selected service.

---

## 11. Why TUI and MCP stay outside the core

The TUI and MCP answer the same questions over different transports. Keeping
both outside the core means:

- behavior, policy, and error semantics are identical for humans and agents;
- a new capability is implemented once, in the core;
- the core can be tested without a terminal or an MCP client, using fakes for
  the task engine and log service.

The TUI reads application state, renders it, and turns input into application
calls. The MCP server validates tool input, applies the same policy, and
forwards to the same use cases. Neither contains Git, build, process, or
workflow logic.

---

## 12. Tests

The application crate is covered by:

- unit tests inside `ports/` (runtime-state store) and integration tests in
  `crates/application/tests/application_core.rs`;
- a fake `TaskManager` that records submitted `TaskDefinition`s and
  `WorkflowDefinition`s instead of executing them;
- the real validated configuration loaded from an in-memory TOML fixture, so
  config validation is exercised;
- the real `InMemoryRuntimeStateStore`, so optimistic state transitions are
  verified;
- `NullLogService` as a no-op log boundary.

The suite covers service listing, unknown entities, workspace status, start/stop
preconditions and idempotency, build and Liquibase validation, policy
allow/deny, task-manager error mapping, and the exact task/workflow definitions
submitted. **No test runs Maven, npm, Git, Liquibase, or a process.**

---

## 13. Related documents

- `docs/adr/004-application-core.md` — the decision record for this layer.
- `docs/architecture.md` — the overall system architecture.
- `docs/configuration.md` — the validated `Config` the application consumes.
- `docs/domain-model.md` — the domain vocabulary.
- `AGENTS.md` — full engineering guidance.

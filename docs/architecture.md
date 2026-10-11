# Architecture

Service Orchestrator is a macOS-native developer environment orchestration
platform written in Rust. It prepares and manages local multi-repository,
multi-service development environments: synchronizing repositories, checking
out branches, building shared libraries, running Liquibase migrations, building
and starting applications, inspecting logs, running health checks, and
executing dependency-aware workflows.

It exposes the same capabilities through two interfaces:

- a Ratatui terminal UI for human developers, and
- an MCP server for AI agents.

Both interfaces are adapters over one shared application core.

---

## 1. Product purpose

Reduce the repetitive work of preparing a developer environment. A developer
should be able to select services, synchronize repositories, build libraries,
migrate databases, build and start applications, inspect logs, and run health
checks without remembering the exact `git`/`mvn`/`npm`/`liquibase` incantations
for each repository. AI agents should be able to do the same work through MCP.

The application is a **developer orchestration platform**, not an AI
application. AI is an optional interface.

---

## 2. The core architectural rule

> The application core must not depend on Ratatui or MCP. Both are interface
> adapters over the same application core.

There is exactly one source of truth for behavior:

```text
TUI -> application use case -> task engine -> infrastructure
MCP -> application use case -> task engine -> infrastructure
```

Business logic is never duplicated per interface.

---

## 3. Layers

From innermost to outermost:

1. **Domain** — pure business concepts and rules. No workspace dependencies.
2. **Core services** — the task/workflow engine and the policy engine. They
   depend only on the domain.
3. **Application** — use cases that coordinate the domain and the execution
   engine, plus the port traits that infrastructure implements. It consumes the
   validated `Config` produced by the configuration system.
4. **Infrastructure** — concrete adapters: Git, process management, build
   tooling, Liquibase, health checks, logging, and configuration loading.
5. **Interfaces** — TUI and MCP adapters.
6. **Composition root** — the `service-orchestrator` binary. It wires concrete
   infrastructure into the application core and hands the core to an interface.

### Architecture diagram

```text
                         Human developer            AI agent
                                |                      |
                                v                      v
                        +---------------+      +---------------+
   Interfaces           |      tui      |      |      mcp      |
                        +-------+-------+      +-------+-------+
                                |                      |
                                +----------+-----------+
                                           |
                                           v
                        +-------------------------------+       future:
   Application          |          application          |<----- cli, api
   (use cases + ports)  |  use cases / orchestration    |
                        +---+-----------+-----------+---+
                            |           |           |
              +-------------+           |           +--------------+
              |                         v                          |
              |              +---------------------+               |
   Core       |              |   execution engine  |               |
   services   |              | tasks / workflows   |               |
              |              +----------+----------+               |
              |                         |                          |
              |              +----------v----------+               |
              |              |    policy engine    |               |
              |              +----------+----------+               |
              |                         |                          |
              +-------------------------+--------------------------+
                                        |
                                        v
                        +-------------------------------+
   Domain               |            domain             |
   (no dependencies)    | concepts, rules, state types  |
                        +-------------------------------+
                                        ^
                                        | implements ports
                        +-------------------------------+
   Infrastructure       |  git  process  build  health   |
   (adapters)           |  liquibase        logging      |
                        +-------------------------------+
                                        ^
                                        | wired by
                        +-------------------------------+
   Composition root     |     service-orchestrator      |
                        +-------------------------------+
```

The important property of this diagram is that arrows point *inward* toward the
domain. Nothing inner knows about anything outer.

---

## 4. Crate responsibilities

| Crate | Layer | Responsibility |
| --- | --- | --- |
| `domain` | Domain | Pure business concepts and rules. Depends on no workspace crate. |
| `execution` | Core service | Task/workflow engine: state, dependency-aware scheduling, bounded concurrency, cancellation, retry, timeout, progress. |
| `policy` | Core service | Permission model and policy engine (`READ`, `SAFE_WRITE`, `DESTRUCTIVE`). |
| `config` | Infrastructure | Versioned configuration loading, parsing, validation, migration. Produces one validated `Config` for the application. |
| `application` | Application | The shared application core: use cases (commands and queries), orchestration, policy application, and the ports infrastructure implements (`RuntimeStateStore`, `LogService`). Consumes `Config` and submits work to the `execution` engine. |
| `git` | Infrastructure | Git adapter implementing the application Git port. |
| `process` | Infrastructure | Local process lifecycle, state, ports, child processes, logs. |
| `build` | Infrastructure | Build tooling (Maven, npm) via structured process execution. |
| `liquibase` | Infrastructure | Liquibase migration adapter. |
| `health` | Infrastructure | Configurable health checks (process, port, HTTP/actuator). |
| `logging` | Infrastructure | Structured logging and bounded, filterable log retrieval. |
| `mcp` | Interface | MCP stdio adapter over the application core. |
| `tui` | Interface | Ratatui adapter over the application core. |
| `service-orchestrator` (root) | Composition root | Wires infrastructure into the core and dispatches to an interface. |

---

## 5. Dependency direction

The intended direction, expressed as allowed Cargo dependencies:

```text
tui  ─────────┐
              │
mcp  ─────────┼──> application ──> execution ──> domain
              │            │    ├──> policy  ──> domain
future cli ───┘            │    └──> config  ──> domain
                           └──> domain

git, process, build, liquibase, health, logging
    ────────────────────────> application + domain   (implement ports)

service-orchestrator (root) ──> everything         (composition root)
```

### Rules

- `domain` depends on no workspace crate.
- `execution` and `policy` depend only on `domain`.
- `application` depends on `domain`, `execution`, `policy`, and `config`. It
  consumes the validated `Config`; it never parses configuration files itself.
- Infrastructure crates depend on `application` (to implement its ports) and
  `domain`. They must not depend on each other or on interfaces.
- `tui` and `mcp` depend only on `application`.
- The root binary is the only crate allowed to depend on both infrastructure
  and interfaces; it is the composition root.

### Forbidden edges

```text
domain      -> tui / mcp / any infrastructure
application -> ratatui / mcp / concrete infrastructure
tui         -> infrastructure (git, process, build, ...)
mcp         -> infrastructure (git, process, build, ...)
git         -> tui / mcp
```

These edges are not merely documentation: because every dependency is an
explicit workspace dependency, adding a forbidden edge requires editing a
`Cargo.toml` and is therefore visible in review. Cyclic dependencies are
rejected by Cargo at build time.

---

## 6. Why TUI and MCP are adapters

The TUI and the MCP server answer the same questions ("what services exist?",
"build this service", "start this profile") over different transports. If each
implemented its own version of those operations, behavior would drift and every
new capability would have to be built twice.

Instead:

- The TUI reads application state, renders it, and turns user input into
  application commands. It contains no Git, build, process, or workflow logic.
- The MCP server validates tool input, applies policy, and forwards calls to
  application use cases. It contains no business logic and never shells out
  directly.

Both therefore see identical behavior, identical policies, and identical error
semantics. This is also why the permission model lives in `policy` and is applied
by the application and interfaces rather than inside a specific UI.

---

## 7. Why the core must work without an LLM

Service Orchestrator is a developer orchestration platform. A developer must be
able to run:

```bash
service-orchestrator tui
```

with no LLM, no MCP client, no internet, and no remote backend.

The MCP server is optional:

```bash
service-orchestrator mcp
```

Keeping the core free of any AI dependency is what makes this possible, and it
keeps AI-assisted usage a thin, replaceable layer rather than a load-bearing
part of the system.

---

## 8. Where future interfaces fit

A future CLI or HTTP API is just another adapter:

```text
future cli ──> application
future api ──> application
```

It depends only on `application`, reuses the same use cases, and is wired in by
the composition root exactly like `tui` and `mcp`. No core change is required to
add an interface, and no interface may add business logic.

---

## 9. Where service executors belong

Service-specific behavior (how a Java service builds, how a Node service
builds, how a library is installed) is isolated behind a `ServiceExecutor`
abstraction.

- The `ServiceExecutor` trait will be defined by the **application** layer,
  because it is part of the use-case vocabulary and must be replaceable by
  tests. It is intentionally **not** introduced yet: the application currently
  requests tasks and the execution engine reaches the adapters, so no executor
  boundary is needed until concrete build/run behavior is implemented.
- The concrete executors (Java, Node, Library, and future Python/Go/Docker/
  Kubernetes) will be **infrastructure** implementations wired in by the
  composition root.

Generic orchestration code must never branch on service technology. It asks an
executor for the tasks to run; it does not know how they are built.

---

## 10. Where the task engine belongs

All meaningful operations are tasks. The task engine lives in the `execution`
crate, one layer above the domain and below the application:

- It owns task states (`queued`, `running`, `succeeded`, `failed`, `cancelled`,
  `skipped`, `blocked`), dependency-aware scheduling, bounded concurrency,
  cancellation, retry, timeout, progress, events, and an in-memory task
  registry.
- The **application** layer submits tasks to the engine and supplies concrete
  executors; it does not implement scheduling itself.
- Interfaces observe task status through application use cases or the engine's
  query and event API; neither the TUI nor MCP owns task execution.

Workflows are compositions of tasks and live in the same engine, so retry,
timeout, cancellation, and failure propagation are defined once.

### Engine shape

```text
TaskEngine ──> TaskStore (in-memory registry, dependency index)
     │              ↑
     └──> Scheduler ─┘        (event-driven, FIFO, bounded concurrency)
              │
       ExecutorRegistry       (TaskKind -> TaskExecutor)
              │
         TaskExecutor         (supplied by infrastructure)
```

- **Scheduler vs executor.** The scheduler answers *when* a task runs; an
  executor answers *how*. The scheduler contains no per-operation branching:
  concrete executors are registered against a `TaskKind` and looked up at run
  time.
- **State machine.** The engine defers to the domain `Task` state machine for
  every transition; an illegal transition is rejected. Failure propagation
  settles dependents to `queued` or `skipped` according to the task's
  `DependencyPolicy` (`AllSucceeded` or `AnyCompleted`).
- **Concurrency.** A single `tokio::sync::Semaphore` sized to
  `max_parallel_tasks` guarantees `running_tasks <= max_parallel_tasks`;
  `max_queued_tasks` provides optional backpressure.
- **Cancellation** is cooperative through a per-task `Cancellation` token.
- **Retry** is opt-in and happens within one `running` episode.
- **Events** are published on an interface-neutral `broadcast` stream.
- The engine captures a Tokio `Handle` but does not own a runtime, so the
  composition root controls runtime creation and shutdown.
- It depends only on `domain` (plus `tokio`/`async-trait`) and never on `tui`,
  `mcp`, or concrete infrastructure.

The full model, state machine, scheduler behaviour, cancellation and shutdown
semantics, and test strategy are in `docs/task-engine.md`; the decisions behind
them are in `docs/adr/005-task-engine.md`.

---

## 11. Where the process manager belongs

The process manager is the **only** component allowed to create, monitor, and
terminate operating-system processes (`AGENTS.md` §15, §32). It lives in the
`process` infrastructure crate and depends only on `domain` (plus `tokio`,
`thiserror`, and, on Unix, `nix`).

- It spawns, supervises, stops, and kills local processes from a structured
  `ProcessSpec` (a program plus typed arguments — never a shell string), tracks
  pids, captures `stdout` and `stderr` independently, enforces timeouts, and
  applies a restart policy.
- Each process is driven by one supervisor task that owns the Tokio `Child`, so
  the rest of the system interacts only through a `ProcessHandle`. On Unix,
  processes run in their own process group and are signalled as a group, so a
  shell that spawned children never leaves orphans behind.
- A requested stop is always graceful first (`SIGTERM` by default), escalates
  to `SIGKILL` after a configurable grace period, and records an explicit
  `ExitReason` (`Terminated`, `Cancelled`, `Killed`, `TimedOut`, ...).
- It publishes interface-neutral lifecycle events and log lines over a bounded
  broadcast channel, so the TUI, MCP, and logging subsystem each subscribe
  independently. A live pid does not imply a healthy service; health is a
  separate concern (`AGENTS.md` §16).
- It integrates with the task engine's cooperative cancellation through
  `spawn_with_cancellation(spec, future)` rather than depending on `execution`,
  keeping the dependency edge in the caller and the manager usable on its own.
- Git, build tooling, Liquibase, and health adapters must route process
  execution through this manager instead of constructing `std::process::Command`
  themselves.

The full model, specification, lifecycle, registry, cancellation, logging, and
test approach are in `docs/process-manager.md`; the decisions behind them are in
`docs/adr/006-process-manager.md`.

---

## 12. Enforcement and validation

- Dependencies are declared exclusively through `[workspace.dependencies]` so
  boundaries are visible in one place.
- `cargo check/build` rejects dependency cycles.
- Shared lints (`unsafe_code = "forbid"`, `missing_docs`, `rust_2018_idioms`,
  `unreachable_pub`) are defined once in the workspace and opted into by every
  crate.
- CI/validation runs:

  ```bash
  cargo fmt --check
  cargo check --workspace
  cargo test --workspace
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  ```

---

## 13. Domain model

The domain layer defines the vocabulary every other layer speaks. It lives in
the `service-orchestrator-domain` crate and depends on no other workspace crate
(`AGENTS.md`, `docs/adr/001-core-architecture.md`). It declares *what things
are*, never *how they are done*.

The root aggregate is `Workspace`, which owns services, libraries, groups, and
profiles. Key concepts:

- **`Service`** — a buildable/runable unit with an optional repository, runtime
  requirements, structured commands, dependencies, health checks, ports, and an
  explicit capability set. Behaviour is derived from `ServiceType` and
  `ServiceCapability`, never from a service's name.
- **`Library`** — a shared buildable artifact; it may depend only on other
  libraries.
- **`Repository` / `Runtime` / `CommandSet`** — provider-independent
  descriptions of source, toolchain requirements, and structured (non-shell)
  commands.
- **`Dependency`** — a typed reference (`Build`/`Runtime`) to another service or
  library. The domain validates that dependencies resolve and are acyclic but
  does not schedule them.
- **`Task` / `Workflow`** — the task lifecycle (`queued`, `running`, `succeeded`,
  `failed`, `cancelled`, `skipped`, `blocked`) and acyclic workflow graphs. The
  `execution` engine schedules them; the domain owns the vocabulary and the
  state machine.
- **`Permission`** (`Read`, `SafeWrite`, `Destructive`) — the risk vocabulary
  shared by the TUI and MCP; the `policy` engine applies it.
- **State value objects** — `ProcessState`, `GitState`, and `HealthCheck`
  describe observed state without leaking OS, Git, or HTTP details.

Strongly typed identifiers, validated paths/ports/versions, and a small
`DomainError` round out the model. `Workspace::validate` enforces uniqueness,
reference resolution, and acyclicity across the aggregate.

The full entity list, invariants, relationship diagram, and the list of things
intentionally not yet modelled are in `docs/domain-model.md`.

## 14. The application core

The `application` crate is the shared application core (`docs/application-core.md`).
It is the one API the TUI, MCP, and any future interface call, so business
behavior lives in exactly one place.

- **Container.** `Application` holds the validated `Config` plus its boundaries
  (`TaskManager`, `RuntimeStateStore`, `LogService`, `PolicyEngine`) injected
  through the constructor. It is cloneable, so the TUI and MCP each hold an
  `Application` observing the same state and engine.
- **Commands and queries.** Queries (`list_services`, `get_service_status`,
  `get_workspace_status`, `get_task`, `get_logs`, ...) are side-effect free and
  read configuration plus last observed runtime state. Commands
  (`sync_service`, `build_service`, `build_library`, `run_liquibase`,
  `start_service`, `stop_service`, `restart_service`, `health_check`,
  `sync_workspace`, `build_workspace`, `prepare_environment`, `start_group`,
  `start_profile`, ...) validate preconditions, apply policy, and submit a task
  or workflow definition.
- **Results are structured.** `OperationResult`, `ServiceStatus`,
  `WorkspaceStatus`, and `PrepareEnvironmentRequest` are machine-readable
  application values — never pre-formatted strings.
- **Policy.** Every mutating command calls `authorize` before scheduling; a
  destructive operation without confirmation becomes
  `ApplicationError::PermissionDenied`. The check cannot be bypassed by an
  interface.
- **Runtime state is separate from configuration.** Transient process, health,
  and Git state lives in the `RuntimeStateStore` port (`InMemoryRuntimeStateStore`
  for the MVP); the application never parses configuration files and never
  persists runtime state in a database.
- **Work is described, not executed.** The application submits `TaskDefinition`
  and `WorkflowDefinition` values to the `execution` engine; it never runs Git,
  Maven, npm, Liquibase, or processes itself.

## 15. Related documents

- `docs/adr/001-core-architecture.md` — the decision record for the core-first
  architecture.
- `docs/adr/002-domain-model.md` — the decision record for the domain model.
- `docs/adr/003-configuration-system.md` — the decision record for versioned,
  validated configuration.
- `docs/adr/004-application-core.md` — the decision record for the shared
  application core.
- `docs/adr/005-task-engine.md` — the decision record for the task engine and
  scheduler.
- `docs/adr/006-process-manager.md` — the decision record for the process
  manager.
- `docs/application-core.md` — use cases, ports, request/response models, and
  the runtime-state approach.
- `docs/task-engine.md` — the task/workflow engine: state machine, scheduler,
  dependencies, concurrency, cancellation, retry, timeout, events, shutdown.
- `docs/process-manager.md` — the process manager: specification, lifecycle,
  supervision, registry, cancellation, logs, and events.
- `docs/domain-model.md` — the domain entities, relationships, and invariants.
- `AGENTS.md` — the full engineering guidance and product philosophy.

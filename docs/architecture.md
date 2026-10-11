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
| `build` | Infrastructure | Build adapter. A build-system-agnostic manager selects a `BuildProvider` (Maven, npm) that emits a structured `BuildPlan`, which is run in the process manager. Also implements the application's `ArtifactInspector` port (effective Maven local repository and conservative consumer-version reading) and hosts the per-module artifact coordinator. |
| `runtime` | Infrastructure | Service runtime adapter. A runtime-agnostic Service Manager selects a `ServiceRuntime` (Java, Node), builds a structured `StartPlan`, applies idempotency, version, port, and ownership rules, and drives the process manager to run, stop, and restart local services. Implements the read-only `ServiceRuntimeIndex` port. |
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

git   ──> execution (TaskExecutor) + process (run git)     (documented exception, ADR 007)
build ──> execution (TaskExecutor) + process (run builds)  (documented exception, ADR 008)
runtime ──> execution (TaskExecutor) + process (run services) + config   (documented exception, ADR 010)

service-orchestrator (root) ──> everything         (composition root)
```

### Rules

- `domain` depends on no workspace crate.
- `execution` and `policy` depend only on `domain`.
- `application` depends on `domain`, `execution`, `policy`, and `config`. It
  consumes the validated `Config`; it never parses configuration files itself.
- Infrastructure crates depend on `application` (to implement its ports) and
  `domain`. They must not depend on each other or on interfaces, with three
  documented exceptions (`docs/adr/007-git-manager.md`,
  `docs/adr/008-build-manager.md`, `docs/adr/010-service-runtime.md`): the `git`
  adapter, the `build` adapter, and the `runtime` adapter each depend on
  `execution` (they implement a `TaskExecutor` for mutating operations) and on
  `process` (they run the tool through the single process owner); the `runtime`
  adapter additionally reads the validated `config`. No other infrastructure
  crate may take such an edge without an ADR.
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

## 12. Where the Git manager belongs

The Git manager is the infrastructure adapter that implements the application's
Git port. It lives in the `git` crate and drives the installed Git CLI through
the process manager; it never spawns an operating-system process itself
(`AGENTS.md` §15).

- It implements the `GitManager` port (`status`, `branches`, `diff`, `fetch`,
  `pull`, `checkout`, `clone`, `sync`, and the guarded destructive operations)
  from a workspace-aware `ResolvedRepository`, and returns structured
  `RepositoryStatus`, `BranchInfo`, `RepoDiff`, `GitOperationResult`, and
  `GitError` values — never pre-formatted terminal text.
- Mutating operations are expressed as tasks. The crate registers a
  `GitTaskExecutor` against every Git `TaskKind`, so the application submits a
  `TaskDefinition` carrying the repository as parameters and the engine runs it
  with the configured concurrency. Read-only operations (`status`, `branches`,
  `diff`) are called directly on the port.
- It depends on `application` (the port it implements), `domain` (the values it
  produces), `execution` (the `TaskExecutor`/`TaskContext` boundary), and
  `process` (to run Git). The `git -> execution` and `git -> process` edges are
  the first documented infrastructure-to-infrastructure exception (`docs/adr/007-git-manager.md`);
  the Build Manager takes the second (`docs/adr/008-build-manager.md`, §13).
- Repository-scoped coordination serializes mutations of the same checkout while
  independent repositories and concurrent reads proceed in parallel. The
  engine's `max_parallel_tasks` remains the only global concurrency bound.
- It contains no business rules: validation, safety checks, conflict policy, and
  destructive-operation gating stay in the application and policy layers, so the
  TUI and MCP reach identical behavior through the same engine.

The full model, operations, safety rules, concurrency, and authentication
behavior are in `docs/git-manager.md`; the decisions behind them are in
`docs/adr/007-git-manager.md`.

---

## 13. Where the Build Manager belongs

The Build Manager turns a validated build target into a structured build plan
and has it executed. It lives in the `build` crate and is **build-system
agnostic**: Maven is the primary implementation and npm the second, but nothing
in orchestration is coupled to either.

- A focused `BuildProvider` abstraction declares a provider's capabilities,
  validates its build-system requirements, resolves the executable or wrapper,
  and emits a `BuildPlan` (program, typed arguments, working directory,
  environment *requirements*, timeout, expected outputs). Providers are pure,
  stateless, and synchronous; their only output is a value.
- **Providers never spawn processes or schedule tasks.** The `BuildManager` hands
  a plan to the `BuildRunner`, which runs it through the process manager — the
  single process owner (`AGENTS.md` §15). Builds therefore stay supervised,
  cancellable, timeout-bounded, and inside the engine's concurrency.
- Operations are build-system-neutral (`Clean`, `Build`, `Test`,
  `InstallDependencies`, `PublishLocalArtifact`). Providers declare the subset
  their system genuinely supports; an unsupported request fails fast with
  `BUILD_UNSUPPORTED_OPERATION` instead of being approximated. `BuildSystem` and
  `BuildOperation` are domain vocabulary; whether a system supports an operation
  is the provider's honest declaration.
- `BuildProviderRegistry` is a small map from `BuildSystem` to provider. Adding a
  system (Gradle, for example) is: implement `BuildProvider`, register it,
  extend configuration validation, and add tests — no change to the manager, the
  engine, the application core, or the interfaces.
- Every build is a task. `BuildTaskExecutor` decodes a workspace-aware
  `ResolvedBuildTarget` from the task's parameters and delegates to the manager.
  There is no `BuildManager` port in the application, because a build has no
  read-only query an interface would call directly.
- Build-system selection is configuration-driven: an explicit `build_system`
  key is authoritative and validated, otherwise the system is inferred from
  declared runtimes and the programs of the build-related commands. Gradle is
  planned but intentionally not accepted yet, so a configuration can never
  select a system that cannot build it.
- It depends on `application` and `domain`, and — like Git — on `execution`
  (the `TaskExecutor` boundary) and `process` (to run build tools). This is the
  second documented infrastructure-to-infrastructure exception, recorded in
  `docs/adr/008-build-manager.md`.

The full model, operations, capabilities, plans, provider selection, errors, and
test approach are in `docs/build-manager.md`; the decisions behind them are in
`docs/adr/008-build-manager.md`.

---

## 14. Where the Library Manager belongs

The Library Manager manages shared libraries and their consumers. It is
**application-layer orchestration**, not a new crate: it validates library
configuration, resolves an artifact against the effective build system, composes
the existing Git and build tasks into dependency-ordered workflows, and submits
them to the task engine. It never runs a build tool or a Git command itself.

- A library is a first-class domain entity (`docs/domain-model.md`), identified
  by artifact **coordinate** (`group:artifact:version`), never by repository
  name. A reactor project may produce several coordinates.
- **Build Manager integration.** Builds and installs are build-system-neutral
  operations (`Build`, `PublishLocalArtifact`) submitted as `BuildLibrary` and
  `LibraryInstall` tasks. Maven performs the install with its `install` phase; the
  manager never copies a JAR and never shells out. A library whose build system
  cannot publish a local artifact fails with `BUILD_PROVIDER_UNSUPPORTED`.
- **Git integration.** Synchronization and checkout reuse the Git task kinds and
  are optional, so the current local branch (with uncommitted changes) can be
  built. The configured default branch is never forced.
- **A new port.** Verification and version alignment are the only Maven-specific
  needs, so the application declares an `ArtifactInspector` port (read-only
  `locate`/`dependency_version`) that the `build` crate implements. The effective
  local repository is discovered through Maven, never assumed; there is no custom
  dependency resolver.
- **Ordering and coordination.** Selected libraries are ordered dependency-first
  as workflow-step dependencies — the engine stays the only scheduler. Concurrent
  writers over one module are serialized by a keyed `ArtifactCoordinator` scoped
  to the module directory; there is no global lock.
- **Version alignment is read-only.** Consumers are mapped to `Aligned`,
  `Mismatch`, `Unknown`, or `Unresolved`; a version that cannot be resolved
  reliably is never guessed and no POM is rewritten. Automatic modification is
  deliberately deferred.
- It adds no crate and no dependency edge: it lives in `application` and reaches
  Git, build, and inspection through the existing task kinds and the new port.

The operations, artifact model, workflows, safety rules, and limitations are in
`docs/library-manager.md`; the decisions behind them are in
`docs/adr/009-library-manager.md`.

---

## 15. Where the Service Runtime belongs

The Service Runtime turns a validated service into a structured start plan and
drives the process manager to run, stop, and restart it. It lives in the
`runtime` infrastructure crate and keeps three seams separate: the **runtime**
decides *how* a service starts, the **process manager** owns OS processes, and
the **task engine** owns scheduling (`AGENTS.md` §11, §15, §21).

- A focused `ServiceRuntime` abstraction validates its preconditions and emits a
  **`StartPlan`** (program, typed arguments, absolute working directory, full
  environment, shutdown strategy). Runtimes are pure and synchronous; their only
  output is a value. Collectively they contain no business rules.
- **Runtimes never spawn processes or schedule tasks.** The `ServiceManager`
  hands the plan to the process manager — the single process owner — and records
  ownership of the resulting process synchronously, before the first `await`, so
  the managed process outlives the startup task (`tasks/010.md` §19).
- `RuntimeRegistry` is a small `RuntimeType -> runtime` map, not a plugin
  framework. `JavaRuntime` and `NodeRuntime` are the initial set; a future
  Python/Go/Docker/Kubernetes runtime is a new implementation and a registry
  line, with no change to orchestration, the engine, the process manager, the
  application, or the interfaces.
- Environment observation is separated from execution: a `RuntimeHost` seam
  resolves executables across `PATH`, `JAVA_HOME`, and version managers
  (`nvm`, `fnm`, Volta, `asdf`, `sdkman`, `jenv`), and an asynchronous
  `RuntimeInspector` seam probes the installed toolchain version through the
  process manager. A `NullRuntimeInspector` default keeps most starts free of
  probing.
- Common lifecycle rules are applied once, uniformly: start/stop idempotency, a
  per-service lock for concurrent starts, pinned-version checks, port-conflict
  reporting that **never kills** the occupying process, and cooperative
  cancellation that never orphans a spawned process (`tasks/010.md` §14, §16,
  §17, §21, §26).
- Every lifecycle operation is a task. A `ServiceRuntimeTaskExecutor` decodes a
  workspace-aware `ResolvedServiceTarget` (identity and paths only — never
  environment values) from the task parameters and delegates to the manager.
- The application only needs a **read-only** port, `ServiceRuntimeIndex`, for
  its status queries (`get_service_runtime_status`, `list_running_services`); a
  `NullServiceRuntimeIndex` keeps it constructible. Mutating operations need no
  port because they are tasks, exactly like builds and Git.
- It depends on `application` and `domain`, and — like Git and Build — on
  `execution` (the `TaskExecutor` boundary) and `process` (to run services), plus
  the validated `config`. This is the third documented
  infrastructure-to-infrastructure exception, recorded in
  `docs/adr/010-service-runtime.md`.

The full model, runtimes, registry, lifecycle, ownership, cancellation, events,
errors, and macOS behavior are in `docs/service-runtime.md`; the decisions behind
them are in `docs/adr/010-service-runtime.md`.

---

## 16. Enforcement and validation

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

## 17. Domain model

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
- **`Library`** — a shared buildable artifact identified by artifact coordinate
  (`group:artifact:version`); it produces a primary coordinate plus any
  additional reactor artifacts, and may depend only on other libraries.
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

## 18. The application core

The `application` crate is the shared application core (`docs/application-core.md`).
It is the one API the TUI, MCP, and any future interface call, so business
behavior lives in exactly one place.

- **Container.** `Application` holds the validated `Config` plus its boundaries
  (`TaskManager`, `RuntimeStateStore`, `LogService`, `PolicyEngine`) injected
  through the constructor. It is cloneable, so the TUI and MCP each hold an
  `Application` observing the same state and engine.
- **Commands and queries.** Queries (`list_services`, `get_service_status`,
  `get_workspace_status`, `get_service_runtime_status`, `list_running_services`,
  `get_task`, `get_logs`, ...) are side-effect free and read configuration plus
  last observed runtime state. Commands (`sync_service`, `build_service`,
  `build_library`, `run_liquibase`, `start_service`, `stop_service`,
  `restart_service`, `start_services`, `stop_services`, `restart_services`,
  `health_check`, `sync_workspace`, `build_workspace`, `prepare_environment`,
  `start_group`, `start_profile`, ...) validate preconditions, apply policy, and
  submit a task or workflow definition.
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
- **Git.** Git use cases express every mutating operation (`sync`, `fetch`,
  `pull`, `checkout`, `clone`, and the destructive `reset`/`clean`/`discard`) as
  a task, and read-only operations (`get_git_status`, `list_branches`,
  `get_repository_diff`) as queries over the `GitManager` port. Destructive
  commands pass through `authorize(Permission::Destructive)` at submission, so
  an interface cannot schedule them without confirmation (`AGENTS.md` §30).
  Bulk commands submit one independent workflow step per repository;
  `summarize_workflow` aggregates the per-repository outcomes.

## 19. Related documents

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
- `docs/adr/007-git-manager.md` — the decision record for the Git manager.
- `docs/adr/008-build-manager.md` — the decision record for the Build Manager
  and its provider abstraction.
- `docs/adr/009-library-manager.md` — the decision record for library
  management, artifact inspection, and version alignment.
- `docs/adr/010-service-runtime.md` — the decision record for the Service
  Runtime, its registry, and managed-process lifecycle.
- `docs/application-core.md` — use cases, ports, request/response models, and
  the runtime-state approach.
- `docs/task-engine.md` — the task/workflow engine: state machine, scheduler,
  dependencies, concurrency, cancellation, retry, timeout, events, shutdown.
- `docs/process-manager.md` — the process manager: specification, lifecycle,
  supervision, registry, cancellation, logs, and events.
- `docs/git-manager.md` — the Git manager: operations, safety, coordination,
  task-engine integration, and authentication.
- `docs/build-manager.md` — the Build Manager: provider abstraction, build
  plans, operations and capabilities, provider selection, and errors.
- `docs/library-manager.md` — the Library Manager: library model, artifact
  coordinates, Git and build integration, local installation, multi-module
  projects, version alignment, and safety.
- `docs/service-runtime.md` — the Service Runtime: runtimes, registry,
  lifecycle, ownership, cancellation, events, and macOS behavior.
- `docs/domain-model.md` — the domain entities, relationships, and invariants.
- `AGENTS.md` — the full engineering guidance and product philosophy.

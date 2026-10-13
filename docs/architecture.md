# Architecture

Service Orchestrator is a macOS-native developer environment orchestration
platform written in Rust. It prepares and manages local multi-repository,
multi-service development environments: synchronizing repositories, checking
out branches, building shared libraries, running Liquibase migrations, building
and starting applications, inspecting logs, running health checks, and
executing dependency-aware workflows.

It exposes the same capabilities through three interfaces:

- a Ratatui terminal UI for human developers,
- an MCP server for AI agents, and
- a Tauri 2 desktop GUI for human developers (`apps/desktop`).

All interfaces are adapters over one shared application core. That core runs
inside a single local **daemon**; the TUI, MCP, and GUI are clients of it over a
Unix domain socket, so they share one task engine, one process registry, and one
log store (`docs/local-daemon.md`, `docs/adr/015-shared-local-runtime.md`,
`docs/adr/018-tauri-desktop-gui.md`).

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

> The application core must not depend on Ratatui, Tauri, or MCP. All are
> interface adapters over the same application core.

There is exactly one source of truth for behavior:

```text
TUI -> application use case -> task engine -> infrastructure
MCP -> application use case -> task engine -> infrastructure
GUI -> application use case -> task engine -> infrastructure
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
5. **Client / local runtime** — the `client` crate (the `OrchestratorClient`
   boundary and its IPC transport) and the `daemon` crate (the local IPC server
   that owns the application core). Neither contains business logic.
6. **Interfaces** — TUI, MCP, and desktop GUI adapters, all clients of the daemon.
7. **Composition root** — the `service-orchestrator` binary. It wires concrete
   infrastructure into the application core, hands the core to the daemon for the
   `daemon` subcommand, and hands a client to an interface for `tui`/`mcp`.

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
                                           |  OrchestratorClient
                                           v
                        +-------------------------------+
   Client / runtime     |    local daemon (IPC)         |
                        |  routes requests + fans events|
                        +---------------+---------------+
                                        |  application use cases
                                        v
                        +-------------------------------+       future:
   Application          |          application          |<----- cli, gui
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
domain. Nothing inner knows about anything outer. The daemon sits between the
interfaces and the application: it owns the application core and exposes it to
clients over IPC, but it adds no business logic of its own.

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
| `liquibase` | Infrastructure | Migration adapter. A provider-based Migration Manager selects a `MigrationProvider` (Liquibase) that emits a structured `MigrationPlan`, which is run in the process manager. Applies the database-target environment safety policy and serializes migrations against the same configured database. |
| `health` | Infrastructure | Health adapter. A `HealthManager` selects a `HealthChecker` (process, TCP, HTTP/actuator) through a small registry, runs each configured probe under its own timeout, aggregates the outcomes deterministically, records bounded history and events, mirrors the aggregate into the runtime state store, and implements the read-only `HealthIndex` port. A `HealthMonitor` optionally polls watched services. |
| `logging` | Infrastructure | Observability adapter. A `LogHub` unifies managed-process output, task-engine events, and application messages into one structured, bounded, filterable store, answers stable cursor queries, and streams realtime entries to multiple backpressure-tolerant subscribers. Thin ingestors correlate existing event sources; messages are redacted before storage. Implements the `LogService`/`LogSubscription` ports. |
| `client` | Client / runtime | The shared `OrchestratorClient` boundary every interface talks to, the versioned length-prefixed JSON IPC protocol, socket-path resolution, daemon auto-start and readiness waiting, and `RemoteClient` — the daemon-backed client. Also implements the trait for `Application` so in-process tests need no daemon. |
| `daemon` | Client / runtime | The local shared runtime: binds the Unix domain socket, validates protocol versions, routes each request to the shared `Application`, fans task/log events out to clients over bounded channels, and manages single-instance locking and graceful shutdown. Contains no business logic. |
| `mcp` | Interface | MCP stdio adapter over the shared runtime client: strict tool schemas and structured envelopes, read-only resources, workflow prompts, and standard-authorization policy enforcement. |
| `tui` | Interface | Ratatui adapter over the shared runtime client. |
| `service-orchestrator` (root) | Composition root | Wires infrastructure into the core, runs the daemon for `daemon`, and hands a client to an interface for `tui`/`mcp`. |

---

## 5. Dependency direction

The intended direction, expressed as allowed Cargo dependencies:

```text
tui  ─────────┐
              │   (client)          application ──> execution ──> domain
mcp  ─────────┼──> client ──> ...            │    ├──> policy  ──> domain
              │                              │    └──> config  ──> domain
future cli ───┘                              └──> domain

daemon ──> client + application + execution + domain   (local runtime host)

git, process, build, liquibase, health, logging
    ────────────────────────> application + domain   (implement ports)

git   ──> execution (TaskExecutor) + process (run git)     (documented exception, ADR 007)
build ──> execution (TaskExecutor) + process (run builds)  (documented exception, ADR 008)
runtime ──> execution (TaskExecutor) + process (run services) + config   (documented exception, ADR 010)
liquibase ──> execution (TaskExecutor) + process (run migrations) + build (Maven/host seam)   (documented exception, ADR 011)
health ──> execution (TaskExecutor) + config (resolve checks) + runtime index   (documented exception, ADR 012)
logging ──> execution (task events) + process (captured output)   (documented exception, ADR 012)

service-orchestrator (root) ──> everything         (composition root)
```

### Rules

- `domain` depends on no workspace crate.
- `execution` and `policy` depend only on `domain`.
- `application` depends on `domain`, `execution`, `policy`, and `config`. It
  consumes the validated `Config`; it never parses configuration files itself.
- Infrastructure crates depend on `application` (to implement its ports) and
  `domain`. They must not depend on each other or on interfaces, with six
  documented exceptions (`docs/adr/007-git-manager.md`,
  `docs/adr/008-build-manager.md`, `docs/adr/010-service-runtime.md`,
  `docs/adr/011-migration-manager.md`,
  `docs/adr/012-health-observability.md`): the `git`, `build`, `runtime`,
  `liquibase`, and `health` adapters each depend on `execution` (they implement a
  `TaskExecutor`), and the `git`, `build`, `runtime`, `liquibase`, and `logging`
  adapters depend on `process` (to run the tool, or to consume captured output
  through the single process owner); the `runtime`/`liquibase` adapters
  additionally read the validated `config` / reuse the build adapter's
  Maven-resolution and host seam, and the `health` adapter reads the validated
  `config` to resolve a service's checks. No other infrastructure crate may take
  such an edge without an ADR.
- `tui` and `mcp` depend only on `client` (which re-exports the shared boundary),
  plus the `domain` and `execution` event/value types. They never depend on the
  daemon, the transport, or any infrastructure crate.
- `daemon` depends on `client` (for the wire protocol, framing, and socket
  helpers), `application` (the core it hosts), `execution` (the task engine it
  fans events from), and `domain`; it never depends on `tui` or `mcp`.
- The root binary is the only crate allowed to depend on both infrastructure
  and interfaces/the daemon; it is the composition root.

### Forbidden edges

```text
domain      -> tui / mcp / client / daemon / any infrastructure
application -> ratatui / mcp / client / daemon / concrete infrastructure
tui         -> infrastructure / daemon (git, process, build, ...)
mcp         -> infrastructure / daemon (git, process, build, ...)
daemon      -> tui / mcp
git         -> tui / mcp / daemon
```

These edges are not merely documentation: because every dependency is an
explicit workspace dependency, adding a forbidden edge requires editing a
`Cargo.toml` and is therefore visible in review. Cyclic dependencies are
rejected by Cargo at build time.

---

## 6. Why the interfaces are adapters

The TUI, the MCP server, and the desktop GUI answer the same questions ("what
services exist?", "build this service", "start this profile") over different
transports. If each implemented its own version of those operations, behavior
would drift and every new capability would have to be built three times.

Instead:

- The TUI reads application state, renders it, and turns user input into
  application commands. It contains no Git, build, process, or workflow logic.
- The MCP server validates tool input, applies policy, and forwards calls to
  application use cases. It contains no business logic and never shells out
  directly.
- The desktop GUI validates its inputs, applies policy, and forwards calls to the
  same use cases for reads, service lifecycle, Git, build, and library
  management, migration management, task browsing and cancellation, health
  queries and checks, and unified logs, then renders the answers; it holds no
  second client, task engine, migration manager, health manager, log system, or
  process manager.

All speak to the application core through the same `OrchestratorClient` boundary
and, in production, through the same local daemon. They are therefore not only
logically but *operationally* identical: they observe one shared task engine,
process registry, and log store, so a change made through one is immediately
visible to the others (`docs/local-daemon.md`).

They therefore see identical behavior, identical policies, and identical error
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

## 16. Where the Migration Manager belongs

The Migration Manager turns a validated migration target into a structured
migration plan and has it executed. It lives in the `liquibase` crate and is
**provider-based**: Liquibase is the initial implementation, but nothing in
orchestration is coupled to it.

- A focused `MigrationProvider` abstraction declares a provider's capabilities,
  validates its requirements, resolves its executable or wrapper, and emits a
  `MigrationPlan` (program, typed arguments, working directory, environment,
  optional timeout, display-safe database identity and coordination key, safety
  classification). Providers are pure, stateless, and synchronous; their only
  output is a value.
- **Providers never spawn processes or schedule tasks.** The `MigrationManager`
  hands a plan to the `MigrationRunner`, which runs it through the process
  manager — the single process owner (`AGENTS.md` §15). Migrations therefore stay
  supervised, cancellable, timeout-bounded, and inside the engine's concurrency.
- Operations are provider-neutral (`Validate`, `Status`, `Pending`, `Preview`,
  `Apply`). Only `Apply` mutates the database. Providers declare the subset they
  support; an unsupported request fails fast with
  `MIGRATION_UNSUPPORTED_OPERATION`. `MigrationProviderType`,
  `MigrationOperation`, and `MigrationTargetEnvironment` are domain vocabulary;
  whether a provider supports an operation is the provider's honest declaration.
- `MigrationProviderRegistry` is a small map from `MigrationProviderType` to
  provider. Adding a tool (Flyway, for example) is: implement `MigrationProvider`,
  register it, extend configuration validation, and add tests — no change to the
  manager, the engine, the application core, or the interfaces.
- **Database target identification and safety.** A target is described by
  display-safe metadata only (environment, type, name, host, schema), never a
  credential. Only an explicitly configured local database may mutate without
  explicit authorization; every other declared environment — including an
  unidentified one — requires `Permission::Destructive`. Locality is never
  inferred from a hostname. The application applies the policy at submission and
  the manager re-checks it, so it cannot be bypassed with a hand-crafted task.
- **Database-level coordination.** `DatabaseCoordinator` is a keyed write lock
  over the target database's identity, so two writers never race over one
  database while unrelated targets still run concurrently. It is not a scheduler;
  the engine's `max_parallel_tasks` remains the only global bound. The lock is
  in-process only — it does not coordinate separate Service Orchestrator
  instances; Liquibase's own database lock is the cross-process mechanism, and it
  is recognized but **never** released automatically.
- **Failure and cancellation semantics.** A failed migration never claims a
  rollback, is never retried automatically, and reports its affected service and
  database target with a stable error code. Cancelling the external process does
  not guarantee that database changes were rolled back.
- Every migration is a task. `MigrationTaskExecutor` decodes a workspace-aware
  `ResolvedMigrationTarget` from the task's parameters and delegates to the
  manager. There is no `MigrationManager` port in the application, because status
  inspection is itself a process execution — there is no read-only migration
  query an interface would call directly.
- It depends on `application` and `domain`, and — like Git, Build, and Runtime —
  on `execution` (the `TaskExecutor` boundary) and `process` (to run migration
  tools), plus the `build` crate for the Maven-resolution and host seam. This is
  the fourth documented infrastructure-to-infrastructure exception, recorded in
  `docs/adr/011-migration-manager.md`.

The full model, provider abstraction, Liquibase implementation, configuration,
target safety, coordination, failure semantics, and limits are in
`docs/migration-manager.md`; the decisions behind them are in
`docs/adr/011-migration-manager.md`.

---

## 17. Where the Health Manager and Observability layer belong

The Health Manager and the Observability layer answer two questions the rest of
the platform cannot: *is a service actually serving?* and *what evidence do we
have?* They live in the `health` and `logging` infrastructure crates and reuse
the process manager, the runtime adapter, the task engine, and validated
configuration rather than duplicating any of them (`AGENTS.md` §16, §35).

### Health

- **Health is not process state.** `HealthState` (`unknown`, `checking`,
  `healthy`, `unhealthy`, `degraded`, `unreachable`) is separate from
  `ProcessState`, so "running but unhealthy" and "running but still starting" are
  both representable. A live pid never implies a healthy service.
- **Providers are small and pure.** `HealthChecker` is the only place probe
  mechanics live; `ProcessHealthChecker`, `TcpHealthChecker`, and
  `HttpHealthChecker` are resolved through a small `BTreeMap<CheckType, ...>`
  registry, not a plugin framework. The process provider reads the runtime
  adapter's managed-process index (never scanning the OS); the TCP provider
  probes a configured loopback port; the HTTP provider sends one `GET` and treats
  a Spring Boot Actuator `status` in the body as authoritative.
- **A probe result is not an error.** An ordinary "not healthy" observation is a
  `CheckResult` recorded as a state; a `HealthError` means the check could not be
  attempted at all.
- **Aggregation is deterministic and ordered.** Required failures dominate,
  unreachable dominates undetermined, and a failing *optional* check only
  degrades. Statuses are never averaged into a percentage.
- **Readiness is explicit and bounded.** The required checks represent the
  process/port/HTTP stages; `wait_until_ready` polls with cancellation until the
  service is serving or the deadline passes, exposed as a `ReadinessCheck` task
  with a 60-second default. Starting and waiting-for-readiness are distinct, and
  a startup task is never left running indefinitely.
- **Polling is opt-in.** `HealthMonitor` polls only explicitly watched services,
  is idempotent (no duplicate loops), bounds concurrency with a semaphore, and
  shuts down cleanly. It submits no tasks; the task engine remains the only
  scheduler.
- **One read-only port.** The application reads the last observed health through
  the `HealthIndex` port (`service_health`, `services`); scheduling a probe is a
  command, and a query never probes (`AGENTS.md` §14). `NullHealthIndex` keeps the
  application constructible.

### Observability

- **Reuse, don't rewrite.** Thin ingestors translate the process manager's
  captured output and the task engine's events into a common `LogEntry`; the
  process manager and task engine stay the owners of their own behaviour.
- **Correlation, not fabrication.** A process line resolves its service through
  the process manager's registry; a task event inherits the service cached from
  `TaskCreated`. Missing metadata is left absent.
- **One bounded, queryable store.** `LogHub` assigns a monotonic `sequence`,
  redacts and truncates messages, enforces per-service and global retention, and
  answers filtered, newest-first queries plus stable, oldest-first cursor pages —
  not a database and not a search engine.
- **Streaming is independent and lossy.** Each service has a bounded broadcast
  channel, so a slow subscriber drops only its own entries and observes
  `Lagged { dropped }`; it never blocks a managed process's `stdout`/`stderr`
  reader (`AGENTS.md` §35).
- **Diagnostics report observed facts.** `get_service_diagnostics` composes the
  runtime state, managed process, latest health result, recent error logs, and
  declared ports, and never fabricates a root cause.

Both adapters are the fifth and sixth documented infrastructure-to-infrastructure
exceptions (health to `execution`/`config`, logging to `execution`/`process`),
recorded in `docs/adr/012-health-observability.md`.

The full model, providers, state and aggregation rules, readiness, polling,
events, errors, and security considerations are in `docs/health-manager.md`; the
log model, ingestion, filtering, pagination, retention, streaming, and redaction
are in `docs/observability.md`.

---

## 18. Where workspace validation belongs

Validating a configured workspace — the root exists, the repositories are
present and valid, the runtimes are installed, the dependency graph resolves,
the declared ports do not collide — is an application-level concern, so every
interface observes the same answer (`docs/adr/017-workspace-validation.md`).

- **A read-only query.** `ValidateWorkspace` inspects and reports; it never
  mutates a repository, a port, or the configuration. A configured port that
  is occupied by an unrelated process is reported as a warning and the
  occupying process is never signalled — the same rule the Service Runtime
  follows (`AGENTS.md` §30).
- **One read-only port.** System facts (path existence, executable on `PATH`,
  Git working tree, loopback port occupancy) arrive through the
  `WorkspaceProbe` port; `NullWorkspaceProbe` is the permissive default, and
  the composition root supplies `SystemWorkspaceProbe` (`src/probe.rs`).
- **Defence in depth.** The configuration loader already rejects unknown
  dependencies, cycles, and undeclared health ports; the validator re-checks
  them so a `Config` assembled by any other means is still diagnosed.
- **Client boundary, not a special case.** The use case is reached through
  `OrchestratorClient::validate_workspace` over the daemon's IPC, exposed as
  the MCP `validate_workspace` tool with a strict schema.

---

## 19. Where the shared local runtime belongs

Three interfaces over three private application cores cannot share one developer
environment. The **shared local runtime** gives them one owner of mutable state:
a local **daemon** that hosts the application core, with the TUI and MCP as
clients over a Unix domain socket (`docs/local-daemon.md`,
`docs/adr/015-shared-local-runtime.md`).

- **A single binary, three subcommands.** `service-orchestrator daemon` runs the
  runtime in the foreground; `tui` and `mcp` connect to it. There is no separate
  daemon executable and no external backend.
- **One owner of mutable state.** The daemon owns the task engine, the process
  registry, per-service runtime state, health, log subscriptions, and the
  validated configuration snapshot (read-only to clients). Clients hold only
  refreshable caches, so a task submitted by MCP is immediately visible in the
  TUI and a service started from the TUI is manageable from MCP.
- **The `client` crate is the boundary.** `OrchestratorClient` mirrors the
  application's use cases one-to-one; `Application` implements it in-process (so
  interface tests run on the real core with no daemon) and `RemoteClient`
  implements it as the socket proxy. `client` also owns the versioned protocol,
  socket-path resolution, auto-start, and readiness waiting.
- **The `daemon` crate is a router, not a business layer.** It validates the
  protocol version, decodes each request's parameters, calls the shared
  `Application`, and encodes the structured result. It contains no business
  logic, so an operation behaves identically in-process and over IPC. Requests
  are routed to explicit, versioned operations — the daemon does not proxy the
  application's Rust port surface generically.
- **IPC, not a shell.** The transport is a Unix domain socket carrying
  length-prefixed JSON frames capped at 8 MiB. There is no arbitrary command
  execution and no public TCP port. The protocol is versioned; an incompatible
  request is refused with an actionable `PROTOCOL_INCOMPATIBLE` error rather than
  silently misinterpreted.
- **Automatic startup and one instance.** `tui`/`mcp` connect to a running
  daemon or start one, wait for readiness, and connect. Single-instance
  coordination uses an advisory `flock` on a lock file beside the socket, so two
  clients racing to start a daemon still produce exactly one authoritative
  engine; a stale socket file is removed safely once the lock is held.
- **Events and reconnection.** The daemon fans task and log events out to
  clients over bounded, per-connection channels, so a slow client never blocks
  the daemon. A client reconnects transparently only when a request was never
  written; a request written but unanswered is never retried, because the
  mutation may have been applied.
- **Authorization is unchanged and centralized.** A request carries an
  `allow_destructive` intent and the caller's `ClientKind`; the daemon honors the
  intent only for an interactive (`tui`) peer and always runs the shared policy
  engine. A non-interactive client can never self-confirm, and no client-supplied
  boolean is sufficient.
- **Shutdown is explicit.** Client exit never stops the daemon or the developer's
  services; `service-orchestrator daemon stop` requests a graceful shutdown, and
  `daemon status` reports the runtime without starting one.
- **Process ownership.** The daemon owns managed service processes; a fresh
  daemon adopts nothing and never signals a process by PID or port, so a reused
  PID cannot harm an unrelated process. Managed-process recovery after a crash is
  deliberately deferred and documented as a limitation.

The full lifecycle, protocol, security, and limitations are in
`docs/local-daemon.md`; the decision and its trade-offs are in
`docs/adr/015-shared-local-runtime.md`.

---

## 20. Enforcement and validation

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

## 21. Domain model

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

## 22. The application core

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
- **Environment workflows.** `prepare_environment` composes the end-to-end
  preparation graph — Git sync, library builds, migrations, service builds,
  startup, health — into one `WorkflowDefinition`, and
  `plan_prepare_environment` returns the matching read-only `EnvironmentPlan`
  from the *same* composer, so a plan cannot drift from what runs.
  `get_workflow_status` aggregates state, per-phase progress, and structured
  failures by workflow id, and `cancel_workflow` cancels the still-active steps.
  Planning and execution are separate policy modes: planning needs only `Read`
  and reports `requires_destructive_authorization`; execution re-enforces
  `SafeWrite`/`Destructive` on every revalidated precondition
  (`docs/environment-workflows.md`).
- **Git.** Git use cases express every mutating operation (`sync`, `fetch`,
  `pull`, `checkout`, `clone`, and the destructive `reset`/`clean`/`discard`) as
  a task, and read-only operations (`get_git_status`, `list_branches`,
  `get_repository_diff`) as queries over the `GitManager` port. Destructive
  commands pass through `authorize(Permission::Destructive)` at submission, so
  an interface cannot schedule them without confirmation (`AGENTS.md` §30).
  Bulk commands submit one independent workflow step per repository;
  `summarize_workflow` aggregates the per-repository outcomes.

## 23. The Ratatui TUI adapter

The `tui` crate is the human-facing interface (`docs/tui.md`,
`docs/adr/013-ratatui-tui.md`). It is launched as `service-orchestrator tui`
(and by running the binary with no arguments).

- **Adapter, not an implementation.** It reads application state, renders it,
  and turns keyboard input into application use cases. It never runs Git, Maven,
  npm, Liquibase, or an operating-system process, and it never duplicates
  application logic. The crate depends on the `client` boundary (plus the domain
  and execution event types) but not on the daemon, the transport, or any
  infrastructure crate, so the boundary is enforced by the dependency graph
  rather than by convention.
- **A client of the shared runtime.** In production the TUI holds an
  `Arc<dyn OrchestratorClient>` backed by `RemoteClient`, so every action it
  performs is a request to the one daemon (`docs/local-daemon.md`). Its caches are
  refreshable and never authoritative. The same code runs against an in-process
  `Application` in tests.
- **State machine.** `App` turns an `Action` into either a change of interface
  state or an application-core call. It owns `UiState` (presentation: screen,
  cursor, typed selection, dialogs, search, scroll) and a read-only `Snapshot`
  (a wholesale-replaced cache of application state). The application core remains
  authoritative.
- **Single async event loop.** Terminal input, task-engine events, realtime log
  entries, spawned-query results, and a tick are multiplexed onto one channel.
  The loop draws, then awaits one event; no long-running operation blocks the
  renderer. Asynchronous queries are described by `Effect` values that only the
  event loop executes.
- **Event-driven updates.** The interface subscribes to the task engine's event
  stream and to per-service log subscriptions; a light periodic tick refreshes
  the in-memory snapshot without polling repositories.
- **Typed multi-selection.** Selections are sets of `ServiceId`/`LibraryId`, so
  they stay stable under filtering and sorting; bulk operations reuse the
  application's workflow use cases.
- **Policy and confirmation.** Destructive operations are denied by the
  application under standard authorization; the interface opens a confirmation
  dialog and, on confirmation, re-runs the operation with destructive
  authorization. The application performs the same check, so a UI confirmation is
  never the authorization itself.
- **Terminal lifecycle.** Raw mode and the alternate screen are restored on
  normal exit, error, Ctrl+C, and panic; terminals below 60×14 render an
  informative minimum-size notice instead of a broken layout.

## 24. The MCP adapter

The `mcp` crate is the agent-facing interface (`docs/mcp.md`,
`docs/adr/014-mcp-integration.md`). It is launched as `service-orchestrator mcp`
and speaks the Model Context Protocol over **stdio**, built on the official Rust
SDK `rmcp` 3.5.1.

- **Adapter, not an implementation.** It validates tool input, applies policy,
  and forwards calls to application use cases. It never runs Git, Maven, npm,
  Liquibase, or an operating-system process, and it never duplicates application
  logic. The crate depends on the `client` boundary (plus the domain and execution
  event types) but not on the daemon, the transport, or any infrastructure crate,
  so the boundary is enforced by the dependency graph. The raw IPC protocol is
  never exposed to an agent.
- **A client of the shared runtime.** In production MCP holds an
  `Arc<dyn OrchestratorClient>` backed by `RemoteClient`; `serve_stdio` receives
  the client and never learns it is talking to a daemon. This is what makes an MCP
  session share state with the TUI: a task submitted by an agent is visible in the
  TUI and vice versa (`docs/local-daemon.md`).
- **stdout is protocol-only.** Every diagnostic the adapter emits goes to stderr
  as a single-line JSON record (lifecycle events and one record per tool
  invocation, carrying the tool name, outcome, duration, and error code — never
  an argument value). A stray `println!` would corrupt the transport.
- **One shared core, now one shared runtime.** The composition root builds one
  application core and one task engine and hands them to the daemon; the TUI and
  MCP connect to that daemon as clients. `Application::new` defaults to standard
  authorization, and the daemon enforces it for every request, so both interfaces
  observe identical behavior, policies, and errors.
- **Tools.** Each tool is a thin, named projection of exactly one application use
  case, with a strict typed input schema derived from a `params.rs` struct
  (`deny_unknown_fields`, so an unmodelled option is rejected). Tools are
  advertised only where a use case exists; gaps (`install_dependencies`,
  `test_service`, a combined bulk build) are documented, not faked. There is no
  arbitrary shell tool.
- **Structured envelopes.** Every result is `{success, data}` or
  `{success: false, error}`. A malformed request (unknown tool, wrong JSON
  shape) is a JSON-RPC protocol error; a valid request that the application
  refuses or that fails — including a policy denial — is a tool result with
  `isError: true` and a structured, coded error.
- **Policy.** The adapter presents itself to the daemon as a non-interactive
  (`mcp`) client and never requests destructive authorization. A destructive tool
  (for example `git_reset_hard`) is denied by the shared policy engine with
  `CONFIRMATION_REQUIRED` — the daemon ignores an `allow_destructive` intent from
  a non-interactive peer. There is no `confirmed` flag for an agent to
  self-authorize, so a human must approve destructive work interactively.
- **Long-running work.** Every mutation returns the task or workflow identifier
  the application core produced. `get_task` / `task://{taskId}` expose status,
  progress, and structured failure; a bounded `TaskMonitor` boundary is the only
  extra surface the adapter needs. The task itself lives in the daemon's single
  engine, so it is shared with every other interface rather than being
  per-process.
- **Resources and prompts.** Read-only resources (`workspace://`, `service://`,
  `task://`) are backed by existing application queries, with templates so the
  list stays bounded. Prompts are guidance only and reference only tools that
  exist.

## 25. The Tauri desktop GUI adapter

The `apps/desktop` application is the graphical interface (`docs/desktop-gui.md`,
`docs/adr/018-tauri-desktop-gui.md`, `docs/adr/019-desktop-service-management.md`,
`docs/adr/020-desktop-git-build-library.md`).
It is launched with `npm run tauri dev` during development; the Tauri crate is
deliberately its own Cargo workspace, excluded from the root one, so the
`wry`/`tao`/`objc` dependency tree is compiled only for the GUI.

- **Adapter, not an implementation.** The Rust crate behind the webview
  validates input, applies policy, and forwards calls to application use cases.
  It never runs Git, Maven, npm, Liquibase, or an operating-system process, and
  it never duplicates application logic. It depends on the `client` boundary
  (plus the domain, application, execution, and config types) but not on the
  daemon, the transport, or any infrastructure crate, so the boundary is
  enforced by the dependency graph rather than by convention.
- **A client of the shared runtime.** The adapter holds the same
  `Arc<RemoteClient>` the TUI and MCP use, so a task visible in one interface is
  visible in all of them and the GUI holds no competing state
  (`docs/local-daemon.md`).
- **Read and lifecycle commands.** The adapter exposes read commands
  (`get_daemon_status`, `list_workspaces`, `list_services`,
  `get_workspace_status`, `get_service_status`, `get_service_detail`, and
  `get_connection_state`) and lifecycle commands (`start_service`,
  `stop_service`, `restart_service`, `start_services`, `stop_services`,
  `restart_services`, and `check_service_health`). Every command validates its
  identifiers into domain types at the boundary and delegates to the shared
  client. Bulk operations submit a single daemon workflow rather than a
  frontend loop, and restart always uses the daemon's coordinated
  stop-then-start.
- **Git, build, and library commands.** The adapter also exposes Git reads
  (`list_repositories`, `get_git_status`, `inspect_repositories`,
  `list_branches`, `preview_git_sync`) and mutations (`fetch_repository`,
  `pull_repository`, `checkout_repository`, `sync_repository`, and their bulk
  `*_repositories` workflows); build reads and mutations (`list_build_targets`,
  `get_build_capabilities`, `build_service`, `build_services`); and library reads
  and mutations (`list_libraries`, `get_library_dependents`,
  `verify_library_artifact`, `inspect_version_alignment`, `build_library`,
  `install_library`, `sync_library`, `prepare_library`, `prepare_libraries`),
  plus task and log reads (`get_task`, `summarize_workflow`, `get_task_logs`)
  and cancellation (`cancel_task`, `cancel_workflow`). Each maps onto an
  existing application use case; bulk Git sync, bulk builds, and library
  preparation submit one coordinated core operation. No destructive Git
  operation is exposed. `get_build_capabilities` is the one query this interface
  required from the core: a read-only projection of the build adapter's provider
  registry behind the `BuildCapabilityIndex` port
  (`crates/application/src/ports/build.rs`), so the GUI renders a build system's
  operations without hard-coding Maven or npm.
- **One connection, one client.** A shared `AppState` owns a single client
  behind a connect lock, so a burst of commands cannot start several daemons.
  The adapter resolves the real `service-orchestrator` binary (explicit
  override, `PATH`, sibling, `target/{debug,release}` walk-up) and exports it
  through `SERVICE_ORCHESTRATOR_DAEMON_BIN`, so the GUI's own executable is
  never re-executed as a daemon.
- **Connection state machine.** The adapter — not the frontend — tracks
  `Connecting`/`Connected`/`Reconnecting`/`Disconnected`/`Error`, classifies
  probe and command outcomes, and pushes only transitions as
  `daemon://connection` events. Passive probing is guarded by a reachability
  check, so an idle GUI does not issue probes against a stopped daemon; a
  daemon is started only by an explicit action or by the shared client's single
  reconnect-retry, exactly as the TUI and MCP behave while attached.
- **Mutations pass through the policy engine.** Starting, stopping, and
  restarting a service, Git synchronization, builds, and library preparation are
  `SafeWrite` operations submitted with the `Desktop` client kind, exactly as
  the TUI and MCP submit their own kinds; a confirmation dialog in the webview
  is a usability affordance and never substitutes for the daemon's
  authorization. Protected operations such as applying migrations forward the
  confirmation as intent (`confirmed: true`) and are upgraded to
  `Authorization::allow_destructive()` by the daemon only when all three trust
  signals hold — the request asks for it, the declared client kind is
  interactive, and the kernel-verified peer UID matches the daemon's
  (`crates/daemon/src/dispatch.rs`). A refusal surfaces as the structured
  `PermissionDenied` code. The frontend's pending state and the daemon's own
  operation coordination together prevent duplicate requests (`tasks/019.md`
  §28, `tasks/020.md` §31, `tasks/021.md` §8).
- **Migration, task, health, and log commands.** The adapter additionally
  exposes migration reads and task-producing operations (`list_migrations`,
  `get_migration_status`, `validate_migration`, `inspect_migration_status`,
  `inspect_migration_pending`, `preview_migration`, `apply_migration`,
  `apply_selected_migrations`), task browsing and cancellation (`list_tasks`,
  `get_workflow`, plus the existing `get_task`/`cancel_task`/`cancel_workflow`),
  health reads and checks (`list_health`, `get_health`,
  `check_service_health`, `check_service_readiness`), and unified logs
  (`get_logs` with task/service/workspace scope, cursor pagination, and
  truncation flags; `subscribe_service_logs`/`unsubscribe_service_logs` with
  `daemon://log` events). Each maps onto an existing core capability: there is
  no second migration manager, health manager, task engine, or log storage, and
  the TUI and MCP observe the same authoritative state (`tasks/021.md` §2, §43).
- **Forwarded events.** The adapter subscribes to the daemon's task-event
  stream and re-emits events verbatim as `daemon://task`, plus a synthetic
  `Lagged` payload when the subscription falls behind; the frontend never polls
  for task state. Per-service log subscriptions are forwarded as `daemon://log`
  (`entry`, `lagged`, `closed`). The task stream carries task, log, and
  shutdown events only, so the GUI refreshes service runtime/health state when a
  task settles (or after a command) rather than on a dedicated service event —
  a documented limitation of the current daemon event surface
  (`docs/desktop-gui.md` §26).
- **Minimal capabilities.** The webview is granted `core:default` only — no
  shell, filesystem, HTTP, or process plugin — behind a strict CSP and a
  navigation filter that accepts the `tauri:` scheme plus the Vite dev server in
  debug builds only.
- **Typed frontend.** A single typed API module wraps `invoke`/`listen`; React
  contexts hold connection, workspace, and task-event state; presentation
  components contain no business rules; the TypeScript build is strict with no
  `any`.

## 26. Related documents

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
- `docs/adr/011-migration-manager.md` — the decision record for the provider-based
  Migration Manager, database-target safety, and coordination.
- `docs/adr/012-health-observability.md` — the decision record for the Health
  Manager, its providers and aggregation, and the observability layer.
- `docs/adr/013-ratatui-tui.md` — the decision record for the Ratatui TUI, its
  state machine, async event loop, and policy-driven confirmation flow.
- `docs/adr/014-mcp-integration.md` — the decision record for the MCP server: the
  `rmcp` SDK choice, the thin adapter, structured envelopes, policy enforcement,
  resources and prompts, and the documented gaps.
- `docs/adr/015-shared-local-runtime.md` — the decision record for the shared
  local runtime: why a local daemon, why Unix domain sockets, the preserved
  single-binary model, shared state, process ownership, authorization, and the
  trade-offs.
- `docs/adr/016-environment-workflows.md` — the decision record for end-to-end
  environment workflows: why orchestration belongs in the application core, why
  the task engine is reused, dependency closure, planning versus execution,
  precondition revalidation, and the trade-offs.
- `docs/adr/017-workspace-validation.md` — the decision record for read-only
  workspace validation: the `WorkspaceProbe` port, defence-in-depth checks, and
  the configuration-conflict versus port-occupancy distinction.
- `docs/adr/018-tauri-desktop-gui.md` — the decision record for the Tauri 2
  desktop GUI: the thin adapter over the client boundary, the separate Cargo
  workspace, the read-only command set, the connection state machine, the
  daemon-binary resolution that keeps the GUI from being re-executed as a
  daemon, and the minimal capabilities.
- `docs/adr/019-desktop-service-management.md` — the decision record for desktop
  service management: validated lifecycle commands over the shared client, the
  normalized operation outcome, bulk as one daemon workflow, the policy engine
  as the sole authority, task-event-driven refresh, stable-id selection, and
  disconnection that never replays uncertain writes.
- `docs/adr/020-desktop-git-build-library.md` — the decision record for desktop
  Git, build, and library management: the read-only build-capability index,
  configuration-derived repository lists with on-demand inspection, one
  coordinated core operation per bulk action (no frontend loop), the read-only
  sync preview with no silent discard of local changes, provider-agnostic build
  capabilities, read-only version alignment, and the shared task-monitoring and
  log components.
- `docs/environment-workflows.md` — environment preparation: the request and its
  defaults, selection precedence, dependency closure, planning, the execution
  phases, cancellation and failure handling, aggregated results, and TUI and MCP
  usage.
- `docs/application-core.md` — use cases, ports, request/response models, and
  the runtime-state approach.
- `docs/local-daemon.md` — the shared local runtime: architecture, IPC protocol,
  daemon lifecycle, client connection and auto-start, process ownership, task
  state, event streaming, reconnection, security, shutdown, and limitations.
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
- `docs/migration-manager.md` — the Migration Manager: provider abstraction,
  Liquibase execution modes, database-target identification and safety,
  coordination, failure semantics, and limits.
- `docs/health-manager.md` — the Health Manager: providers, state model,
  aggregation, readiness, configuration, polling, events, history, and
  diagnostics.
- `docs/observability.md` — the Observability layer: the structured log model,
  ingestion and correlation, filtering, cursor pagination, retention, streaming,
  backpressure, and redaction.
- `docs/tui.md` — the Ratatui TUI: screens, keyboard model, multi-selection,
  policy-driven confirmations, async event loop, and terminal lifecycle.
- `docs/mcp.md` — the MCP server: stdio connection model, tools, resources,
  prompts, permissions, observability, long-running work, example workflows,
  testing, and limitations.
- `docs/desktop-gui.md` — the Tauri 2 desktop GUI: the shell, service lifecycle,
  Git, build, and library screens, the command set, the two event streams, the
  connection model, the daemon-binary resolution, security controls, shared task
  monitoring and logs, performance, development workflow, testing, and
  limitations.
- `docs/domain-model.md` — the domain entities, relationships, and invariants.
- `docs/workspace-validation.md` — workspace validation: scope, read-only
  guarantees, the individual checks, diagnostic codes, and safe execution.
- `docs/performance-report.md` — measured performance at the documented service
  scales, methodology, and bottlenecks.
- `docs/reliability-report.md` — the reliability audit, defects found and fixed,
  regression tests, and remaining risks.
- `AGENTS.md` — the full engineering guidance and product philosophy.

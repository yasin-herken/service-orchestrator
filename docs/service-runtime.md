# Service Runtime Management

The Service Runtime layer manages the **local execution lifecycle** of backend
and frontend services. It turns a validated service into a structured start plan
and drives the process manager to run, stop, and restart it — without owning
process execution or scheduling.

This document describes the `service-orchestrator-runtime` infrastructure crate:
the runtime abstraction, the Java and Node runtimes, the registry, the Service
Manager, lifecycle semantics, events, errors, and macOS behavior. The decisions
behind it are recorded in `docs/adr/010-service-runtime.md`.

---

## 1. Where it sits

The runtime adapter is **infrastructure**. It implements the process execution
for the service lifecycle task kinds and a small read-only application port. It
never contains business rules and never touches an interface.

```text
TUI / MCP / CLI
       │
       ▼
Application core  (use cases: start_service, stop_service, restart_service,
       │           get_service_runtime_status, list_running_services, bulk ops)
       ▼
Task Engine ──> ServiceRuntimeTaskExecutor (start / stop / restart)
       │
       ▼
Service Manager ──> Runtime Registry ──> JavaRuntime / NodeRuntime
       │                                   │
       │                                   └── StartPlan (data, not behaviour)
       ▼
Process Manager  (the only owner of OS processes)
       │
       ▼
Local Java / Node processes
```

Three seams stay separate (`AGENTS.md` §11, §15, §21, `tasks/010.md` §3):

| Seam | Decides |
| --- | --- |
| **Service Runtime** | *how* a service starts: runtime, executable, working directory, environment, shutdown strategy |
| **Process Manager** | how processes are spawned, tracked, signalled, terminated, and logged |
| **Task Engine** | when operations run, how they are scheduled, how cancellation and state propagate |

No runtime spawns a process. No manager schedules a task. The process manager
remains the single process owner.

---

## 2. Runtime abstraction

A runtime is the single place where runtime-specific knowledge lives, so adding
Python, Go, Docker, or Kubernetes is a new runtime and a registry line — not a
change to orchestration, the task engine, the process manager, the application,
or the interfaces (`tasks/010.md` §5, §6).

```rust
pub trait ServiceRuntime: Send + Sync {
    fn runtime_type(&self) -> RuntimeType;
    fn validate(&self, service: &Service) -> Result<(), ServiceRuntimeError>;
    fn prepare_start(
        &self,
        service: &Service,
        working_dir: &AbsolutePath,
        environment: Environment,
    ) -> Result<StartPlan, ServiceRuntimeError>;
}
```

- `validate` checks the runtime-specific preconditions that can be checked
  without running anything.
- `prepare_start` returns a **`StartPlan`** — program, structured arguments,
  absolute working directory, full environment, and shutdown strategy. A plan is
  *data*: the runtime builds it and the manager hands it to the process manager.
- `runtime_type` identifies the runtime for resolution, version checks, and
  events.

The trait has no `stop`/`restart`/`spawn`: those are manager concerns. It also
has **no capability system** — the two current runtimes support the same
lifecycle, and a speculative capability set would be unused surface
(`tasks/010.md` §7).

### StartPlan

```rust
pub struct StartPlan {
    pub runtime: RuntimeType,
    pub program: String,
    pub args: Vec<String>,
    pub working_dir: AbsolutePath,
    pub environment: Environment,
    pub shutdown: ShutdownStrategy,
}
```

`StartPlan::command_identity()` renders the program and its structured
arguments **without any environment value**, so a startup command can be
surfaced through an event or a task result without leaking a credential
(`AGENTS.md` §31).

---

## 3. Java runtime

`JavaRuntime` handles Java/Spring Boot services.

- The start command is **never hard-coded**; it comes from validated
  configuration (`tasks/010.md` §8). Typical values are
  `./mvnw spring-boot:run` or `java -jar target/application.jar`.
- **Maven Wrapper preference.** When the configured start program is `mvn` and a
  `mvnw` wrapper exists in the working directory, the wrapper is preferred, so a
  developer's commits pin their Maven version (`tasks/010.md` §8). A configured
  `./mvnw` is used as-is.
- The Java toolchain version is probed by the manager through the
  `RuntimeInspector` seam and compared with the version the service pins; it is
  not the runtime's concern.

Validation performed for a Java service (manager + runtime):

- the service and workspace exist; the working directory exists and is inside
  the workspace root;
- the runtime resolves and the service declares a usable start command;
- the pinned Java major version matches the installed one, when both are known;
- configured ports are free.

The runtime never installs Java and never builds the project: building belongs to
the Build Manager, and installing a toolchain is out of scope (`tasks/010.md`
§9).

---

## 4. Node runtime

`NodeRuntime` handles Node.js / React services.

- The start command is configuration-driven (`npm start`, `npm run dev`, ...);
  nothing is hard-coded (`tasks/010.md` §10).
- Node and npm executables are resolved by the manager's `RuntimeHost` across
  version managers (see §5), so a service runs under the developer's effective
  Node.
- The runtime never runs `npm install` on start; dependency installation belongs
  to the Build Manager (`tasks/010.md` §11).

Validation mirrors Java: the service, workspace, working directory, start
command, and pinned Node major version are checked before spawning.

### macOS executable and version-manager resolution

`SystemRuntimeHost` resolves a bare program name (`java`, `node`, `npm`) without
assuming an interactive shell has initialized the environment
(`tasks/010.md` §12). Search order:

1. every directory on `PATH`;
2. `$JAVA_HOME/bin` (for Java);
3. common version-manager locations under `$HOME`:
   `.volta/bin`, `.asdf/shims`, `.local/share/fnm/aliases/default/bin`,
   `.sdkman/candidates/java/current/bin`, `.jenv/shims`;
4. the newest `~/.nvm/versions/node/*/bin`;
5. the system directories `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`,
   `/bin`.

A program that already contains a `/` (for example `./mvnw`) is returned
unchanged. Environment values are read through the same host so runtimes and
the manager stay testable. Version probing is separate (see §7) because it runs
a process.

---

## 5. Runtime registry

`RuntimeRegistry` is a small, explicit map from `RuntimeType` to runtime. It is
**not** a plugin framework: there is no discovery, lifecycle, or dynamic
loading (`tasks/010.md` §6).

- `RuntimeRegistry::standard()` registers `JavaRuntime` and `NodeRuntime`.
- `register(runtime)` replaces any runtime of the same type, which is how tests
  inject a fake runtime.
- `resolve(service)` selects a runtime, in order (`tasks/010.md` §6):
  1. an explicitly declared runtime requirement — `java`/`maven` imply Java,
     `node`/`npm` imply Node;
  2. the service's build system — `maven` implies Java, `npm` implies Node;
  3. the start command's program — `java`/`mvn`/`mvnw` imply Java;
     `node`/`npm`/`npx`/`yarn`/`pnpm` imply Node.
- No match yields a structured `UnsupportedRuntime` error, never a guess.

`RuntimeType` is a closed enum of the supported runtimes. A future runtime is a
new variant, a new `ServiceRuntime` implementation, and one `standard()` line —
the application, engine, process manager, TUI, and MCP are unchanged. Docker and
Kubernetes fit here later (see ADR 010).

---

## 6. Service Manager

`ServiceManager` is the infrastructure coordinator. Given a workspace-aware
`ResolvedServiceTarget` it:

1. validates the target's path is present and inside the workspace root;
2. resolves the configured workspace and service;
3. applies **start idempotency** (see §8);
4. resolves the runtime and validates the service;
5. probes the runtime version and compares it with the pinned version;
6. checks configured ports for conflicts (see §10);
7. checks cancellation (see §11);
8. resolves the working directory and merges the environment;
9. builds the `StartPlan`, publishes `ServiceStarting`, spawns through the
   process manager, records ownership **synchronously**, and publishes
   `ServiceStarted`.

Stop and restart reuse the same coordinator lock per service, so operations on
one service serialize while different services stay concurrent.

### Working directory, environment, secrets

- The command-level relative working directory wins over the service directory,
  and the result must exist and be inside the workspace root (`tasks/010.md`
  §23).
- The environment merges workspace, service, and start-command variables, with
  later layers overriding earlier ones (`tasks/010.md` §22).
- Lifecycle **tasks carry only non-secret identity and paths** — never
  environment values. The manager reads the full environment from validated
  configuration at execution time and hands it straight to the process manager,
  which redacts secret values whenever a specification is displayed or
  serialized (`AGENTS.md` §31). No secret reaches a task result, event, or
  interface.

---

## 7. Version probing (RuntimeInspector)

Whether a compatible toolchain is installed is an environment question answered
by a separate asynchronous seam, so a start only pays for probing when version
compatibility matters (`tasks/010.md` §9, §12):

```rust
#[async_trait]
pub trait RuntimeInspector: Send + Sync {
    async fn inspect(&self, runtime: RuntimeType) -> Result<RuntimeInfo, ServiceRuntimeError>;
}
```

- `NullRuntimeInspector` is the default: it reports "unknown" without spawning
  anything, so most starts are free.
- `ProcessRuntimeInspector` resolves the toolchain through the `RuntimeHost`,
  subscribes to the process manager's event stream **before** spawning, runs
  `java -version` / `node --version` with a bounded timeout, and parses the first
  version-like token. It returns `Ok` with an unknown version when the toolchain
  cannot be found; the start itself then surfaces a precise spawn failure.
- The manager compares only the **major** version, and only when the service
  pins one. Legacy Java (`1.8.0` → 8) is handled. A mismatch fails with
  `RuntimeVersionMismatch` before any process is spawned.

The process manager is the only component that runs processes, so probing goes
through it — the runtime adapter never builds `std::process::Command` itself
(`AGENTS.md` §15, §32).

---

## 8. Lifecycle semantics

### Start

- **Idempotency.** Starting an already-`Running` service returns the existing
  process (`already_running == true`) and spawns nothing (`tasks/010.md` §14).
- **Concurrency.** A per-service async lock serializes operations, so two start
  requests (say TUI and MCP) racing for the same service produce exactly one
  process; operations on different services run concurrently.
- A service in `Starting`/`Stopping` yields `ServiceAlreadyStarting` rather than
  an unsafe overlap.
- A `StartService` operation performs **only** the start: no Git sync, no Maven
  build, no `npm install`, no Liquibase (`tasks/010.md` §13, §39). Those belong to
  higher-level workflows.

### Stop

- Stopping a service that is not running is a successful no-op
  (`already_stopped == true`), not a fatal error (`tasks/010.md` §16).
- A real stop publishes `ServiceStopping`, requests a **graceful** stop through
  the process manager (which escalates to force-kill after its grace period),
  awaits confirmed exit, updates state, removes the managed record, and returns
  the exit code and reason (`tasks/010.md` §15).
- The runtime adapter never sends OS signals directly; it uses the process
  manager's `stop`/`wait` (`tasks/010.md` §15, §33).

### Restart

- A restart is a stop followed by a start **under one lock**, and the process is
  awaited to a terminal state before the replacement starts, so two instances
  never overlap (`tasks/010.md` §17). A failed stop prevents the start.

### Startup task versus managed process

A start task is short; the process it starts is not (`tasks/010.md` §19). The
manager records ownership synchronously, before the first `await`, and spawns a
detached exit watcher. The startup task therefore **succeeds as soon as the
process is confirmed started**, and the managed process keeps running under the
process manager's supervision. The engine never sees a long-running service as
an unfinished task. Stopping requires an explicit `StopService` operation.

---

## 9. Process ownership and state

The manager tracks every process *it* started (`tasks/010.md` §20):

- workspace, service id, managed process id, the process `ProcessHandle`, the
  runtime, the redacted startup command identity, the start timestamp, live
  state, and exit information.

Live state is derived from the handle's snapshot so it is never stale. The
registry never spawns, signals, or reaps — those belong to the process manager.

On unexpected exit the watcher updates the observed state, preserves the exit
information, and publishes `ServiceFailed`/`ServiceExited`; the failure remains
queryable. Services are **never restarted automatically** (`tasks/010.md` §25).
A live process does not imply a healthy service; health checks are a later task
(`tasks/010.md` §18).

The manager also publishes a plain `ProcessState` into the application's
`RuntimeStateStore` as a fallback, while the rich state flows to interfaces
through the read-only `ServiceRuntimeIndex` port (§13).

---

## 10. Port conflicts

Before starting, the manager checks each configured port through the
`PortChecker` seam (`tasks/010.md` §21). `SystemPortChecker` opens a TCP
connection to localhost and treats a *successful* connection as "in use";
anything else — including a filtered-port timeout — is treated as free, so the
check never blocks a legitimate start on a false positive.

A conflict returns `PortAlreadyInUse { service, port }` with the affected port.
The occupying process is **never killed or reclaimed**: a process on a port is
not assumed to belong to Service Orchestrator, and a port conflict is never
silently treated as a successful start (`tasks/010.md` §20, §21, §33).

---

## 11. Cancellation semantics

Cancellation is cooperative (`tasks/010.md` §26):

- **Cancelling a start that has not spawned** leaves nothing behind: the manager
  checks the token after validation and port checks, and returns
  `OperationCancelled`. The manager has no `await` between the synchronous spawn
  and recording ownership, so a cancelled start future can never leave a spawned
  process untracked.
- **After a service has started**, cancelling the (now completed) startup task
  does **not** stop the service. The startup task and the managed process have
  separate lifecycles; stopping requires `StopService`.
- The executor maps `OperationCancelled` to a cancelled task attempt, so the
  engine records a `cancelled` task rather than a failure.

---

## 12. Events

The runtime adapter publishes an interface-neutral, service-scoped vocabulary
over a bounded broadcast bus (`tasks/010.md` §27):

```text
ServiceStarting { workspace, service, runtime, command }
ServiceStarted  { workspace, service, process_id, pid, runtime }
ServiceStopping { workspace, service, process_id }
ServiceStopped  { workspace, service, process_id, exit_code, exit_reason }
ServiceFailed   { workspace, service, process_id, message }
ServiceExited   { workspace, service, process_id, exit_code, exit_reason }
```

The process manager's bus is process-scoped and crate-private, so the adapter
uses a small parallel bus carrying the workspace/service context an interface
needs to render "auth-service started" without joining a process id back to a
service. Delivery is in-process and bounded; the managed registry stays
authoritative, so a subscriber that lags can re-read current state. The bus is
reused rather than duplicated (`tasks/010.md` §27).

---

## 13. Application integration

The application reaches the runtime through the same task engine as every other
mutating operation (`tasks/010.md` §29):

- `start_service`, `stop_service`, `restart_service` validate preconditions
  (service exists, supports running, has a start command), apply policy
  (`SAFE_WRITE`), and submit a `StartService`/`StopService`/`RestartService`
  task carrying only a non-secret `ResolvedServiceTarget`.
- `get_service_runtime_status` and `list_running_services` are read-only
  queries. They read live managed-process state through the small
  `ServiceRuntimeIndex` port (implemented by the manager), and fall back to the
  last observed `RuntimeStateStore` value when no adapter is wired. The default
  `NullServiceRuntimeIndex` keeps the application constructible.
- Results are structured application values (`ServiceRuntimeStatus`,
  `OperationResult`) — never TUI-formatted strings and never MCP types
  (`tasks/010.md` §29).

### Bulk operations

`start_services`, `stop_services`, and `restart_services` compose the same task
kinds into one workflow per selection (`tasks/010.md` §30):

- concurrency is the engine's configured `max_parallel_tasks` — there is no
  second scheduler;
- steps are independent, so a partial failure is isolated and each step's
  result is reported separately;
- cancellation and per-service coordination are the engine's and the manager's,
  unchanged;
- startup dependency ordering reuses the dependency graph where runtime
  dependencies are explicitly modelled; build and runtime dependencies are not
  treated as interchangeable.

The Service Manager applies one additional rule for bulk guards: positions in
the workflow are expressed as step dependencies, and the manager still enforces
per-service serialization at execution time.

---

## 14. Errors

Failures are structured and interface-neutral (`tasks/010.md` §32). Every variant
carries the context an interface needs and a stable machine-readable `code()`:

| Variant | Code |
| --- | --- |
| `WorkspaceNotFound` | `RUNTIME_WORKSPACE_NOT_FOUND` |
| `ServiceNotFound` | `RUNTIME_SERVICE_NOT_FOUND` |
| `UnsupportedRuntime` | `RUNTIME_UNSUPPORTED` |
| `RuntimeNotFound` | `RUNTIME_NOT_FOUND` |
| `RuntimeVersionMismatch` | `RUNTIME_VERSION_MISMATCH` |
| `StartCommandMissing` | `RUNTIME_START_COMMAND_MISSING` |
| `WorkingDirectoryInvalid` | `RUNTIME_WORKING_DIRECTORY_INVALID` |
| `PortAlreadyInUse` | `RUNTIME_PORT_ALREADY_IN_USE` |
| `ServiceAlreadyStarting` | `RUNTIME_ALREADY_STARTING` |
| `ProcessStartFailed` | `RUNTIME_PROCESS_START_FAILED` |
| `ProcessStopFailed` | `RUNTIME_PROCESS_STOP_FAILED` |
| `ProcessExitedUnexpectedly` | `RUNTIME_PROCESS_EXITED` |
| `OperationCancelled` | `RUNTIME_OPERATION_CANCELLED` |
| `OperationTimeout` | `RUNTIME_OPERATION_TIMEOUT` |
| `InvalidConfiguration` | `RUNTIME_INVALID_CONFIGURATION` |
| `MissingParameter` | `RUNTIME_MISSING_PARAMETER` |
| `InvalidParameter` | `RUNTIME_INVALID_PARAMETER` |
| `Internal` | `RUNTIME_INTERNAL_ERROR` |

The task executor maps these onto the engine's error vocabulary: cancellation
becomes a cancelled attempt, `Internal` stays internal, and every other failure
becomes a structured task failure with the code above. No variant ever includes
a secret value; command identities contain only the program and its arguments.

---

## 15. Security

- Startup commands come from validated configuration, never from untrusted input
  (`tasks/010.md` §33).
- Arguments are structured (`Program` + `Vec<args>`), never a shell string
  (`AGENTS.md` §32).
- The runtime adapter cannot bypass the process manager and never sends OS
  signals itself.
- External processes are never killed automatically — not even when they occupy
  a configured port.
- Secret environment values are never logged, and never included in task
  results, events, errors, or configuration output (`AGENTS.md` §31).

Configured startup commands are trusted developer-workspace operations, not
sandboxed code.

---

## 16. macOS behavior

- Processes are spawned through the process manager, which runs each in its own
  process group and signals the group, so a shell that spawned children leaves
  no orphans (`docs/process-manager.md`).
- Executable resolution searches `PATH`, `JAVA_HOME`, and the version-manager
  locations in §4 without assuming interactive shell initialization.
- `PATH`/`HOME`/`JAVA_HOME` are read through the `RuntimeHost` seam, so behavior
  is testable with a fake host and does not depend on the terminal that launched
  the application.
- No Linux-only assumption (such as `setsid` semantics) is baked into the
  runtime adapter; group/signal handling stays in the process manager.

---

## 17. Testing

Unit tests cover runtime selection, runtime validation, Java and Node plan
generation (including Maven-wrapper preference), unsupported runtimes, state
derivation, port-check conservatism, and cancellation mapping.

Integration tests in `crates/runtime/tests/` run real, tiny local processes
(a `sleep`, or `/bin/sh -c 'exit 3'`) — never Java, Node, Maven, or npm — and
exercise start, stop, restart, idempotency, concurrent-start protection, port
conflicts, version mismatch, cancellation, unexpected exit, and the full task
engine flow end to end. The application tests verify the runtime queries and the
bulk operations against the real in-memory state store.

# ADR 004: A shared application core for every interface

- **Status:** Accepted
- **Date:** 2026-10-07
- **Deciders:** Service Orchestrator maintainers

## Context

Service Orchestrator exposes the same developer workflows through two very
different interfaces: a Ratatui terminal UI for humans and an MCP server for AI
agents. Both need to answer the same questions ("what services exist?", "build
this service", "prepare this profile") and perform the same operations
(synchronize, build, migrate, start, stop, inspect logs, run health checks).

If each interface implemented those operations itself, three problems follow:

1. **Behavior drifts.** The TUI and the AI agent would not agree on
   preconditions, ordering, or results, and every new capability would be built
   twice.
2. **Safety is bypassable.** A destructive operation guarded in the TUI could be
   invoked unfiltered through MCP. The permission model must not depend on which
   interface calls it.
3. **Testing is coupled to a UI.** Verifying orchestration would require driving
   a terminal or an MCP client.

The architecture (ADR 001) already places `application` between the interfaces
and the core services, and the configuration ADR (003) defines the validated
`Config` the application consumes. This ADR records what the application layer
*is*, what it owns, and how it stays independent.

## Decision

Introduce `service-orchestrator-application` as the single shared application
core. Every interface depends only on it; it contains all business
orchestration and depends inward on `domain`, `execution`, and `policy`.

### TUI and MCP are adapters

```text
tui ─┐
     ├──> application ──> execution ──> domain
mcp ─┘              └───> policy    ──> domain
```

The TUI reads application state, renders it, and turns user intent into
application calls. The MCP server validates tool input, applies the same policy,
and forwards to the same use cases. Neither contains Git, build, process, or
workflow logic, and the application never depends on `ratatui`, `rmcp`, or any
interface type.

### Commands and queries are separated conceptually

Use cases are split into side-effect-free **queries** and mutating **commands**.
This is a logical separation, not a distributed CQRS system:

- A status query cannot accidentally fetch a repository, start a process, or
  build anything; it reads configuration and last observed runtime state.
- A command validates preconditions, applies policy, builds a task or workflow
  definition, and submits it.

The split keeps the read path predictable and the write path auditable without
introducing buses, projections, or eventual consistency.

### Work is described, not executed

Commands do not run infrastructure. They construct a `TaskDefinition` (or a
`WorkflowDefinition` composed of steps with prerequisites) and submit it to the
`TaskManager`. The task-engine contract lives in the `execution` crate so the
engine owns its API and there is no dependency back-edge from `execution` to
`application`.

Application use cases request work; the task engine executes it. Scheduling,
bounded concurrency, retry, timeout, and failure propagation remain future
engine work.

### Infrastructure is abstracted through ports — but only where needed

Ports are declared only where the application currently needs a boundary:

- `TaskManager` (in `execution`) — submit and observe tasks/workflows;
- `RuntimeStateStore` — transient service runtime state;
- `LogService` — bounded, structured service logs.

There is deliberately **no** `GitService`, `BuildService`, `ProcessService`,
`LiquibaseService`, or `HealthService` trait yet, because the application does
not call those subsystems directly — it requests tasks and the engine reaches
the adapters. Ports are introduced when a real boundary exists, not for every
noun (AGENTS.md §25).

Dependencies are injected through the constructor with plain composition.
There is no dependency-injection framework.

### Runtime state is separate from configuration

Two kinds of state are never mixed:

- **Persistent state** — workspaces, services, libraries, groups, profiles,
  commands, dependencies, ports, health-check definitions. This is the validated
  `Config` loaded once; the application never parses files.
- **Runtime state** — last observed process, health, and Git state, and the
  checked-out branch. This lives in the focused `RuntimeStateStore` port.

The MVP implementation is `InMemoryRuntimeStateStore`: dependency-free, easy to
share across threads, and not persisted. There is no database. A future
process-manager-backed store implements the same port, keeping the change local
to infrastructure.

Commands optimistically transition runtime state after submitting work (for
example, marking a service `Starting`); queries only read the last observed
state.

### Policy is applied inside the use case

Every mutating command calls `authorize` before scheduling. The pure
`PolicyEngine` (from the `policy` crate) turns the operation's `Permission` and
the caller's `Authorization` into a decision: `Read`/`SafeWrite` proceed;
`Destructive` requires explicit authorization. A denial becomes
`ApplicationError::PermissionDenied`.

Because the check is inside the use case, an interface cannot bypass it by
calling the operation directly. Interfaces may additionally call `authorize`
themselves to decide whether to prompt for confirmation.

### Structured results, never formatted strings

Results are application/domain values (`OperationResult`, `ServiceStatus`,
`WorkspaceStatus`, `Vec<LogEntry>`), not `String` or `Vec<String>`. They derive
`serde` so the TUI and MCP format or serialize them independently without
re-deriving business meaning.

`OperationResult` distinguishes a scheduled task, a scheduled workflow, and
`AlreadyInDesiredState`, which is how command idempotency is expressed:
starting an already-running service returns a predictable result instead of
scheduling a duplicate process.

## Alternatives considered

1. **Business logic in each interface.**
   Rejected: behavior drifts, safety is bypassable per interface, and every
   capability is built twice. It directly violates the core architectural rule
   (AGENTS.md §1).

2. **One giant `ServiceOrchestrator` trait.**
   Rejected: a single trait for every operation is hard to navigate, hard to
   evolve, and forces unrelated use cases behind one object. Concrete use cases
   on `Application` are simpler and still give MCP a uniform surface.

3. **Full CQRS with separate command/query buses and handlers.**
   Rejected as premature: it adds indirection, dispatch, and eventual
   consistency for no current requirement. A logical split captures the useful
   property (reads are side-effect free) without the machinery.

4. **A trait per infrastructure noun (`GitService`, `BuildService`, ...) now.**
   Rejected as speculative (AGENTS.md §25). The application does not call these
   subsystems directly; the task engine does. Inventing the traits now would
   bake in an interface before the real boundary is known.

5. **Persisting runtime state (a database or on-disk store) in this task.**
   Rejected: runtime state is transient, there is no requirement to survive a
   restart, and a database would add operational weight. The port keeps a future
   choice open.

6. **Returning pre-formatted strings for convenience.**
   Rejected: it couples business meaning to presentation and forces MCP to parse
   text. Structured values keep both interfaces independent.

7. **A third-party dependency-injection framework.**
   Rejected: Rust's type system plus explicit constructor injection is
   sufficient and clearer.

8. **Putting the task contract in `application`.**
   Rejected: the engine owns its API. Placing the contract in `execution` keeps
   the dependency direction inward and avoids a back-edge from `execution` to
   `application` once the engine is implemented.

## Reasoning

- **One source of truth.** TUI and MCP cannot diverge because they call the same
  methods.
- **Safety by construction.** Policy is enforced in the core, not trusted to the
  caller.
- **Testability without interfaces.** Use cases run against fakes for the task
  engine and logs and the real in-memory runtime store — no terminal, no MCP
  client, no Maven/Git/processes.
- **Clean boundaries.** Ports exist only where the application genuinely needs a
  seam, and the task engine owns execution.
- **Structured contracts.** Machine-readable results serve the TUI, MCP, and
  tests equally.
- **Honest scope.** No scheduler, no adapters, no UI. This task fixes the
  contracts those future tasks will implement.

## Consequences

### Positive

- Adding an interface (CLI, HTTP API) requires no core change.
- Preconditions, policy, and idempotency are defined once and tested once.
- The engine and infrastructure can be implemented later behind stable ports.
- Queries are provably side-effect free, so status reads are safe and cheap.

### Negative / trade-offs

- An extra layer and an extra crate to maintain; simple operations still route
  through the core.
- Commands optimistically update runtime state, which is a best-effort view
  until the process/health adapters publish authoritative observations.
- `TaskManager` and the runtime/log ports live in three different places
  (`execution`, `application`, `application`); this is intentional but means the
  boundary is spread across crates.
- No real dependency-aware scheduling exists yet: workflows carry prerequisites,
  but the engine that honors them is future work.
- `PrepareEnvironmentRequest.branch` is recorded in step descriptions but is not
  yet forwarded as per-step task parameters; wiring structured per-step
  parameters to handlers is deferred until the engine and handlers exist.

## Follow-up

- Implement the task/workflow engine in `execution` (scheduling, concurrency,
  retry, timeout, cancellation, progress).
- Implement infrastructure adapters (`git`, `process`, `build`, `liquibase`,
  `health`, `logging`) that publish runtime state and implement `LogService`.
- Replace `InMemoryRuntimeStateStore` with a process-manager-backed store behind
  the same port, or keep it as the process manager's view.
- Forward `PrepareEnvironmentRequest` per-step parameters (branch, timeouts) to
  task handlers once the engine exposes a parameter channel.
- Build the TUI and MCP adapters strictly on top of the use cases defined here.

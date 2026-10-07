# ADR 005: The task engine and scheduler

- **Status:** Accepted
- **Date:** 2026-10-07
- **Deciders:** Service Orchestrator maintainers

## Context

Every meaningful operation in Service Orchestrator is long-running and shared
across interfaces: a Git sync, a Maven build, an npm install, a Liquibase
migration, starting or stopping a process, a health check, or a composed
workflow. The TUI, the MCP server, and a future CLI all need to start that work,
observe its progress, and cancel it.

ADR 001 placed a task/workflow engine in the `execution` crate, between the
application and the domain, and ADR 004 fixed the `TaskManager` contract the
application submits work through. This ADR records how that engine is actually
built: what it owns, how scheduling and execution are separated, and the
concrete choices for state, concurrency, cancellation, and events.

The requirements that shaped the decision:

- the engine must not depend on `ratatui`, `rmcp`, Git, Maven, Node, Liquibase,
  or any OS-specific UI or process behaviour;
- exactly the states `queued`, `running`, `succeeded`, `failed`, `cancelled`,
  `skipped`, `blocked` must be supported, with valid transitions enforced;
- scheduling must honour dependencies and a bounded concurrency limit;
- cancellation, retry, timeout, progress, events, backpressure, and graceful
  shutdown must all be supported;
- no real infrastructure and no database may be introduced.

## Decision

Implement the task engine as the `service-orchestrator-execution` crate,
depending only on `service-orchestrator-domain` plus `tokio` and `async-trait`.
`TaskEngine` implements the application's `TaskManager` trait and adds a richer,
interface-neutral query and event API.

### Why the engine is a separate component

The engine is separated for three reasons:

1. **One execution model, many callers.** If the TUI and MCP each implemented
   scheduling, retry, and cancellation, behaviour would drift and every
   capability would be built twice (the core rule of ADR 001/004).
2. **Inward dependency direction.** The engine knows tasks generically and
   nothing about how a task is performed. It depends only on the domain, so it
   can never leak interface or infrastructure concerns into the core.
3. **Testability without infrastructure.** Because concrete work is injected,
   the engine is fully testable with fake executors — no terminal, no MCP
   client, no Maven, no Git, no processes.

### Scheduler vs executor responsibilities

This is the central rule:

- The **scheduler** answers *when* a task runs. It selects ready tasks, applies
  dependencies and the concurrency limit, and drives the state machine. It
  contains no `match task.kind { ... }` with one branch per operation.
- An **executor** answers *how* a task is performed. It is registered against a
  `TaskKind` and looked up at run time from an `ExecutorRegistry`.

```text
Scheduler (when) ──> TaskExecutor (how) ──> Infrastructure
```

Because executors are registered, adding a new kind of work is a registration,
not a change to the scheduling core.

### In-memory task registry

The engine owns a `TaskStore`: a single `std::sync::Mutex<Inner>` holding every
task's state, a dependency index, inserted order, and submitted workflows. There
is no database and no persistence.

- Completed tasks are intentionally retained so they stay queryable ("recent
  tasks"); eviction/retention is future work.
- The mutex is held only for short, synchronous critical sections and never
  across an `.await`.
- A `tokio::sync::watch<u64>` revision counter lets `wait_for` observe status
  changes without polling; the task read and the projection subscribe happen
  under one lock to avoid a lost-notification race.

This keeps the MVP dependency-free and operationally light, and a future
persistent store could sit behind the same in-memory views without changing the
scheduler.

### Dependency graph model

Dependencies are `TaskId` edges forming a DAG. Each task carries a
`DependencyPolicy`:

- `AllSucceeded` (default) — run only if every dependency succeeded; otherwise
  the dependent is `skipped`;
- `AnyCompleted` — run once every dependency is terminal.

Workflows are expanded eagerly into one task per step, with step prerequisites
translated into task dependencies. The domain validates the workflow's shape
(unique steps, resolvable prerequisites, acyclicity) and `graph::validate`
re-checks the mapped task graph for duplicates, unknown dependencies, and
cycles. A cycle is a static error, so the scheduler can never deadlock on mutual
waits.

The simple two-policy model was chosen deliberately over a policy engine: it
expresses the required "all succeed" and "any completed" semantics without
premature flexibility.

### Concurrency approach

Bounded concurrency is enforced by a single `tokio::sync::Semaphore` sized to
`max_parallel_tasks` (default 4). A permit is acquired before a task is marked
`running` and released when its execution finishes, so
`running_tasks <= max_parallel_tasks` always holds.

The scheduler is a single event-driven loop woken by a `tokio::sync::Notify` —
it never polls. Ready tasks are selected FIFO by insertion sequence, which is
predictable and starvation-free. Priorities and more sophisticated scheduling
are deliberately not implemented.

`max_queued_tasks` provides opt-in backpressure, rejecting submissions that
would exceed the cap with `QueueFull`.

### Cancellation strategy

Cancellation is **cooperative**. Each task owns a `Cancellation` — a one-way
`tokio::sync::watch<bool>`. Cancelling signals the token and wakes the
scheduler; a queued task is checked when admitted and becomes `cancelled`
without running, while a running executor observes the token through its
`TaskContext` and returns `TaskExecutorError::Cancelled` after cleanup.

The engine does **not** drop the executor future out from under a running task.
This preserves the documented cooperative contract and lets an executor release
a child process or handle. The trade-off is explicit: an executor that ignores
cancellation runs to completion; timeouts still bound runaway work and shutdown
waits a bounded time for cooperation. Process termination itself is delegated to
the future process/executor layer via drop guards.

Retry is modelled *inside* one `running` episode, because the domain state
machine has no `failed -> queued` edge. Retry is opt-in (default: one attempt),
so destructive operations are never retried automatically.

### Event propagation

The engine publishes a `TaskEvent` for each meaningful change over a bounded
`tokio::sync::broadcast` channel. Events are interface-neutral domain values;
the TUI, MCP, logging, and future telemetry subscribe independently. Delivery
is in-process only — no broker. A lagging subscriber gets `Lagged` and
resyncs from the authoritative store.

### No runtime ownership

The engine does not create a Tokio runtime. It captures a `Handle` at build
time, so the composition root decides how the runtime is created and kept alive.
`submit` and `cancel` are synchronous and safe to call from a TUI event loop or
an MCP handler; scheduling and execution run on the captured runtime.

### Shutdown

`shutdown()` stops accepting work, cancels the scheduler, signals running tasks,
and waits (bounded to 5 seconds) for them to release their permits. `Drop`
performs the same signal-and-stop without waiting, so dropping the engine never
leaves the scheduler running.

### Duplicate-execution protection

The engine provides identity, states, and metadata, but it does **not** lock
services. Preventing "start auth-service twice" from concurrent TUI and MCP
requests is an *operation coordination* concern that belongs in the application
layer, which owns command preconditions and idempotency (`OperationResult`,
`AlreadyInDesiredState`). The engine is the correct place to *represent* two
distinct tasks; the application is the correct place to *prevent* them. This
boundary is recorded here so it is not implemented in the wrong layer.

## Alternatives considered

1. **Task logic inside the application or each interface.**
   Rejected: duplicates behaviour, bypasses a single execution model, and
   violates the core architectural rule.

2. **A `match task.kind` in the scheduler.**
   Rejected: it couples generic scheduling to concrete operations and forces a
   change to the core for every new task type. A registry keeps it additive.

3. **Owning a Tokio runtime inside the engine.**
   Rejected: it hides the runtime from the composition root, makes shutdown
   ordering harder, and prevents the engine from being built inside an existing
   runtime.

4. **A `tokio-util` `CancellationToken`.**
   Rejected to avoid an extra dependency; a `watch<bool>` wrapper is small,
   sufficient, and fully under our control.

5. **Forcible, drop-based cancellation of running executors.**
   Rejected for the running case: it contradicts the cooperative contract and
   can skip cleanup. Cooperative cancellation plus timeouts and bounded shutdown
   was chosen instead. (Executors may still use drop guards internally.)

6. **A persistent task database.**
   Rejected: runtime task state is transient, there is no requirement to survive
   a restart, and the MVP needs no operational weight. The store is an in-memory
   structure; a future store can implement the same views.

7. **An expressive dependency-policy engine.**
   Rejected as premature. Two explicit policies cover the required semantics.

8. **Retry as a `failed -> queued` state transition.**
   Rejected: it would invent a transition the domain forbids and blur a single
   execution episode into several. Retry stays inside `running`.

9. **A distributed event broker / external bus.**
   Rejected: in-process broadcast is sufficient; events are convenience, and the
   store is authoritative.

10. **Service locking inside the engine.**
    Rejected: coordination of duplicate operations is an application concern;
    the engine should not know what a "service" is beyond a target identifier.

## Consequences

### Positive

- One execution model — scheduling, retry, timeout, cancellation, progress, and
  events — is defined once and reused by every interface.
- The engine is fully testable with fake executors and a deterministic gate; no
  infrastructure is required.
- New task kinds are additive registrations; the scheduler never changes.
- The dependency direction stays inward: `execution -> domain` only.
- Bounded concurrency and backpressure keep a 40–100+ service workspace safe.

### Negative / trade-offs

- The in-memory store retains completed tasks indefinitely until a retention
  policy is added.
- Cancellation depends on executor cooperation; a misbehaving executor is not
  force-killed by the engine.
- FIFO selection has no priorities; a future need for prioritisation will
  require a scheduler change.
- The engine does not prevent duplicate operations; that responsibility is
  deliberately left to the application layer.
- The two dependency policies are simple; richer conditions (for example,
  run-if-any-succeeded) are not expressible yet.

## Follow-up

- Implement concrete executors (`git`, `build`, `process`, `liquibase`,
  `health`) behind the `TaskExecutor` trait and register them in the composition
  root.
- Forward per-step workflow parameters (branch, timeout) now that the engine
  reproduces step metadata on tasks.
- Add a completed-task retention/eviction policy.
- Add service-level operation coordination in the application layer.
- Build the TUI and MCP adapters strictly on the engine's query and event API.

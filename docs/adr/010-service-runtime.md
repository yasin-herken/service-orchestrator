# ADR 010: The Service Runtime

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

Starting a local service is not the same as running a process. A service needs a
runtime (Java or Node), a start command, a working directory, an environment, a
shutdown strategy, a port check, and an ownership story that outlives the task
that started it. ADR 001 and ADR 002 made `Service` and `Runtime` first-class
domain concepts; ADR 005 established the task engine; ADR 006 the process
manager; ADR 007 the Git adapter; ADR 008 the Build Manager; ADR 009 library
management.

`tasks/010.md` shaped this decision with hard requirements:

- a Service Runtime layer separate from process execution, independent of
  Ratatui and MCP;
- Java/Spring Boot and Node/React as the initial runtimes, with the architecture
  open to Python, Go, Docker, and Kubernetes later (but not implemented now);
- a focused runtime abstraction and a small explicit registry, not a plugin
  framework;
- runtimes produce structured process specifications rather than spawning;
- start/stop idempotency and coordinated concurrent starts;
- separate lifecycles for the startup task and the long-running managed process;
- port-conflict detection that never kills the occupying process;
- structured errors, no leaked secrets, no bypassing the process manager;
- `docs/service-runtime.md` and this ADR.

## Decision

Implement the Service Runtime as a new **infrastructure crate**,
`service-orchestrator-runtime`, with a `ServiceManager` that composes a runtime
registry, a runtime host, a runtime inspector, a port checker, the process
manager, and the task engine.

```text
Application use case
      │  (StartService / StopService / RestartService task + target parameters)
      ▼
Task Engine ──> ServiceRuntimeTaskExecutor ──┐
                                            ▼
                                      Service Manager ──> Runtime Registry
                                            │                 ├── JavaRuntime
                                            │                 └── NodeRuntime
                                            │
                                            ├── RuntimeInspector (version probe)
                                            ├── RuntimeHost (executables, env)
                                            └── PortChecker (conflicts)
                                            ▼
                                       Process Manager
```

### Why Service Runtime is separate from the Process Manager

The process manager answers *how an OS process is spawned, tracked, signalled,
terminated, and logged*. It has no notion of a "service", a Java version, a
Maven wrapper, or a configured port. The Service Runtime answers *how a
particular kind of service should be run* — which executable, which working
directory, which environment, which shutdown strategy — and turns that into a
structured plan. Keeping them separate means the process manager stays a
general, reusable process owner and the runtime layer stays free of OS process
plumbing (`AGENTS.md` §11, §15). The runtime adapter never builds
`std::process::Command`; the process manager remains the only process owner.

### Why Java and Node are the initial runtimes

They are the stacks actually in use (Spring Boot 3 + Maven; React 16 +
Node 20.19.4). Implementing exactly two keeps the abstraction honest: every
runtime-independent rule (idempotency, ownership, port checks, cancellation,
events) is proven against both, and no rule is designed around a hypothetical
third. `RuntimeType` is a closed enum, so an unsupported service fails with
`UnsupportedRuntime` rather than guessing.

### Why runtimes generate process specifications

A runtime returns a `StartPlan` (program, structured args, working directory,
environment, shutdown strategy) and nothing else. Plans are *data*: they are
easy to unit-test without spawning, they cannot schedule or signal anything, and
the manager can apply common rules (validation, version checks, port checks,
ownership, events) uniformly before execution. A runtime that could spawn would
duplicate the process manager and scatter process handling across the codebase
(`tasks/010.md` §5, §33).

### Why startup tasks and managed processes have separate lifecycles

A service may run for hours; its startup task must not. The manager records
ownership synchronously, before the first `await`, and spawns a detached exit
watcher, so the startup task succeeds as soon as the process is confirmed
started while the process keeps running under the process manager
(`tasks/010.md` §19). This is also what makes cancellation safe: there is no
`await` between the synchronous spawn and the ownership record, so a cancelled
start future can never orphan a process, and cancelling a *completed* startup
task never stops the service.

### How runtime state is tracked

The managed-process registry (`ManagedProcessRegistry`) holds a cloneable
`ProcessHandle` per started service, keyed by service id, and derives live state
from the handle snapshot so it is never stale. The manager also publishes a
plain `ProcessState` into the application's `RuntimeStateStore` as a fallback.
Interfaces read rich managed-process detail through a new **read-only**
application port, `ServiceRuntimeIndex` (implemented by the manager), so the
application never reaches into the runtime crate. A `NullServiceRuntimeIndex`
keeps the application constructible without the adapter.

### How future Docker/Kubernetes runtimes can be introduced

A runtime is a `ServiceRuntime` implementation plus one `RuntimeRegistry::standard()`
line. `RuntimeType` gains a variant. Nothing in the manager, the task engine,
the process manager, the application, or the interfaces branches on runtime
identity beyond the registry and the closed enum. A future Kubernetes runtime
would still emit a `StartPlan` and still delegate process execution to the
process manager (or a future runtime target), so local and cluster execution
share one orchestration path.

### The new dependency edges

The runtime crate depends on `application` (the port and error vocabulary it
implements/produces), `domain` (the values it consumes/produces),
`execution` (the `TaskExecutor`/`TaskContext` boundary), `process` (to run
services), and `config` (the validated configuration it reads). The
`runtime -> execution`, `runtime -> process`, and `runtime -> config` edges are
the same documented infrastructure exceptions as Git (ADR 007) and Build
(ADR 008): the adapter implements a `TaskExecutor`, routes all execution through
the single process owner, and reads the already-validated configuration rather
than parsing files. `application` never depends on the runtime crate.

## Alternatives considered

1. **Run services directly from the application core.**
   Rejected: it would put OS process handling in the application, bypass the
   task engine and process manager, and duplicate behavior across TUI and MCP.

2. **A `ServiceManager` port in the application with a runtime implementation.**
   Rejected for the mutating path: every lifecycle operation is a task, exactly
   like builds and Git, so a task-parameter value (`ResolvedServiceTarget`) is
   the contract, not a trait. Only the read path needs a port, so only
   `ServiceRuntimeIndex` — a small read-only trait — was added.

3. **Put the runtime in the `process` crate.**
   Rejected: it would couple a general process owner to service/JVM/Node
   knowledge. Java and Node specifics belong in their own runtimes.

4. **A dynamic plugin framework for runtimes.**
   Rejected (`tasks/010.md` §6): no discovery, lifecycle, or dynamic loading is
   needed. A `BTreeMap<RuntimeType, Arc<dyn ServiceRuntime>>` is enough.

5. **A capability system for runtimes.**
   Rejected for now (`tasks/010.md` §7): both current runtimes support the same
   lifecycle, so capabilities would be unused surface. Add them when a runtime
   genuinely lacks an operation.

6. **Keep the process alive by keeping the startup task running.**
   Rejected (`tasks/010.md` §19): a multi-hour task would exhaust the engine's
   concurrency and confuse task status. Ownership lives in the manager; the task
   completes.

7. **Tie the managed process to the startup task's cancellation token.**
   Rejected (`tasks/010.md` §26): cancelling a completed startup task must never
   stop the running service. The executor deliberately does not use
   `spawn_with_cancellation`.

8. **Probe the toolchain version inside the runtime's `prepare_start`.**
   Rejected: `prepare_start` would then be async and would spawn a process,
   breaking the "runtimes produce data" rule and the process-owner rule. Version
   probing is a separate `RuntimeInspector` seam, defaulting to a no-op.

9. **Kill or reclaim a process found on a configured port.**
   Rejected (`tasks/010.md` §21, §33): a process on a port is not assumed to
   belong to Service Orchestrator. The manager reports the conflict and stops.

10. **Extend `RuntimeStateStore` to carry rich managed-process detail.**
    Rejected: the store is a simple observed-state cache; coupling it to
    pids/handles would blur the boundary. Rich detail flows through
    `ServiceRuntimeIndex` instead.

## Consequences

### Positive

- One orchestration path serves the TUI and MCP: lifecycle behavior lives in the
  task executors, manager, and runtimes, and reaches adapters through the task
  engine.
- The process manager stays the single process owner; the runtime adapter
  contains no OS process plumbing.
- Start/stop idempotency, coordinated concurrent starts, port-conflict
  reporting, version checks, and safe cancellation are enforced once, uniformly.
- Startup tasks are short and managed processes are owned, so long-running
  services fit the engine's concurrency model.
- Secrets never enter task parameters, results, events, or errors.
- Adding a runtime is localized: implement `ServiceRuntime`, extend
  `RuntimeType`, register it, add tests.

### Negative / trade-offs

- A new infrastructure crate and three new documented dependency edges
  (`runtime -> execution`, `runtime -> process`, `runtime -> config`).
- The default `NullRuntimeInspector` means a pinned version is only checked when
  a real inspector is wired; production wiring must install
  `ProcessRuntimeInspector`.
- Port detection is conservative and best-effort (a localhost TCP connect); a
  filtered port reads as free. This favors not blocking legitimate starts.
- `RuntimeType` is closed, so a service whose runtime is not Java/Node fails with
  `UnsupportedRuntime` until its runtime is implemented.
- Startup readiness is process-started, not service-ready or healthy; full health
  checks are a later task (`tasks/010.md` §18).

## Follow-up

- Wire `ProcessRuntimeInspector` in the composition root when the application
  binary wires the runtime adapter.
- Add Docker/Kubernetes runtimes when their process targets exist, reusing the
  `StartPlan`/registry seams.
- Add per-runtime capabilities if a future runtime cannot support an operation.
- Add automatic restart policies (explicitly out of scope here) as an opt-in
  manager policy, never implicit.
- Surface runtime status and lifecycle events through the TUI and MCP strictly
  via the application use cases and `ServiceRuntimeIndex`.

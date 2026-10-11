# ADR 006: The process manager

- **Status:** Accepted
- **Date:** 2026-10-07
- **Deciders:** Service Orchestrator maintainers

## Context

Every capability in Service Orchestrator that actually touches the machine ends
at an operating-system process: a Git command, a Maven or npm build, a
Liquibase migration, a long-running application, and a health probe. ADR 001 and
ADR 004 placed a `process` adapter in the infrastructure layer and fixed the
inward dependency rule; the `AGENTS.md` engineering guidance goes further and
requires that the process manager be the *only* component allowed to create,
monitor, and terminate OS processes (`AGENTS.md` §15, §32).

This ADR records how that component is built: what it owns, its public API and
lifecycle, how it captures output and signals, and the concrete choices for
cancellation, shutdown, and the registry.

The requirements that shaped the decision:

- only the process manager may spawn processes;
- it must support spawn, stop, kill, and restart, with pid tracking;
- `stdout` and `stderr` must be captured independently and never merged;
- exit detection, status reporting, cancellation, graceful shutdown, and
  cleanup of child processes must all be supported;
- it must expose a `ProcessHandle` (`status`, `pid`, `stop`, `kill`, `wait`,
  `subscribe_logs`) and a structured `ProcessSpec` (arguments, working
  directory, environment, timeout, restart policy);
- the lifecycle must have explicit states with documented transitions;
- a requested stop must be `SIGTERM` first, then escalate to `SIGKILL` after a
  timeout;
- a restart must preserve the specification and produce a new execution
  instance;
- environment secrets must never leak;
- there must be a registry addressable by process and by service, listing
  running and all processes;
- log streaming must be usable by a future TUI, MCP server, and logging
  subsystem; many processes must run concurrently; and the whole component must
  be thread-safe and interface-independent;
- it must be testable without Maven, Git, or any developer repository; and
- Git, Maven, npm, Liquibase, Spring Boot, React, and health checks are **not**
  implemented in this task.

## Decision

Implement the process manager as the `service-orchestrator-process` crate,
depending only on `service-orchestrator-domain` plus `tokio`, `thiserror`, and
(on Unix) `nix` for signal delivery. It exposes a `ProcessManager`, a
`ProcessHandle`, a `ProcessSpec`, a `ProcessRegistry`, and an
interface-neutral `ProcessEvent`/`LogStream` surface.

### Why a single owner of processes

If each adapter spawned its own processes, there would be no authoritative view
of what is running, no single place to enforce stop-before-kill, and no way to
prevent orphaned children. Centralizing OS process ownership:

1. gives the application and both interfaces one source of truth for running
   processes (`AGENTS.md` §15);
2. keeps shell-avoidance and argument validation in one place, so untrusted
   configuration values can never be re-interpreted by a shell (`AGENTS.md`
   §32); and
3. isolates all platform-specific process and signal handling behind one
   boundary.

### Structured specifications, not shell strings

`ProcessSpec` is a program plus a typed argument vector. It is deliberately not
a `sh -c` string, even though tests use a shell for convenience. Structured
arguments mean a value from configuration cannot inject a second command, and
they make the recorded command line auditable and redactable.

### Explicit lifecycle with one source of truth

The state machine is `Created -> Starting -> Running -> (Stopping) ->
Stopped | Exited | Failed | Killed`, with `Running -> Starting` and
`Stopping -> Starting` as the only extra edges, used solely by restart. Legality
is decided by `ProcessStatus::can_transition_to`, so both the manager and its
tests agree on the graph and an illegal transition returns a structured error
rather than corrupting state.

The manager distinguishes *why* an execution ended (`ExitReason`) rather than
collapsing every termination into one status. This lets the application decide
policy (for example, whether to restart) from structured data and lets a future
health subsystem separate "the process exited" from "the service is healthy"
(`AGENTS.md` §16).

### One supervisor task per process

`spawn` is synchronous and returns a `ProcessHandle` immediately. A single
supervisor task then owns the Tokio `Child` for its whole life: it drains
`stdout` and `stderr` on independent readers, waits for exit, performs graceful
stop and escalation, and applies the restart policy. Centralizing the child in
one task avoids sharing a raw child across tasks and guarantees the output pipes
are drained before the terminal state is published.

### Separate `stdout` and `stderr`

Merging streams loses the distinction between program output and diagnostics,
which matters for build tooling and logs. The manager keeps two independent
line readers, and each line records its originating stream.

### Process groups and stop-before-kill

On Unix each process is placed in its own process group and signals are sent to
the whole group. This prevents orphaned grandchildren: a shell that spawned
children is reaped together with them. A requested stop is always graceful
first (`SIGTERM` by default, configurable), waits a configurable grace period,
and only then escalates to `SIGKILL`. The escalation outcome is recorded
honestly: a stop that needed `SIGKILL` is `Killed`, not `Stopped`.

### Cancellation without a dependency on the task engine

The process crate does not depend on `execution` or `TaskContext`. Instead
`spawn_with_cancellation(spec, future)` accepts any future that resolves on
cancellation and performs the same graceful stop. This keeps the process
manager usable on its own while still integrating with the engine's cooperative
cancellation model, and it avoids a dependency edge that would point outward.

### Restart preserves the specification

`restart(id)` stops and awaits the old instance, then spawns a new one with a
new `ProcessId`. Automatic restarts reuse the handle and id but begin a fresh
execution instance. In both cases the specification is preserved verbatim, so a
restart never drifts from what was configured.

### In-memory registry

The registry is a `std::sync::Mutex<BTreeMap<ProcessId, Arc<ProcessEntry>>>`.
`BTreeMap` gives deterministic (id-ordered) listing, which makes tests and TUI
rendering predictable. Finished processes are retained until explicitly
forgotten, mirroring the task engine's "recent tasks" behaviour. The lock is
short-lived and never held across an `.await`; status waiters use a
`tokio::sync::watch` channel instead of polling.

### Interface-neutral events and logs

The manager publishes lifecycle events and log lines over a bounded
`tokio::sync::broadcast` channel. Events carry only identifiers, structured
exit information, and log text — never terminal rows or MCP shapes — so the
TUI, MCP server, and logging subsystem each subscribe and render independently.
A lagging subscriber gets `Lagged` and resyncs from the authoritative registry,
so a slow consumer can never block a process.

### No runtime ownership

The manager captures a Tokio `Handle` and does not create a runtime. The
composition root controls runtime creation and shutdown, exactly as the task
engine does (ADR 005), keeping shutdown ordering explicit and letting the
manager be built inside an existing runtime.

## Alternatives considered

1. **Spawn processes directly in each adapter (Git, build, Liquibase, health).**
   Rejected: no single owner, no consistent stop semantics, and a high risk of
   orphaned child processes (`AGENTS.md` §15).

2. **A shell-string command (`sh -c "..."`) as the primary specification.**
   Rejected as the default because it re-opens command injection and hides the
   real argument vector (`AGENTS.md` §32). Structured arguments are the model;
   a shell is just one program a test may run.

3. **Expose the raw Tokio `Child` from the handle.**
   Rejected: it would leak runtime types across the boundary and let callers
   bypaass the supervisor's stop/timeout/restart logic.

4. **Merging `stderr` into `stdout`.**
   Rejected: it destroys the output/diagnostic distinction that build and log
   consumers rely on.

5. **One global lock or a single supervisor task for all processes.**
   Rejected: it serializes independent processes and makes a stalled process
   block unrelated ones. Per-process supervisors plus short-lived locks scale to
   the 40–100+ process target (`AGENTS.md` §40).

6. **Signalling only the child pid rather than the process group.**
   Rejected: a shell that spawned children would leave them running, producing
   orphans. Group signalling is what enforces cleanup.

7. **Force-killing immediately, or dropping the future to cancel.**
   Rejected: dropping skips cleanup and immediate `SIGKILL` gives services no
   chance to flush. Graceful-then-escalate was chosen instead.

8. **Cancellation by depending on the `execution` crate's `TaskContext`.**
   Rejected: it would point a dependency outward and couple the process manager
   to the engine. A caller-supplied future keeps the edge in the caller.

9. **A cancelling restart that reuses the same handle for manual restarts.**
   Rejected: `restart` produces a fresh execution instance with a new id, which
   keeps "which run is this?" unambiguous for logs and queries; only policy-
   driven automatic restart reuses the handle.

10. **A persistent process store / database.**
    Rejected: process state is inherently transient and tied to a running
    machine; an in-memory registry is sufficient for the MVP.

11. **Integrating health checks into the manager.**
    Rejected: a live pid does not imply a healthy service (`AGENTS.md` §16).
    Health is a separate, configurable concern.

## Consequences

### Positive

- Exactly one owner of OS processes, so running state is authoritative and
  children are never orphaned.
- Structured specifications keep untrusted configuration out of a shell.
- One documented lifecycle, one stop protocol (graceful then escalate), and one
  restart model are defined once and reused by every caller.
- The manager is fully testable with ordinary short-lived processes and bounded
  waits; no Maven, Git, or developer repository is required.
- Interface-neutral events and logs let the TUI, MCP, and logging subsystem
  subscribe without the manager knowing any of them.
- The dependency direction stays inward: `process -> domain` only.

### Negative / trade-offs

- Log delivery is in-process and lossy under backpressure; durable, bounded log
  retrieval is deferred to the logging subsystem (`AGENTS.md` §35).
- Completed processes are retained until explicitly forgotten; a retention
  policy is future work.
- Graceful stop depends on the process honouring the signal; a process that
  ignores it is force-killed after the grace period, and the outcome is recorded
  as `Killed`.
- Graceful cancellation requires caller cooperation: an executor must pass a
  cancellation future to `spawn_with_cancellation` for task cancellation to stop
  the process.
- Automatic restart reuses a handle and id, so consumers must read the
  `Restarting` event / `started_at` rather than assume an id means one run.

## Follow-up

- Implement the Git, build (`build`), Liquibase, and health adapters on top of
  `ProcessManager::spawn`, and route them through the task engine's executors.
- Connect task cancellation to `spawn_with_cancellation` in the executors.
- Add a completed-process retention/eviction policy.
- Add bounded historical log retrieval in the logging subsystem.
- Surface process state and log streams in the TUI and MCP adapters strictly
  through the manager's snapshot, event, and log APIs.

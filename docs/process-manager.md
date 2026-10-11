# The Process Manager

The process manager is the **only** component in Service Orchestrator that
creates, monitors, and terminates operating-system processes. It owns process
pids, exit status, child processes, lifecycle state, and output capture. Every
other subsystem — Git, build tooling (Maven/npm), Liquibase, health checks, and
the service executors — must ask the process manager to run a process rather
than constructing a `std::process::Command` itself (`AGENTS.md` §15, §32). It
lives in the `service-orchestrator-process` crate.

The manager is deliberately independent of:

- the TUI (`ratatui`) and the MCP server (`rmcp`),
- Git, Maven, npm, Liquibase, health checks, and Spring Boot,
- the application layer and the other infrastructure adapters.

It depends only on `service-orchestrator-domain` (plus `tokio`, `thiserror`,
and, on Unix, `nix` for signal delivery). It emits interface-neutral events and
log lines, so the eventual TUI, MCP server, and logging subsystem can each
observe the same processes without the manager knowing about any of them.

```text
Git / Build / Liquibase / Health / ServiceExecutor
                        │
                        ▼
                 ProcessManager ───> ProcessRegistry (in-memory)
                        │                    ▲
                        ▼                    │
                  supervisor task ───────────┘
                        │
           ┌────────────┼────────────┐
           ▼            ▼            ▼
         stdout       stderr       wait/exit
                        │
                        ▼
             ProcessEvent / LogStream  (interface-neutral)
```

The single most important rule is **ownership**: the manager is the sole owner
of the Tokio `Child`, and the rest of the system interacts with a process only
through a [`ProcessHandle`](#processhandle).

> This task implements the generic manager only. Git, Maven, npm, Liquibase,
> Spring Boot, React, and health checks are intentionally **not** implemented
> here and are out of scope.

---

## 1. Specification

A process is described by a `ProcessSpec`: *what to run* before anything is
spawned. It is a structured command (a program plus a typed argument vector),
never an opaque shell string, so values from configuration cannot be
re-interpreted by a shell (`AGENTS.md` §32).

| Field | Meaning |
| --- | --- |
| `program` | The executable. Must be non-empty and contain no whitespace. |
| `args` | The structured argument vector, passed in order. |
| `working_dir` | Optional working directory. |
| `environment` | Additional variables, carried in the domain `Environment` type, which redacts secret values on display and serialization. |
| `timeout` | Optional maximum runtime; when exceeded the process is stopped. |
| `restart` | The `RestartPolicy` (default `Never`). |
| `shutdown` | The `ShutdownStrategy` (default graceful `SIGTERM`, then `SIGKILL` after 5s). |
| `service` | The owning `ServiceId`, when the process belongs to a service. |

```rust
let spec = ProcessSpec::new("mvn")?
    .args(["clean", "install"])
    .working_dir("/work/auth-service")
    .with_timeout(Duration::from_secs(600))
    .for_service(ServiceId::new("auth-service")?);
```

`ProcessSpec::description()` returns the redacted command line — program plus
arguments only, never environment values.

### Restart policies

Restart is opt-in:

- `Never` (default) — a process that ends stays ended.
- `OnFailure { max_restarts, backoff }` — restart after a non-zero exit or a
  signal, up to a limit.
- `Always { max_restarts, backoff }` — restart after any non-requested end.

A requested stop (`Terminated`, `Cancelled`, `Killed`) never triggers an
automatic restart, even under `Always`: an operator's intent wins.

### Shutdown strategy

`ShutdownStrategy` selects the graceful signal (`Term`, `Int`, or `Kill`) and
the grace period before escalation. The default is `SIGTERM` followed by a
five-second wait and then `SIGKILL`.

---

## 2. Lifecycle

Every managed process moves through the `ProcessStatus` state machine, whose
single source of truth is `ProcessStatus::can_transition_to`:

```text
Created ──> Starting ──> Running ──┬──> Stopping ──┬──> Stopped
                         ▲         │               ├──> Exited
                         │         │               ├──> Failed
                         │         │               └──> Killed
                         │         ├───────────────> Stopped
                         │         ├───────────────> Exited
                         │         ├───────────────> Failed
                         │         └───────────────> Killed
                         │
                         └────────────────────────────── Starting
                            (a restart begins a new execution)
```

- `Created` — the specification was accepted; the id is published.
- `Starting` — a spawn is in progress.
- `Running` — the process is alive and monitored.
- `Stopping` — a graceful stop has been requested.
- `Stopped` / `Exited` / `Failed` / `Killed` — terminal states with no outgoing
  edges.

`Running -> Starting` and `Stopping -> Starting` are the only extra edges, used
exclusively when a restart policy begins a fresh execution instance on the same
handle. `Starting -> Failed` records a process that could not be spawned.

`ProcessStatus::domain_state()` maps the rich status into the interface-neutral
domain `ProcessState` (`Stopped`, `Exited`, and `Killed` all map to
`ProcessState::Stopped`), so the application and TUI keep the vocabulary they
already understand.

### Exit reasons

The manager distinguishes *why* an execution ended via `ExitReason`:

| Reason | Terminal status | Cause |
| --- | --- | --- |
| `Exited` | `Exited` | clean exit, code 0 |
| `Failed` | `Failed` | non-zero exit |
| `Signalled` | `Failed` | killed by a signal the manager did not send |
| `Terminated` | `Stopped` | manager-requested graceful stop |
| `Cancelled` | `Stopped` | the owning task was cancelled |
| `Killed` | `Killed` | force-killed after a graceful stop failed |
| `TimedOut` | `Killed` | exceeded the configured timeout |
| `SpawnFailed` | `Failed` | the process could not be started |

`ExitInfo` carries the exit code, the terminating signal number, the reason,
and start/finish timestamps plus a computed duration.

---

## 3. Supervision

Each spawned process is driven by exactly one supervisor task on the runtime
the manager captured. The supervisor:

1. captures `stdout` and `stderr` line by line, independently and never merged;
2. waits for the process to exit (or for a stop request or timeout);
3. honours a graceful stop: sends the configured signal, waits the grace
   period, and escalates to `SIGKILL` if the process is still alive;
4. applies the restart policy, spawning a fresh execution instance when
   required; and
5. records the terminal `ExitInfo` and publishes the matching event.

The supervisor is the only owner of the Tokio `Child`. It waits for the output
readers after the process exits, so a trailing line is never lost.

### Process groups

On Unix each process is spawned into its own process group
(`Command::process_group(0)`), and signals are sent to the whole group via the
negative pid. This is what prevents a shell that spawned children from leaving
those children behind: terminating the group reaps the descendants too, so the
manager never orphans a child process (`AGENTS.md` §15).

Signal delivery is best-effort. A process that already exited, or whose pid was
recycled in the tiny window between observation and delivery, may cause the
signal call to fail; the supervisor treats a failed signal as non-fatal and
still waits for the child to be reaped.

---

## 4. `ProcessHandle`

`ProcessHandle` is the cloneable, strongly typed handle returned by `spawn`. It
exposes:

| Method | Description |
| --- | --- |
| `id()` | The stable `ProcessId`. |
| `status()` | The current `ProcessStatus`. |
| `pid()` | The OS pid while alive. |
| `spec()` | The originating `ProcessSpec`. |
| `is_running()` / `is_terminal()` | Lifecycle predicates. |
| `snapshot()` | A point-in-time `ProcessSnapshot`. |
| `stop()` | Graceful stop; recorded as `Terminated`. |
| `cancel()` | Graceful stop; recorded as `Cancelled`. |
| `kill()` | Immediate `SIGKILL`; recorded as `Killed`. |
| `wait()` | Awaits a terminal state and returns a snapshot. |
| `subscribe_events()` / `subscribe_logs()` | Streams scoped to this process. |

Stopping or killing an already-terminal process is a no-op that returns `Ok`.

A `ProcessSnapshot` is a `serde`-serializable, immutable view of a process's
identity, specification, observed state, and last exit — never a raw handle,
and never a secret environment value.

---

## 5. The manager

`ProcessManager` is cheaply cloneable so the TUI, the MCP server, and the
service executors can each hold one observing the same registry and event
stream.

| Method | Description |
| --- | --- |
| `new()` / `with_config(config)` | Build inside the current Tokio runtime. |
| `spawn(spec)` | Start a process; returns a `ProcessHandle`. Synchronous. |
| `spawn_with_cancellation(spec, future)` | Spawn and stop when `future` resolves. |
| `restart(id)` | Stop and await the old instance, then spawn a new id. |
| `stop(id)` / `cancel(id)` / `kill(id)` | Request termination. |
| `handle(id)` / `snapshot(id)` | Look up one process. |
| `list()` / `list_active()` | List all / active processes. |
| `find_by_service(service)` | Handles for every process of a service. |
| `forget(id)` | Drop a finished process from the registry. |
| `subscribe_events()` / `subscribe_logs()` | Manager-wide streams. |
| `shutdown()` | Stop every active process and refuse new spawns. |

`ProcessManagerConfig` controls the event channel capacity, an optional default
timeout, and the manager-level shutdown grace.

### Runtime ownership

The manager does **not** own a Tokio runtime. It is built inside a runtime
context and captures a `Handle`, so the composition root controls runtime
creation and shutdown. `spawn` is synchronous (Tokio's spawn is non-blocking);
supervision continues on the captured runtime.

### Restart semantics

`restart(id)` is asynchronous: it stops the existing process gracefully, awaits
its terminal state, and then spawns a **fresh execution instance** with a new
`ProcessId`, preserving the original specification. Automatic restarts, by
contrast, reuse the same handle and id, moving it back to `Starting` and
publishing a `Restarting` event with a one-based attempt number.

### Shutdown

`shutdown()` marks the manager as shutting down (new spawns are refused with
`ProcessError::ShuttingDown`), asks every active process to stop gracefully,
waits up to the configured grace period, and force-kills anything that remains.
Dropping the manager does not abort the supervisor tasks; the composition root
shuts the runtime down after `shutdown()`.

---

## 6. The registry

`ProcessRegistry` is the manager's authoritative in-memory index of every
process it has spawned, keyed by `ProcessId`. It supports:

- lookup by process id (`get`, `snapshot`, `contains`);
- lookup by service id (`find_by_service`);
- listing all processes, only the active ones, or handles for either; and
- explicit removal via `remove`/`forget`.

Finished processes remain queryable until they are explicitly removed, so
recent exit information stays available for inspection — mirroring the task
engine's "recent tasks" behaviour. Removing an active process is refused; stop
or kill it first.

A single `std::sync::Mutex` guards the map. Critical sections are short and the
lock is never held across an `.await`. Status waiters block on a
`tokio::sync::watch` channel rather than polling, and the status read plus the
subscription happen under one lock to avoid a lost-notification race.

---

## 7. Cancellation and stop

A stop request travels through a per-process `tokio::sync::watch` channel so
the supervisor wakes immediately. Three requests exist:

- **Graceful (`Terminated`)** — from `stop()`. Sends the configured signal,
  waits the grace period, then escalates to `SIGKILL`.
- **Graceful (`Cancelled`)** — from `cancel()`. Identical behaviour, but the
  exit reason records that a task was cancelled.
- **Force (`Killed`)** — from `kill()`. Sends `SIGKILL` immediately.

If the process exits on its own while a graceful stop is in flight, the
natural exit is recorded instead. If a force-kill arrives during a graceful
stop, the supervisor escalates at once.

### Integration with the task engine

The process crate does **not** depend on the `execution` crate or its
`TaskContext`. Instead, `spawn_with_cancellation(spec, future)` takes any future
that resolves when the owning task is cancelled:

```rust
let handle = manager.spawn_with_cancellation(
    spec,
    async move { context.cancellation().clone().cancelled().await },
)?;
```

This keeps the process manager usable without the task engine while still
participating in its cooperative cancellation model. Cancellation is therefore
graceful: the process receives its signal, is given the grace period to clean
up, and is force-killed only if it does not comply.

---

## 8. Logs and events

`stdout` and `stderr` are captured independently and **never merged**. Each
line becomes a `LogLine` carrying the process id, the originating stream
(`Stdout` or `Stderr`), the text without the trailing newline, and a timestamp.

Lines are delivered two ways from the same bounded broadcast channel:

- through a `LogStream` (`recv()` returns `LogLine`), and
- as `ProcessEvent::StdoutLine` / `ProcessEvent::StderrLine`.

`ProcessEvent` also reports lifecycle changes: `Created`, `Starting`, `Started`,
`Restarting`, `Stopping`, `Stopped`, `Exited`, `Failed`, and `Killed`. The model
carries identifiers, structured exit information, and log lines only — never
terminal rows or MCP response shapes — so each consumer renders independently.
A manager stream observes every process; a handle stream is filtered to one.

Delivery is in-process and lossy under backpressure: a subscriber that falls
behind observes a `Lagged` error and resyncs from the authoritative registry.
Persistent, bounded log retrieval is a future concern owned by the logging
subsystem (`AGENTS.md` §35).

---

## 9. Secrets

Environment variables are carried in the domain `Environment` type, which
redacts secret values when displayed or serialized. Secrets are applied to the
child process but never appear in a `ProcessSnapshot`, a `ProcessSpec`
description, an event, a task result, or an error (`AGENTS.md` §31).

---

## 10. Concurrency and thread safety

- The manager is `Clone` and `Send + Sync`; all shared state is behind `Arc`.
- Many processes run concurrently, each with its own supervisor and readers.
- Shared mutable state is guarded by a short-lived `std::sync::Mutex` that is
  never held across an `.await`.
- The registry, the status channel, and the shutdown channel are the only
  inter-task mechanisms; there is no global lock around the whole manager.
- No process is cancelled by dropping a future; termination is always explicit
  through the supervisor.

---

## 11. Testing

The manager is tested end to end without Maven, Git, or any developer
repository, using small deterministic helpers (`/bin/sh`, `printf`, `sleep`) and
bounding every wait with a timeout.

- **Unit tests** cover the pure pieces: id validation, the `ProcessStatus`
  state machine and its legal/illegal transitions, `ExitReason` mapping,
  `ExitInfo` duration, specification validation, and restart-policy decisions.
- **Integration tests** (`crates/process/tests/process_manager.rs`) drive the
  public API against real short-lived processes: clean exit and exit-code
  classification, independent stdout/stderr capture, graceful stop, `SIGKILL`
  escalation when `SIGTERM` is ignored, force-kill, timeout, restart into a new
  instance, automatic restart after failure, cooperative cancellation, registry
  lookup and listing, many concurrent processes, spawn failure, and manager
  shutdown.

Tests subscribe to the event stream *before* spawning, so a process that exits
immediately still has its lines buffered for a subscriber that already exists.

---

## 12. Non-goals

This component does **not** implement Git, Maven, npm, Liquibase, Spring Boot,
React, or health checks. Those are separate infrastructure adapters that will
call `ProcessManager::spawn` (directly or through a `TaskExecutor`) and
interpret the resulting exit information. The health of a running process is
also not decided here: a live pid does not by itself mean a healthy service
(`AGENTS.md` §15, §16).

See `docs/adr/006-process-manager.md` for the reasoning behind these decisions.

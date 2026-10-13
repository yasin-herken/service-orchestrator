# The Shared Local Runtime (Daemon)

Service Orchestrator runs one authoritative local runtime — the **daemon** — and
every interface reaches it as a client over a Unix domain socket. The Ratatui TUI
and the MCP server no longer each build their own application core, task engine,
process registry, and log store; they connect to the one daemon that owns them.
This document describes that runtime: its lifecycle, the IPC protocol, how
clients connect and reconnect, how state is shared, how it is secured, and what
its known limitations are.

The decision record is `docs/adr/015-shared-local-runtime.md`. The single-binary
model is preserved: `daemon`, `tui`, and `mcp` are subcommands of the same
`service-orchestrator` executable.

---

## 1. The problem it solves

Before the daemon, independent processes each initialized an independent
application core:

```text
TUI process                 MCP process
├── Task Engine A           ├── Task Engine B
├── Process Registry A      ├── Process Registry B
└── Log store A             └── Log store B
```

That produced exactly the failure modes a developer environment cannot tolerate:

- inconsistent service status between interfaces;
- duplicate process startup (both interfaces starting the same service);
- inconsistent task history (a task submitted by MCP was invisible in the TUI);
- the inability to stop a process another interface started;
- conflicting Git, build, and migration operations;
- inconsistent logs and health.

---

## 2. Architecture

```text
                   Human developer            AI agent
                          │                      │
                          ▼                      ▼
                     Ratatui TUI            MCP stdio
                          │                      │
                          └──────────┬───────────┘
                                     │  OrchestratorClient
                                     ▼
                            Local IPC (Unix socket)
                                     │
                                     ▼
                        Service Orchestrator daemon
                                     │
                             one Application core
                                     │
                  ┌──────────────────┼──────────────────┐
                  ▼                  ▼                  ▼
             Task Engine        Runtime manager     Policy engine
                  │                  │
                  └────────┬─────────┘
                           ▼
                     Infrastructure
                (Git, build, processes, …)
```

The daemon owns the mutable runtime state. Clients hold only refreshable caches.

- **Daemon** (`crates/daemon`): binds the socket, validates protocol versions,
  routes each request to the shared application core, and fans task and log
  events out to connected clients. It contains **no business logic**.
- **Client** (`crates/client`): the shared `OrchestratorClient` boundary and its
  daemon-backed implementation, `RemoteClient`. The TUI, MCP, and desktop GUI all
  call this boundary and never learn that the daemon exists.

The application core must not depend on the TUI, MCP, the GUI, or the daemon; the
daemon depends on the core, and the interfaces depend only on the client
abstraction.

---

## 3. Daemon lifecycle

`service-orchestrator daemon` runs the daemon **in the foreground** until it is
asked to stop. Startup proceeds in order:

1. Load and validate configuration (`--config`, `SERVICE_ORCHESTRATOR_CONFIG`,
   or the default location).
2. Construct the infrastructure adapters (Git, build, processes, Liquibase,
   health, logging).
3. Register their task executors and build the **one** task engine.
4. Wire the application core over those adapters with standard authorization.
5. Bind the IPC endpoint and restrict it to the owner.
6. Accept client connections and serve requests and events.
7. On shutdown, stop accepting, notify clients, drain, reap managed processes,
   remove the socket and PID file, and exit cleanly.

The daemon does not initialize Ratatui or the MCP transport.

### Single-instance coordination

Only one daemon may own a runtime directory. The guarantee is an **advisory lock**
on `<socket>.lock` acquired with `flock(LOCK_EX | LOCK_NB)` before the socket is
touched:

- the kernel releases the lock automatically when the process dies, so a crash
  never leaves the directory permanently locked;
- the socket *file* is not used as the lock, because a stale socket file outlives
  the process that created it;
- once the lock is held, a leftover socket file is removed and rebound safely.

A second daemon started against the same socket fails with
`DaemonError::AlreadyRunning`, so two clients racing to auto-start cannot produce
two authoritative engines.

### Runtime files

| File | Purpose |
| --- | --- |
| `<config_dir>/run/daemon.sock` | The IPC endpoint (owner-only, `0600`). |
| `<config_dir>/run/daemon.pid` | The daemon's process id, while running. |
| `<config_dir>/run/daemon.lock` | The advisory single-instance lock. |
| `<config_dir>/run/daemon.log` | Standard output/error of an auto-started daemon. |

The `run/` directory is created with `0700` permissions.

---

## 4. The IPC endpoint

The endpoint is a **Unix domain socket**. Its path is:

1. `$SERVICE_ORCHESTRATOR_SOCKET` when set, otherwise
2. `<config_dir>/run/daemon.sock`, where `<config_dir>` is the directory of the
   active configuration file.

Deriving the path from the configuration keeps two configurations on one machine
from sharing a daemon. The socket is local only; there is no public TCP port and
no remote access.

---

## 5. The IPC protocol

The transport is **length-prefixed JSON**: a big-endian `u32` byte length followed
by that many bytes of UTF-8 JSON. A frame is capped at 8 MiB, so a malformed or
hostile client cannot make the daemon allocate unbounded memory. There is no
shell string and no arbitrary command execution anywhere in the protocol.

### Client → server

```json
{
  "protocol": 1,
  "request_id": "req-7",
  "client": "tui",
  "allow_destructive": false,
  "method": "start_service",
  "params": ["orders-api"]
}
```

- `protocol` — the protocol version the client speaks (`PROTOCOL_VERSION`).
- `request_id` — a unique, client-generated identifier used to correlate the
  response (and to avoid duplicate execution).
- `client` — `tui`, `mcp`, or `cli`.
- `allow_destructive` — honest intent to permit a destructive operation; honored
  only for an interactive peer and always re-checked by the policy engine (§10).
- `method` — a stable operation name (the same vocabulary as the application use
  cases and the MCP tools).
- `params` — the method's positional parameters as a JSON array.

### Server → client

A response:

```json
{ "kind": "response",
  "request_id": "req-7",
  "result": { "outcome": "task_scheduled", "task_id": "task-3", "kind": "start_service" } }
```

…or a structured application error:

```json
{ "kind": "response",
  "request_id": "req-7",
  "result": { "error": "permission_denied", "permission": "destructive", "message": "…" } }
```

An asynchronous event:

```json
{ "kind": "event", "event_type": "task", "event": { "task_created": { "task": { … } } } }
```

Event kinds are `task` (a task-engine state change), `log` (a realtime log entry
for a subscribed service), `configuration` (the active configuration changed),
and `shutdown` (the daemon is stopping).

Responses and events are written by a single per-connection writer task, so a
pushed event can never interleave in the middle of a response frame.

### Versioning

`PROTOCOL_VERSION` versions the wire shape. A request whose `protocol` differs is
refused **before** it is interpreted, with
`ApplicationError::RuntimeUnavailable { code: "PROTOCOL_INCOMPATIBLE" }` and a
message naming both versions. An incompatible session is never silently
misinterpreted.

### Error codes

| Code | Meaning |
| --- | --- |
| `DAEMON_UNAVAILABLE` | The daemon could not be reached, or the connection dropped. |
| `PROTOCOL_INCOMPATIBLE` | The client and daemon speak different protocol versions. |
| `INVALID_REQUEST` | The request was malformed, mistyped, or exceeded the transport's limits. |
| `UNKNOWN_OPERATION` | The method is not recognized by the daemon. |

Application errors (`SERVICE_NOT_FOUND`, `PERMISSION_DENIED`,
`CONFIRMATION_REQUIRED`, build/Git/migration/health errors, …) cross the wire as
their existing structured variants, so a client sees exactly the same error it
would see in-process.

---

## 6. The client abstraction

`service-orchestrator-client` defines `OrchestratorClient`, a one-to-one mirror
of the application's use cases. Two implementations exist:

- `Application` itself, so interface tests run on the real core with no daemon;
- `RemoteClient`, which proxies each call over the socket.

The TUI, MCP, and desktop GUI hold `Arc<dyn OrchestratorClient>` and never know
which implementation they have. `RemoteClient` exposes the same method set as the
application; a new use case is added in one place and is immediately available to
every interface and to the daemon.

A dedicated reader thread owns the read half of the socket and demultiplexes
server messages: responses are matched to the waiting caller by `request_id`, task
events are republished on a bounded local broadcast channel, and log events are
routed to the subscription that asked for them. Calls are blocking (the interface
state machine is synchronous); a local round trip is sub-millisecond, with a
60-second upper bound so a hung daemon cannot block an interface forever.

`RemoteClient` also answers `daemon_status` (the daemon's diagnostic snapshot) and
`shutdown_daemon` (a graceful shutdown request).

---

## 7. Automatic startup

A developer never has to start the daemon by hand:

- `service-orchestrator tui` and `service-orchestrator mcp` connect to a running
  daemon, or start one, wait for it to become reachable, and connect.
- `service-orchestrator daemon status` and `daemon stop` **never** start a
  daemon; they connect only to an existing one (`connect_existing`).

An auto-started daemon is launched as a child in its own process group with its
standard streams redirected to `daemon.log`, so it survives its launching terminal
and cannot be killed by a terminal hang-up.

Set `SERVICE_ORCHESTRATOR_NO_AUTOSTART=1` to make a missing daemon a hard error
instead of an auto-start.

### Startup races

If the TUI and MCP start at the same moment, both attempt to spawn a daemon. The
advisory lock ensures only one becomes authoritative; the loser exits and the
client connects to the winner. The wait for readiness is bounded (15 seconds);
if the spawned child exits during startup, the client reports the failure instead
of hanging.

---

## 8. Process ownership

The daemon owns every managed service process. A service started from one
interface is managed by the daemon, so it is visible and controllable from every
other interface:

```text
TUI  ── start auth-service ──▶ daemon ──▶ auth-service RUNNING
MCP  ── get auth-service ─────▶ daemon ──▶ RUNNING
```

Duplicate startup is prevented because both interfaces see the same runtime state
store: starting an already-running service returns `AlreadyInDesiredState` rather
than spawning a second process.

### After a daemon restart

The runtime-state store is **in memory**. A daemon that was killed does not adopt
the processes it previously managed; it starts with no managed services, reports
zero, and **never** signals a process by PID or port, so an unrelated process that
reused a PID is never harmed. Managed processes that survive a crash therefore
become unmanaged — see §12.

---

## 9. Shared task state

All tasks are submitted to the **one** task engine the daemon owns. A task
submitted through any client is visible to all clients:

```text
MCP ── build auth-service ──▶ daemon ──▶ Task #123
TUI ── tasks ────────────────▶ daemon ──▶ Task #123 RUNNING
```

Clients never create an independent registry. Task listing
(`list_tasks`/`list_running_tasks`/`list_recent_tasks`) and task events are served
entirely by the daemon.

The same is true of **workflows**. A prepared environment is one
`WorkflowDefinition` submitted to the daemon's engine, so `get_workflow_status`
and `cancel_workflow` observe the identical authoritative state from any client:

```text
TUI ── prepare-env (profile) ──▶ daemon ──▶ Workflow #7
MCP ── get_workflow_status #7 ─▶ daemon ──▶ Workflow #7 RUNNING
```

The plan returned by `plan_prepare_environment` is computed by the daemon too, so
the preview an interface shows is the daemon's own authoritative plan, not a
locally re-derived one (`docs/environment-workflows.md`).

The same is true of **configuration**. The daemon owns the live configuration
(one `ConfigManager`) and is its only writer. Every client reads a snapshot from
it, previews and commits a change through it, and observes the identical result:

```text
TUI ── commit (service create) ──▶ daemon ──▶ config.toml
MCP ── get_configuration ───────▶ daemon ──▶ revision + entities
```

`validate` reads and validates the file without changing what the daemon serves,
so a client-side "check" can never mutate shared state. See
[`docs/configuration.md`](configuration.md) §9 for the management layer.

---

## 10. Event streaming

The daemon pushes realtime updates to every connected client over bounded
channels:

- **task events** — every engine state change (`TaskCreated`, `TaskStarted`,
  progress, success, failure, cancellation, …);
- **log events** — realtime log entries for services a client explicitly
  subscribed to;
- **configuration events** (`ConfigurationChanged { revision, description }`) —
  pushed after a commit or a reload, so every client converges on the same
  configuration;
- **shutdown** — the daemon is stopping.

Subscriptions are bounded, and each connection has its own outbound queue, so a
slow client can never block the daemon or another client. A subscriber that falls
behind observes a `Lagged` error and resynchronises from the shared state.

### Reconnection

Clients handle a dropped daemon connection:

- `RemoteClient` transparently reconnects (and re-establishes log subscriptions)
  if the connection is already gone when a request is about to be written, then
  retries **once**;
- a request that was **written** but produced no response is never retried,
  because the mutation may have been applied — the caller receives
  `DAEMON_UNAVAILABLE` instead. This is the idempotency rule;
- an interface that uses `connect_existing` (the diagnostics) never starts a
  daemon, so a stopped daemon is reported as stopped.

---

## 11. Security and authorization

- **Local only.** The endpoint is a Unix domain socket in a per-user `0700`
  directory, with the socket itself at `0600`. There is no public network port.
- **Bounded input.** Requests are size-capped and rejected if malformed or
  mistyped.
- **No shell.** Every operation maps to a typed application use case; a client
  cannot send an arbitrary command.
- **Secrets are never serialized.** Configuration-derived secrets stay redacted
  on every path, including across IPC.
- **The policy engine always runs.** A request carries the caller's
  `allow_destructive` intent and its `ClientKind`. The daemon honors the intent
  **only** for an interactive (`tui`) peer, and then still runs the shared policy
  engine. A non-interactive client (MCP, CLI) can never self-confirm a
  destructive operation, and neither can a client bypass the check by setting a
  boolean. This is the same rule the TUI and MCP already followed, now enforced
  at the single daemon boundary.

---

## 12. Shutdown and client exit

Daemon shutdown and client exit are different operations.

- **Client exit** (closing the TUI window, ending an MCP session) never stops the
  daemon. The daemon keeps managing running services.
- **Explicit shutdown** is requested with `service-orchestrator daemon stop`, or
  the `daemon_shutdown` operation. The daemon stops accepting new requests,
  notifies connected clients, stops accepting new tasks, applies the existing
  managed-process shutdown policy, releases the socket and PID file, and exits
  cleanly. The composition root then drains the task engine and reaps managed
  processes.

### Diagnostics

```bash
service-orchestrator daemon status
```

prints whether the daemon is running and, when it is, its PID, endpoint, protocol
version, connected-client count, active-task count, managed-service count, and
start time. A stopped daemon is reported as stopped (not an error). No sensitive
information is shown.

---

## 13. Known limitations

- **Managed-process recovery is not implemented.** Runtime state is in memory, so
  after a crash or restart the daemon adopts nothing and reports zero managed
  services. Surviving child processes become unmanaged; they are never adopted or
  signalled by PID/port. Recovering them safely (validating identity, not just a
  reused PID) is future work.
- **One daemon per configuration.** The endpoint is derived from the
  configuration directory, so two different configurations run two daemons. There
  is no cross-daemon coordination.
- **Local, single-user.** There is no authentication, remote access, or
  multi-user model; the security boundary is the owner-only socket.
- **Event delivery is best-effort.** A subscriber that falls behind drops events
  and must resynchronise; events are not persisted.
- **Configuration is read-only to clients.** The daemon owns the validated
  configuration snapshot; clients can read a summary but never mutate
  configuration files while the daemon runs. Configuration editing is a later
  task.
- **No OS-level supervision.** The auto-started daemon is a detached child, not a
  `launchd` service, so it does not restart after logout or reboot. On macOS,
  `launchd` integration would be a separate, opt-in feature.

---

## 14. Related documents

- `docs/adr/015-shared-local-runtime.md` — the decision record.
- `docs/architecture.md` — the layered architecture and dependency rules.
- `docs/tui.md` — the TUI adapter over the shared runtime.
- `docs/mcp.md` — the MCP adapter over the shared runtime.
- `docs/environment-workflows.md` — the shared environment workflow both
  interfaces observe through the daemon.
- `docs/application-core.md` — the use cases every client reaches.
- `docs/task-engine.md` — the single task engine the daemon owns.
- `docs/process-manager.md` — the single process owner.

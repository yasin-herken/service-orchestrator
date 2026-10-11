# ADR 015: Shared Local Runtime & Local Daemon

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

ADR 001 established the core-first architecture: one application core and two
interface adapters, the Ratatui TUI and the MCP server. ADRs 004–014 delivered
that core and its capabilities — the task engine, process manager, Git manager,
build manager, library manager, service runtime, migration manager, health and
observability layers, the TUI, and the MCP server.

Every adapter was, however, wired with its **own** application core in its own
process. `bootstrap()` built a fresh task engine, process manager, runtime-state
store, and log store for whichever interface was launched. The ADS already
recorded the consequence: in ADR 014, "task and runtime state are per-process;
two MCP sessions are two engines; a task id from one is not visible in the
other."

`tasks/015.md` made that the problem to solve. Multiple interfaces operating on
one developer environment must share one authoritative runtime, because
per-process cores cause:

- inconsistent service status between the TUI and MCP;
- duplicate process startup;
- inconsistent task history;
- the inability to stop a process another interface started;
- conflicting Git, build, and migration operations;
- inconsistent logs and health.

The task imposed hard constraints:

- preserve the **single-binary** model — `daemon`, `tui`, and `mcp` are
  subcommands, not separate executables, and no mandatory external backend;
- the daemon is the **authoritative owner** of mutable runtime state; clients
  hold only refreshable caches;
- local IPC over Unix domain sockets, with a **versioned, structured protocol**
  — not arbitrary shell commands, and no public TCP port for the MVP;
- one shared `OrchestratorClient` abstraction that both the TUI and MCP call;
  MCP stays a thin adapter and never sees raw protocol details;
- automatic daemon startup from `tui`/`mcp` with readiness waiting, single
  instance, stale-socket handling, and safe permissions;
- shared task and service state across clients, with duplicate startup prevented;
- bounded event streaming, reconnection, and request correlation;
- authorization through the **same** policy engine, with a client-provided
  `confirmed: true` explicitly insufficient;
- graceful explicit shutdown, distinct from client exit;
- a `daemon status` diagnostic;
- protocol versioning with actionable incompatibility errors;
- Docker, Kubernetes, remote access, a public API, and configuration editing all
  explicitly out of scope.

## Decision

Introduce **two new crates** and a shared runtime:

- `service-orchestrator-client` (`crates/client`): the `OrchestratorClient`
  abstraction, the versioned IPC protocol, socket resolution, auto-start, and the
  `RemoteClient` daemon-backed implementation.
- `service-orchestrator-daemon` (`crates/daemon`): the local IPC server, the
  request router, single-instance locking, and lifecycle. It contains no business
  logic.

The `service-orchestrator` binary gains three subcommands over the same core:

```text
service-orchestrator daemon          # run the shared local runtime (foreground)
service-orchestrator daemon status   # report whether it is running
service-orchestrator daemon stop     # ask it to shut down
service-orchestrator tui             # TUI client over IPC
service-orchestrator mcp             # MCP client over IPC
```

```text
TUI ─┐
     ├─▶ OrchestratorClient ─▶ Unix socket ─▶ daemon ─▶ Application ─▶ Task Engine
MCP ─┘
```

### Why a local daemon

A shared runtime needs a single owner of mutable state that outlives any one
interface. Options were weighed:

- **Reassigning ownership per interface** (for example, "the first interface
  becomes the leader") is fragile: leadership changes on exit, and the state dies
  with the leader.
- **A separate server application** violates the single-binary and
  no-external-backend constraints.
- **A local daemon** — a detached process running the same binary — gives one
  long-lived owner, survives client exit, and keeps the core, engine, and process
  registry in exactly one place.

The daemon is deliberately thin: it validates versions, decodes parameters, calls
the shared `Application`, and encodes the structured result. It duplicates no
application logic, so an operation has identical behavior, policy, and errors
whether reached in-process (tests) or over IPC.

### Why Unix domain sockets

macOS is the target platform, and a Unix domain socket is the natural local,
filesystem-permissioned IPC primitive:

- **Local and private by construction** — no network interface is involved.
- **Access control through the filesystem** — the socket is created in a
  per-user `0700` directory and restricted to `0600`, so another local user
  cannot connect.
- **Simple framing** — a stream can carry a length-prefixed JSON frame with no
  extra dependency.
- **No port allocation or firewall prompts**, unlike TCP.

A public TCP port was explicitly rejected for the MVP: it widens the attack
surface and none of the scoped features need remote access.

### Why the single-binary model is preserved

The daemon and the clients are the *same* executable. `daemon` runs the runtime;
`tui` and `mcp` connect to it. This keeps distribution, signing, and the user's
mental model simple: one binary, a handful of subcommands, no service installer.
Auto-start launches the current executable again with the `daemon` subcommand.

### How TUI and MCP share state

Both interfaces hold `Arc<dyn OrchestratorClient>`. `Application` implements the
trait for in-process tests; `RemoteClient` implements it as a socket proxy. The
two implementations expose the same method set, so the interfaces are unaware of
the transport.

The daemon owns:

- the active task engine and its history;
- the process registry and per-service runtime state;
- health state and log subscriptions;
- the validated configuration snapshot (read-only to clients).

Because there is one engine, a task submitted by MCP is immediately visible in
the TUI, and vice versa. Because there is one runtime-state store, starting an
already-running service returns `AlreadyInDesiredState` instead of spawning a
second process.

### How process ownership works

The daemon's process manager owns every managed service. Starting a service from
one interface registers it once; every interface then sees the same state and can
stop or restart it. On shutdown the daemon applies the existing managed-process
shutdown policy.

Runtime state is in memory. A daemon that was killed does **not** adopt the
processes it previously managed: it starts with zero managed services and never
signals a process by PID or port, so a reused PID can never cause an unrelated
process to be killed. This is the safe behavior, and the limitation (surviving
children become unmanaged) is documented rather than worked around with unsafe
PID-based adoption.

### How authorization is enforced

A request carries the caller's `allow_destructive` intent and its `ClientKind`.
The router honors the intent **only** for an interactive (`tui`) peer, and then
still rebuilds an `Application` carrying the authorization and runs the shared
policy engine. MCP and CLI peers can never self-confirm. A caller-supplied
`confirmed: true` is therefore never sufficient — the policy engine remains the
sole authority, exactly as ADR 014 required.

### Protocol, versioning, and framing

The protocol is length-prefixed JSON (a big-endian `u32` length followed by the
payload), capped at 8 MiB. Each request carries a `request_id` used to correlate
the response and to reason about retries. `PROTOCOL_VERSION` versions the shape; a
mismatched request is refused before it is interpreted, with
`RuntimeUnavailable { code: "PROTOCOL_INCOMPATIBLE" }` and a message naming both
versions. No complex RPC framework was introduced.

### Reconnection and idempotency

`RemoteClient` reconnects transparently if the connection is already gone before
a request is written, then retries once. A request that was **written** but
produced no response is never retried — the mutation may have been applied — and
the caller receives `DAEMON_UNAVAILABLE` instead. Log subscriptions are
re-established after a reconnect.

## Alternatives considered

- **A single long-lived leader interface instead of a daemon.** Rejected:
  leadership is fragile and state dies with the leader; closing one window would
  strand the other interface.
- **An HTTP/JSON backend or gRPC service.** Rejected: it introduces a port, a
  network surface, and an external-backend requirement the task forbids.
- **A separate `service-orchestrator-daemon` binary.** Rejected: it breaks the
  single-binary model and doubles distribution/signing.
- **D-Bus / Mach ports / XPC.** Rejected: heavier, less portable across the
  crate's Unix/macOS targets, and unnecessary for a local stream of JSON frames.
- **Proxying the whole `Application` port surface generically over IPC** (for
  example, `serde`-ing a boxed use case). Rejected in favour of explicit
  operation routing: the wire vocabulary stays stable, reviewable, and decoupled
  from internal Rust types.
- **A `run_shell`-style escape hatch.** Rejected by `AGENTS.md` §23 and by the
  same reasoning as ADR 014.
- **Accepting a client `confirmed: true`.** Rejected: the policy engine must
  remain the authority, and a non-interactive client must never self-authorize.
- **PID/port-based process adoption after a restart.** Rejected: PID reuse makes
  it unsafe; a fresh daemon therefore adopts nothing and never signals an
  unrelated process.
- **TCP with a port file.** Rejected: a Unix socket gives filesystem-level access
  control and avoids binding a network interface.

## Consequences

### Positive

- One authoritative runtime: the TUI and MCP observe identical services, tasks,
  logs, and health, with no duplicated logic and no divergent behavior.
- Closing an interface never stops the daemon or the developer's services; an
  explicit `daemon stop` is required.
- Duplicate service startup is prevented by one shared runtime-state store.
- Every destructive operation passes through one policy engine at one boundary;
  a non-interactive client can never self-confirm.
- The client boundary is the only thing an interface depends on, so adding a
  future CLI or GUI is a new client, not a new core.
- The single binary is preserved and no external backend or network port is
  needed.

### Negative / trade-offs

- The daemon is a new process lifecycle to reason about (ownership, readiness,
  stale sockets), and the two new crates add surface. This is the cost of one
  authoritative runtime.
- Managed-process recovery after a crash is not implemented; surviving children
  become unmanaged. Documented in `docs/local-daemon.md` §13.
- Runtime state remains in memory; a restarted daemon starts from zero.
- The protocol must be versioned and kept compatible; a breaking change requires
  a version bump and refuses older clients (by design).
- Event delivery is best-effort: a slow subscriber drops events and must
  resynchronise.
- Auto-start is a detached child, not a `launchd` agent, so it does not restart
  after reboot.

## Follow-up

- A persisted, identity-validated runtime-state store could let a restarted
  daemon safely recover — or explicitly disown — processes it previously managed.
- Optional macOS `launchd` integration could supervise the daemon across logins.
- Configuration editing from the TUI/MCP could be introduced as a dedicated,
  policy-gated operation on top of the existing read-only configuration snapshot.
- Additional IPC transports (for example a streamable transport) can be added
  behind the existing client seam without changing the daemon's routing.

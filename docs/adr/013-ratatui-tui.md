# ADR 013: The Ratatui TUI Adapter

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

ADR 001 established the core-first architecture and named the Ratatui terminal
UI as one of two interfaces over a single application core; ADR 004 defined that
core; ADRs 005–012 delivered the task engine, process manager, Git manager,
build manager, library manager, service runtime, migration manager, and health
and observability layers. By the time of this decision every capability the
developer needs already existed behind the application core, but no interface
exposed them as one cohesive experience.

`tasks/013.md` asked for a production-quality, keyboard-first Ratatui interface
that integrates those existing capabilities, with hard constraints:

- the TUI is an **adapter** and must not execute Git, Maven, npm, Liquibase, or
  OS processes, or duplicate application logic;
- all mutating operations must go through the application core and its policy
  engine, and destructive operations must require explicit confirmation;
- the interface must remain responsive while long-running work executes, and
  must prefer engine events over polling;
- selection must use strongly typed identifiers, never visible row indices, and
  must stay stable under filtering and sorting;
- the terminal must be restored on exit, error, Ctrl+C, panic, and resize, and
  small terminals must not panic;
- testing must use Ratatui's `TestBackend` and fakes, never real repositories;
- MCP, Kubernetes, Docker, new managers, a custom scheduler, and arbitrary shell
  execution are explicitly out of scope.

## Decision

Add a `service-orchestrator-tui` crate and wire it into the `service-orchestrator`
binary as `service-orchestrator tui`.

```text
key / task event / async result / tick
        │
        ▼
    App (state machine) ── Effect ──► terminal event loop ──► Application query
        │
        ▼
   UiState change
        │
        ▼
   router::draw(&App) ──► Ratatui ──► terminal
```

The crate depends only on `application`, `domain`, `execution` (to name task
events), and `ratatui`. The composition root in `src/main.rs` loads and validates
configuration, constructs the infrastructure adapters, registers their executors,
builds the task engine, wires the application core, and runs the interface.

### Why Ratatui

Ratatui is the de-facto standard Rust TUI library: it has a small, stable widget
model, no async runtime of its own, and an in-memory `TestBackend`, which makes
the rendering path testable without a terminal. It fits the project's Rust-first,
single-binary, macOS-native goals and adds no service dependency.

### The TUI is an adapter

The interface reads application state, renders it, and turns input into
application use cases. It contains no Git, build, process, migration, or workflow
logic. Every mutating key maps to an `Action`, which maps to an application call;
long-running work becomes a task or workflow the interface observes. This is the
same rule the MCP server follows, so the two interfaces cannot drift.

To preserve the rule mechanically, the crate depends on the application crate
and never on the infrastructure crates. Adding a Git or Maven call would require
an impossible dependency edge, so the boundary is enforced by the manifest, not
by convention.

### UI state and application state are separate

`UiState` holds presentation state (active screen, cursor, typed selection,
dialogs, search, scroll) and the cached results of on-demand inspections.
`Snapshot` is a read-only cache of application state, replaced wholesale on
refresh. The application core remains authoritative and never observes UI state.
Splitting them keeps the interface a pure adapter and keeps the state machine
testable in isolation.

### Asynchronous event handling

Terminal input is read on a dedicated thread (crossterm's reader is blocking) and
funnelled onto one Tokio `mpsc` channel together with task events, realtime log
entries, spawned-query results, and a tick. The loop draws, then awaits one event.
No Git fetch, build, migration, process start, or health check ever blocks the
renderer; those are tasks or spawned queries whose results return on the same
channel. `Effect` values are descriptions, so the state machine emits "inspect
Git status for X" and the event loop — the only component allowed to do I/O —
runs it.

### Event-driven, not polling

The interface subscribes to the task engine's broadcast stream and to per-service
log subscriptions, so task and log updates arrive as they happen. A light tick
rebuilds the in-memory snapshot roughly once a second; no repository is polled.
On-demand inspections (Git status, branches, artifact verification, version
alignment) run only when the developer asks.

### Multi-selection uses typed identifiers

Selections are `BTreeSet<ServiceId>` / `BTreeSet<LibraryId>`, never row indices.
Filtering and searching therefore cannot change or lose a selection, and bulk
operations target exactly the selected entities. A `TestBackend` test asserts
that a selection survives a filter change.

### Policy and confirmation

Destructive operations first run with standard authorization. The application
denies them with `ApplicationError::PermissionDenied`; the interface opens a
confirmation dialog holding the intent. Confirming re-runs the operation with
destructive authorization. Because the application performs the same check
itself, a UI confirmation is never the authorization itself.

### Performance

Rendering is stateless and cheap: each screen renders from already-read state,
and the tables rebuild their `TableState` each frame. The task list is bounded
(`TASK_HISTORY_LIMIT`), the log buffer is bounded (`LOG_BUFFER_CAPACITY` = 2000),
and the snapshot refresh reads in-memory state only. The design targets tens to
hundreds of services without scanning repositories per frame.

## Alternatives considered

- **Put the TUI in the binary crate.** Rejected: the interface would share a
  crate with the composition root and could reach the infrastructure adapters
  directly, eroding the adapter boundary. A separate crate makes the dependency
  direction visible and enforceable.
- **A single `App` that owns the `Terminal` and does I/O inline.** Rejected: the
  state machine would be untestable without a terminal and would couple input
  handling to rendering.
- **Poll the application for status on every tick.** Rejected: it would run
  expensive repository scans continuously. On-demand effects plus engine events
  keep the interface both correct and light.
- **Model selection by row index.** Rejected: it breaks under filtering and
  sorting and invites off-by-one bugs. Typed identifiers are stable.
- **Reuse `TaskManager` for listing tasks.** Rejected as insufficient: the
  application's boundary exposes submission and lookup by id, not the list and
  event API the interface needs. A small, interface-facing `TaskObserver` trait —
  implemented by the concrete `TaskEngine` and by a test fake — keeps the
  application boundary narrow.
- **Let the UI confirmation stand in for authorization.** Rejected: the policy
  engine must remain the single authority, and the application must enforce it
  independent of any interface.
- **Embed a custom slow-operation scheduler in the UI.** Rejected: concurrency,
  dependencies, cancellation, and retry already belong to the task engine.

## Consequences

### Positive

- One application core, two interfaces, no duplicated behavior.
- The state machine is pure and unit-testable; the rendering path is testable
  through `TestBackend`.
- Destructive actions cannot be triggered without the application's own
  authorization.
- The interface stays responsive and cheap regardless of how much work is in
  flight.
- The dependency edge from `tui` to the infrastructure crates does not exist, so
  future contributors cannot accidentally bypass the application core.

### Negative / trade-offs

- The `TaskObserver` trait is a second, narrow boundary in addition to
  `TaskManager`; it exists because the engine's richer API is genuinely needed by
  an observer and is implemented for both the real engine and a fake.
- The snapshot is refreshed on a periodic tick rather than pushed on every state
  change, so a change made outside the interface (for example by a running task
  executor writing runtime state) may take up to about a second to appear. Task
  and log events are immediate.
- On-demand inspections cache their last result in `UiState`, which is
  presentation state by design; the authoritative value is always re-read from
  the application on the next request.
- The TUI adds a dependency on `ratatui`/`crossterm`; this is confined to one
  leaf crate and to the binary.

## Follow-up

- Wire the remaining interfaces (`mcp`, `doctor`) in the composition root as
  separate tasks.
- If the application gains a safe configuration-update use case, Settings can
  become editable without moving logic into the interface.
- If the application exposes a combined bulk-build or bulk-health use case, the
  per-service fallback in the interface can be replaced by a single workflow.

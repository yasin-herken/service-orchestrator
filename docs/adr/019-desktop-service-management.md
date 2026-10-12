# ADR 019: Desktop Service Management Through the Shared Daemon

- **Status:** Accepted
- **Date:** 2026-10-12
- **Deciders:** Service Orchestrator maintainers

## Context

`tasks/019.md` asked the read-only Tauri desktop foundation (ADR 018) to become
a functional service-management interface: list and select services, start,
stop, and restart them (individually and in bulk), inspect their details, and
watch real task progress — with realtime updates, confirmations, and structured
errors.

The foundation already established the shape: the GUI is a thin adapter over the
`client` boundary and a client of the single local daemon (ADR 001/015), which
owns the task engine, process manager, service runtime, and policy engine. The
task therefore had to add writes **without** re-introducing the failure modes a
second implementation invites:

- a second start/stop implementation that drifts from the TUI's and MCP's
  policy behaviour;
- a second task engine, process manager, or scheduler living beside the daemon;
- optimistic UI that reports success the daemon never confirmed;
- a confirmation dialog that quietly becomes the authorization boundary;
- duplicate submissions from rapid clicks creating duplicate processes.

The daemon's event surface is also a constraint: the forwarded `ServerEvent`
stream carries task, log, and shutdown events, not service runtime or health
transitions.

## Decision

Extend the adapter with validated lifecycle commands and a task-event-driven
service screen, keeping every decision below the boundary.

- **One use case per action, over the existing client.** `start_service`,
  `stop_service`, `restart_service`, the three bulk variants, and
  `check_service_health` map directly onto `OrchestratorClient` methods. The GUI
  never runs an operating-system command, never schedules, and never orders
  anything. Restart is always the daemon's coordinated stop-then-start — never a
  frontend stop/start pair.
- **Normalized `OperationOutcome`.** Commands return a flat
  `{ status, task_id, workflow_id, kind }` where `status` is `task`,
  `workflow`, or `already_in_desired_state`. The frontend branches on this and
  follows the real ids to observe progress; it never re-derives meaning from a
  tagged enum and never invents success.
- **Bulk is one workflow.** A multi-service start/stop/restart submits a single
  workflow to the daemon, so dependency ordering, concurrency limits, runtime
  validation, and duplicate protection stay in the shared scheduler. The
  frontend does not loop.
- **Input validated at the Rust boundary.** Every id is parsed into its domain
  value type before any IPC call; an invalid identifier or an empty bulk
  selection fails fast with `INVALID_INPUT`. Single commands need only a service
  id because service ids are globally unique; bulk commands take a workspace id
  plus ids.
- **The policy engine remains the authority.** Lifecycle commands are
  `SafeWrite`, submitted by the GUI as a `Cli` client — the same classification
  the TUI and MCP use. A confirmation dialog is a usability affordance that does
  not bypass or weaken the daemon's authorization.
- **Realtime refresh is task-driven.** The GUI subscribes to the existing
  `daemon://task` stream (registered once, cleaned up on unmount, re-established
  on reconnect). A bounded map of tasks and workflows plus a `terminalRevision`
  counter let the dashboard and services screens refetch exactly once when a
  task settles. Because the daemon does not forward service runtime/health
  transitions, service state is refreshed on task settlement and after each
  command; progress is derived from real `Task` events grouped by workflow id.
- **Selection is a set of stable ids.** Multi-selection, select-all-visible,
  clear, and selection by type/state operate on ids, so selection survives
  filtering, sorting, and detail navigation. Bulk operations act only on the
  explicit selection, in a deterministic order.
- **Disconnection never replays uncertain writes.** While disconnected the
  screen shows a status notice, disables every mutating action, keeps loaded
  data, and issues no mutating request. Reconnecting refreshes state from the
  daemon; the user re-issues an operation whose outcome was uncertain.

## Alternatives considered

- **A dedicated interactive `ClientKind` for the GUI.** Rejected for this task:
  the lifecycle commands are `SafeWrite`, which a `Cli` client may already
  perform, and no destructive operation is exposed. Introducing a new kind and
  protocol change would be a larger, separately-justified step.
- **Forwarding service runtime/health events to the GUI.** Rejected: it would
  mean extending the daemon's `ServerEvent` surface and the transport protocol.
  The task-event-driven refresh plus post-command reload meets the requirement
  without a protocol change; the gap is documented as a limitation.
- **Optimistic row updates.** Rejected (`tasks/019.md` §24): the only state the
  GUI shows as final is the daemon's. Pending is shown as busy, not as success.
- **A frontend bulk loop.** Rejected (`tasks/019.md` §14–§16): it would bypass
  dependency ordering, concurrency limits, and duplicate protection, and create
  a second scheduler.
- **Treating the confirmation dialog as authorization.** Rejected
  (`tasks/019.md` §27): confirmations are UX only; the policy engine authorizes.
- **A Redux-style store for selection and pending operations.** Rejected: the
  state is small and derivable. Contexts plus two pure utility modules
  (`serviceSelection`, `taskProgress`) keep one owner per concern.

## Consequences

- A service started from the GUI is the same task the TUI and MCP observe: one
  daemon, one task engine, one process manager, one policy engine.
- The adapter's unit tests never run a process; `tests/daemon_ipc.rs` drives a
  real daemon with fake lifecycle executors over an in-memory runtime-state
  store and asserts the wire shape, error codes, single/bulk behaviour, partial
  failures, and the shared-daemon invariant.
- The frontend is testable with a typed mock of the Tauri API; screen behaviour
  (selection, bulk, confirmations, progress, disconnection) is verified without
  a webview.
- Because the daemon's event surface is task-only, runtime/health changes made
  by another interface become visible after the next task settles or refetch;
  adding service events later would let the GUI drop the task-driven reload.
- The GUI still presents itself as a `Cli` client; a dedicated interactive kind
  and any destructive-confirmation flow remain deferred, and no destructive
  operation is exposed by this interface.

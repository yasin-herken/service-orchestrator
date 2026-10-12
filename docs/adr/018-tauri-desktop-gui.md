# ADR 018: The Tauri 2 Desktop GUI as a Thin Adapter

- **Status:** Accepted
- **Date:** 2026-10-12
- **Deciders:** Service Orchestrator maintainers

## Context

`tasks/018.md` asked for a desktop GUI foundation: a native macOS window with a
React frontend, delivering only the shell and the read path (dashboard, service
list, connection status), built on Tauri 2 and connecting to the existing local
daemon over the existing IPC.

The repository already has two interfaces over one application core (ADR 001):
the Ratatui TUI (ADR 013) and the MCP server (ADR 014), both clients of the
single local daemon (ADR 015). The GUI had to become the third without
introducing any of the failure modes an interface rewrite invites:

- a second task engine or process manager living beside the daemon;
- a second, competing source of connection or workspace state;
- a second start/stop implementation that drifts from the TUI's and the MCP
  server's policy behaviour;
- a dependency cost imposed on the backend, daemon, TUI, and MCP builds.

The task also required a strict, no-`any` TypeScript codebase, minimal Tauri
capabilities, and testability without Git, Maven, Liquibase, or real
repositories.

## Decision

Add a Tauri 2 application under `apps/desktop`, consisting of a React +
TypeScript + Vite frontend and a Rust adapter crate in
`apps/desktop/src-tauri`, and treat the adapter as an interface in exactly the
sense of ADR 001/013/014.

- **The adapter depends on the client boundary, not on the transport.** It
  consumes the workspace crates as path dependencies and holds the same
  `Arc<RemoteClient>` used by the TUI and MCP. Command logic is a set of query
  functions over `OrchestratorClient`, so the adapter provably executes shared
  use cases and never speaks IPC itself.
- **The crate is its own Cargo workspace.** The root `Cargo.toml` excludes
  `apps/desktop/src-tauri`, so the Tauri `wry`/`tao`/`objc` tree is compiled
  only for the GUI. The path dependencies keep it honest: the adapter cannot
  drift from the core it embeds.
- **Read-only commands only.** Six commands exist: `get_daemon_status`,
  `list_workspaces`, `list_services`, `get_workspace_status`,
  `get_service_status`, and `get_connection_state`. Nothing starts, stops, or
  edits anything; service control and destructive confirmation are task 019.
- **The adapter owns one connection and one client.** A shared `AppState` holds
  the client behind a connect lock, so a burst of commands cannot start several
  daemons, and every command reuses the same authoritative connection.
- **A connection state machine is the single source of truth.** The adapter
  tracks `Connecting`/`Connected`/`Reconnecting`/`Disconnected`/`Error`,
  classifies probe and command outcomes, and pushes only transitions as
  `daemon://connection` events. The frontend never counts failures itself.
- **Passive monitoring does not poll an unreachable socket.** Probes are guarded
  by a reachability check, so an idle monitor never issues requests against a
  daemon the user stopped. Starting a daemon remains the business of an explicit
  action (Retry, or a real command) and of the shared client's single
  reconnect-retry, which is identical to the TUI's and MCP's behaviour while
  they are attached. The GUI's own executable is never eligible as a daemon: the
  adapter resolves the real `service-orchestrator` binary (explicit override,
  `PATH`, sibling, `target/{debug,release}` walk-up) and exports it through
  `SERVICE_ORCHESTRATOR_DAEMON_BIN` for the shared autostart in
  `crates/client/src/socket.rs`.
- **Task events are forwarded, not re-derived.** The adapter subscribes to the
  daemon's task-event stream and re-emits events verbatim (plus a synthetic
  `Lagged` payload when the subscription falls behind); the activity feed reads
  those events rather than polling tasks.
- **Minimal capabilities.** `core:default` only; no shell, filesystem, HTTP, or
  process plugin. A strict CSP, and a navigation filter that accepts the
  `tauri:` scheme plus the Vite dev server in debug builds only.
- **Typed frontend, contexts, one IPC call per transition.** A single typed API
  module wraps `invoke`/`listen`; React contexts hold connection, workspace,
  and task-event state; the workspace fetches are gated on the connection so a
  disconnected GUI does not hammer the socket.
- **The GUI presents itself as a `Cli` client** for now, because this task's
  command set is entirely read-only. A dedicated interactive client kind and
  the destructive-confirmation flow are deferred to task 019, where mutating
  commands arrive; the shared policy engine already ignores destructive intent
  from non-interactive peers, so nothing here weakens that boundary.

## Alternatives considered

- **A web UI against a new HTTP backend.** Rejected: `AGENTS.md` §22 forbids
  introducing an external HTTP backend merely to serve a UI. The GUI is a local
  desktop application; the daemon's Unix-socket IPC is the existing, correct
  transport.
- **A separate GUI-side application core.** Rejected: it would duplicate
  configuration parsing and state (ADR 001), and the GUI would disagree with
  the TUI and MCP about what the world looks like.
- **Electron or a plain webview with Node modules.** Rejected: Tauri 2 keeps a
  single native binary, avoids shipping a Node runtime to users, and lets the
  frontend be capability-restricted by construction.
- **Managing GUI state with Redux or a similar store.** Rejected: the state is
  small (connection, workspace selection, one activity feed) and derived from
  the adapter; contexts plus a small `useAsync` hook keep one owner per piece of
  state without a second store to synchronize.
- **Polling the daemon from React.** Rejected: the adapter already owns
  connection detection and the task-event subscription; duplicate polling would
  give the frontend a second opinion about connection health.
- **Letting the GUI re-execute itself as the daemon (the default single-binary
  behaviour).** Rejected: it would open a second window instead of a daemon and
  orphan the process. Hence the binary resolver plus the client-crate
  `SERVICE_ORCHESTRATOR_DAEMON_BIN` override.
- **Treating every failed probe as a hard disconnect.** Rejected: a daemon
  restart would flash a failure banner. A bounded reconnection streak keeps a
  restart quiet and settles on `Disconnected` only after repeated failures.

## Consequences

- The GUI shares state with the TUI and MCP for free: one daemon, one task
  engine, one process manager, one policy engine. A task visible in one
  interface is visible in all of them.
- The adapter's unit tests never touch Git, Maven, Liquibase, or a real process;
  `tests/daemon_ipc.rs` runs a real daemon with no executors and asserts the
  exact wire shape the frontend consumes, the structured error codes, and the
  shared-daemon invariant.
- The frontend is testable with a typed mock of the Tauri API, so screen
  behaviour is verified without a webview.
- A change to a shared use case or its serialized shape reaches the GUI through
  the same path as the TUI and MCP; the DTO module is the single translation
  point, and the integration test fails loudly if a shape drifts.
- The GUI's autostart depends on being able to resolve the `service-orchestrator`
  binary; in a packaged bundle that remains a task-019 packaging concern (DMG,
  signing, and notarization are out of scope here).
- Read-only commands mean the GUI cannot yet be used to actually prepare an
  environment; the shell it establishes is the foundation task 019 builds the
  mutating screens on.

# Desktop GUI (Tauri 2)

The desktop GUI is the graphical interface to Service Orchestrator
(`tasks/018.md`, `tasks/019.md`). It is a Tauri 2 application — a React +
TypeScript frontend (`apps/desktop`) inside a native macOS window, with a small
Rust adapter crate (`apps/desktop/src-tauri`) behind it.

It is an **interface**, exactly like the Ratatui TUI and the MCP server:

```text
React webview  ──invoke──▶  Tauri commands  ──▶  OrchestratorClient  ──▶  the one daemon
React webview  ◀──emit────  connection / task events ◀── tracker / subscription
```

The GUI never parses configuration, schedules tasks, or runs a process. It
holds no second task engine, no second process manager, and no competing state:
every read and every mutation goes through the shared client boundary to the
single local daemon (`docs/local-daemon.md`), so a task started by an agent in
MCP, by a human in the TUI, or by the GUI is the same task every interface
displays.

## 1. Scope

This document covers the shell, the dashboard, and **service lifecycle
management**: listing, selection, start, stop, restart, bulk operations, detail
inspection, task progress, realtime refresh, and confirmation/error handling.

| Delivered | Not yet delivered (later tasks) |
| --- | --- |
| Application shell: sidebar, header, workspace selector | Git, build, library, and migration screens |
| Dashboard: real workspace/daemon counters, task activity feed | A full log viewer (the detail panel links to the Logs screen) |
| Services table: state, health, branch, port, last operation | Service or workspace creation/editing |
| Multi-selection with stable ids, select-all-visible, by type/state | Docker and Kubernetes targets |
| Start / stop / restart, single and bulk | DMG packaging, code signing, notarization, CI/CD |
| Service detail panel with configuration, observed state, diagnostics | Destructive operations (`git reset`, `git clean`, force stop) |
| Realtime task-driven refresh; confirmation, error, and progress dialogs | Dedicated interactive client kind (see §6) |
| Connection status with disconnect banner, Retry, and safe reconnect | |

No business logic lives in the GUI. Starting a service runs the daemon's
`StartService` use case; the GUI only submits a request and then displays the
authoritative result.

## 2. Layers and files

```text
apps/desktop/
├── index.html                 # webview entry
├── package.json               # scripts: dev, build, typecheck, test, tauri
├── scripts/preflight.mjs      # builds the root service-orchestrator binary
├── vite.config.ts             # dev server on :5173, strict port
├── vitest.config.ts           # jsdom + testing-library
├── tsconfig.json              # strict, no `any`
├── src/
│   ├── main.tsx, App.tsx      # providers + routing by nav state
│   ├── types/                 # Rust wire shapes + navigation definition
│   ├── services/              # typed desktop API + error normalization
│   ├── context/               # Connection, Workspace, TaskEvents providers
│   ├── hooks/
│   │   ├── useAsync.ts        # loading/error/data with a stale-response guard
│   │   └── useServiceActions.ts # lifecycle submissions, pending, operations
│   ├── utils/                 # pure logic: serviceQuery, serviceSelection, taskProgress
│   ├── layouts/AppShell.tsx   # sidebar + header + content
│   ├── components/            # presentation only
│   ├── pages/                 # Dashboard, Services, placeholders
│   └── styles/global.css      # light + dark via prefers-color-scheme
└── src-tauri/
    ├── Cargo.toml             # its own workspace; excluded from the root one
    ├── tauri.conf.json        # identifier, CSP, dev/build commands
    ├── capabilities/default.json
    ├── icons/icon.png         # development placeholder icon
    └── src/
        ├── main.rs, lib.rs    # event loop, window, navigation filter, command registration
        ├── commands.rs        # the Tauri commands (read + lifecycle)
        ├── queries.rs         # read logic over OrchestratorClient
        ├── mutations.rs       # lifecycle logic over OrchestratorClient
        ├── validation.rs      # domain-typed identifier parsing at the boundary
        ├── dto.rs             # wire view models incl. ServiceDetail/OperationOutcome
        ├── state.rs           # shared AppState, connection, event forwarding
        ├── connection.rs      # connection state machine
        ├── daemon_bin.rs      # locating the daemon executable
        ├── events.rs          # event names and payload mapping
        └── error.rs           # structured command errors
```

The adapter crate is intentionally **not** a member of the root Cargo
workspace: Tauri drags in a large `wry`/`tao`/`objc` dependency tree that the
backend, TUI, MCP, and daemon builds should not pay for. The root
`Cargo.toml` excludes it, and the adapter consumes the workspace crates as path
dependencies, so it still compiles against exactly the same core.

## 3. The commands

Every command is a thin wrapper: validate input at the Rust boundary, delegate
to the shared client (a query or mutation), and map the answer into a view
model. Command arguments are camelCase on the JavaScript side (`workspaceId`,
`serviceId`, `serviceIds`) and snake_case in Rust, which is Tauri's default
mapping.

### Reads

| Command | Backing use case | Returns |
| --- | --- | --- |
| `get_daemon_status` | `RemoteClient::daemon_status` | pid, endpoint, protocol, clients, tasks, managed services |
| `list_workspaces` | `list_workspaces` | workspace summaries (id, name, root, service count) |
| `list_services` | `get_workspace_status` | one row per service in a workspace |
| `get_workspace_status` | `get_workspace_status` | workspace plus all service rows |
| `get_service_status` | `get_service_status` | one service row, checked against its workspace |
| `get_service_detail` | `get_service` + `get_service_status` + `get_service_diagnostics` | configuration, observed state, and diagnostics |
| `get_connection_state` | adapter tracker | the current `ConnectionSnapshot` |

`list_services` reuses the `get_workspace_status` round trip rather than issuing
a second IPC method: the daemon returns everything the table renders in one
response. `get_service_status` and `get_service_detail` re-validate that the
service belongs to the requested workspace, so a stale selection cannot silently
show a service from another workspace. `get_service_detail` is the only command
that fans out to more than one use case; it merges them into one view so the
panel never shows a half-updated service.

### Lifecycle (mutations)

| Command | Backing use case | Returns |
| --- | --- | --- |
| `start_service` / `stop_service` / `restart_service` | `start_service` / `stop_service` / `restart_service` | an `OperationOutcome` |
| `start_services` / `stop_services` / `restart_services` | `start_services` / `stop_services` / `restart_services` | an `OperationOutcome` for the workflow |
| `check_service_health` | `check_service_health` with `HealthCheckScope::All` | an `OperationOutcome` |

Every lifecycle command takes only a `service_id` (single) or a `workspace_id`
plus `service_ids` (bulk); service ids are globally unique, so a single command
needs no workspace. Every id is parsed into its domain type
(`validation.rs`) before any IPC call, so an invalid or empty selection fails
fast with `INVALID_INPUT` and never reaches the daemon.

An `OperationOutcome` is flatter than the core's tagged `OperationResult` so the
frontend can branch on `status` and follow real ids:

```json
{ "status": "task", "task_id": "12345", "workflow_id": null, "kind": "start_service" }
```

- `status` is `task`, `workflow`, or `already_in_desired_state`;
- `task_id` is set for a single task, `workflow_id` for a bulk workflow;
- `kind` is the task/workflow label (`start_service`, `start_services`, …).

Restart is always a **single coordinated** call (`restart_service` /
`restart_services`): the daemon owns the stop-then-start lifecycle, so the GUI
never issues an independent stop/start pair (`tasks/019.md` §13). A bulk request
is one workflow; the GUI does not schedule or order anything itself.

### Errors

A failing command rejects with `{ code, message }`:

- `DAEMON_UNAVAILABLE` — the daemon is unreachable (also drives the banner);
- `PROTOCOL_INCOMPATIBLE` — the daemon speaks another protocol version;
- `INVALID_INPUT` — the identifier failed domain validation (or a bulk
  selection was empty);
- `NOT_FOUND` — the service belongs to a different workspace;
- the application error's variant name (for example `WorkspaceNotFound`,
  `ServiceNotFound`), including a **permission denial** from the policy engine;
- `INTERNAL` — an adapter-side failure.

Messages are short and user-safe: no stack traces, no secrets, no raw IPC
frames. Connection-shaped failures carry the daemon log path as a hint.

## 4. Events and realtime updates

The adapter pushes two event streams; the frontend subscribes once and keeps its
state from them.

| Event | Payload | Meaning |
| --- | --- | --- |
| `daemon://connection` | `ConnectionSnapshot` | a connection state transition |
| `daemon://task` | `TaskEvent` | a forwarded task-engine event |

`daemon://task` keeps the daemon's externally tagged shape
(`{"TaskCreated": { … }}`). If the local subscription falls behind, the adapter
emits a synthetic `{"Lagged": {"dropped": N}}` instead of silently losing
events, and the activity feed shows that events were dropped.

`TaskEventsContext` keeps a bounded, ordered map of the tasks and workflows it
has seen (200 tasks, 50 workflows) and a `terminalRevision` counter that bumps
whenever a task reaches a terminal state. The dashboard and the services screen
depend on that counter, so a settled task triggers exactly one authoritative
reload of service state. The progress dialog derives per-service status from
those same events; a workflow's tasks are grouped by the `workflow` id the
daemon attaches to each `Task`.

**A task-id or workflow-id is always real.** The GUI never fabricates progress
percentages and never marks an operation successful on its own
(`tasks/019.md` §24, §25).

## 5. Connection model

The adapter is the **single source of truth** for connection state, so the
frontend never counts failures itself:

```text
Connecting ──success──▶ Connected ──probe fails──▶ Reconnecting ──5 failures──▶ Disconnected
                          ▲                              │
                          └────────── success ───────────┘
                            any protocol/other failure ──▶ Error
```

- **Probe cadence.** A passive monitor calls `get_daemon_status` every five
  seconds while a client exists, guarded by a socket-reachability check.
- **No idle resurrection.** While the socket is unreachable the monitor records
  failures and issues no call, so a daemon the user stopped stays stopped. A
  probe already in flight when the daemon stopped can still trigger the shared
  client's single reconnect-retry (`connect_or_start`), which starts the daemon
  again — exactly the behaviour the TUI and MCP have while they are attached.
  From a fully disconnected state, only a command or the Retry button starts a
  daemon.
- **Retry.** The banner's Retry button asks the adapter for the daemon status; a
  successful answer flips the state to `Connected` and the pushed event clears
  the banner. `get_daemon_status` is the one command allowed to autostart the
  daemon on first use, which is what the TUI and MCP do too.
- **Failure streak.** The first four unreachable probes read as `Reconnecting`;
  the fifth settles on `Disconnected`, so a brief restart does not flash a hard
  failure.
- **Data while disconnected.** Loaded data stays on screen; only the banner
  changes. Fetches are gated on the connection so a disconnected GUI does not
  hammer the socket.

### Disconnection behaviour

When the connection is not `Connected`, the services screen:

- shows a `role="status"` notice that the daemon is disconnected;
- **disables every mutating action** (start/stop/restart, single and bulk) and
  the detail actions;
- keeps the last loaded rows on screen so the user can still read them;
- issues no mutating request, and — crucially — **does not replay** an
  operation whose outcome was uncertain when the connection dropped. The
  user re-issues it after the connection returns.

When the connection returns, the connection event re-enables actions and the
gated fetches refetch, so state is refreshed from the daemon rather than from
the frontend's memory.

### Which executable gets started

The GUI's own executable is a windowed application, so it must never be
re-executed as a daemon. Before the first connection the adapter resolves the
real `service-orchestrator` binary in this order:

1. `SERVICE_ORCHESTRATOR_DAEMON_BIN` (an explicit override);
2. `PATH`;
3. a sibling of the running program;
4. `target/debug` or `target/release` walking up from the running program,
   which covers the `npm run tauri dev` layout.

The resolved path is exported through `SERVICE_ORCHESTRATOR_DAEMON_BIN` for the
shared autostart logic in `crates/client/src/socket.rs`. If no binary can be
found, the adapter reports one actionable error naming every remedy instead of
spawning something wrong. When a daemon is already running and no binary was
found, the adapter attaches to it without ever enabling autostart.

The adapter also keeps exactly one client for its lifetime, with a single
connect lock, so a burst of commands cannot start several daemons.

## 6. Security

| Control | Setting |
| --- | --- |
| Capabilities | `core:default` only — no shell, filesystem, HTTP, or process plugin |
| CSP (production) | `default-src 'self'`, no remote code, `connect-src 'self' ipc:` |
| CSP (dev) | adds `http://localhost:5173` and the HMR WebSocket only |
| Navigation | `tauri:` scheme always; `http://localhost:5173` only in debug builds |
| Secrets | never returned by a command; the adapter exposes only the shapes above |

The webview cannot ask the operating system for anything: its whole surface is
the commands and the two events. Input is validated at the Rust boundary, and
**every mutating command passes through the daemon's policy engine** exactly as
the TUI and MCP do — the GUI is classified as a `Cli` client and lifecycle
operations are `Permission::SafeWrite`. A confirmation dialog is a
user-interface affordance only; it never bypasses or weakens the daemon's
authorization policy (`tasks/019.md` §27, §35). Destructive operations
(`git reset`, `git clean`, force stop) are not exposed by this interface at all.

## 7. Frontend architecture

- **Typed API, no `any`.** `src/services/desktopApi.ts` is the only module that
  calls `invoke`; every shape in `src/types/models.ts` mirrors the Rust
  serialization exactly, including snake_case field names and enum variants.
- **Contexts, not a store library.** `ConnectionContext` owns the connection
  snapshot and the reconnect action; `WorkspaceContext` owns the workspace list
  and the active selection; `TaskEventsContext` owns the bounded task/workflow
  maps and the `terminalRevision` counter. There is no Redux and no duplicated
  daemon state.
- **One fetch per transition.** Data providers enable their fetches only while
  the connection is `connected`; they also refetch when `terminalRevision`
  changes, so a settled task produces exactly one reload. Last successful data
  is kept while refetching, so the screen does not blank.
- **`useAsync`.** A small hook provides `idle/loading/error/data` with a
  stale-response guard, a reload nonce, and loader-in-ref semantics. The
  stale-response guard is what keeps a slow answer for workspace A from
  overwriting the state of workspace B. The app deliberately does not run
  React's StrictMode in development because its double-invoked effects would
  duplicate IPC calls.
- **Pure logic is separate from components.** `utils/serviceQuery.ts`
  (filter/sort/count), `utils/serviceSelection.ts` (set operations on ids), and
  `utils/taskProgress.ts` (busy ids, last task/operation per service) are pure
  functions with their own tests; components only render and dispatch.
- **Presentation-only components.** Table, badges, cards, dialogs, and state
  views receive data and callbacks; they contain no business rules.
- **Themes.** Light and dark are CSS custom properties switched by
  `prefers-color-scheme`; the TUI-style palette keeps status colours consistent
  between themes. State is never communicated by colour alone — every badge
  carries a text label.

## 8. Dashboard

The dashboard renders **real daemon data only**:

- **Active workspace** (name and root);
- **Total / Running / Starting / Stopped / Failed / Unhealthy** service counts,
  derived from the active workspace's service statuses by `countServices`;
  `stopped` deliberately excludes `failed`, and health is counted separately
  from runtime state, so `RUNNING` is never assumed healthy;
- **Active tasks** from `get_daemon_status`;
- a **Daemon** panel (pid, clients, active tasks, managed services, protocol,
  start time, socket path);
- a **Task activity** feed built from forwarded task events.

Counters are never hard-coded, and they refresh when a task settles via
`terminalRevision`.

## 9. Service management

### The table

One row per service of the active workspace, with a selection checkbox, name,
type, workspace, runtime state, health state, Git branch, port, and the last
operation observed for that service. Runtime state uses the full domain set —
`NotRunning`, `Starting`, `Running`, `Stopping`, `Stopped`, `Failed`, `Unknown`
— and is never collapsed into "stopped". Health (`unknown`, `checking`,
`healthy`, `unhealthy`, `degraded`, `unreachable`) is shown separately. A row
with an in-flight command or a non-terminal task is marked busy.

### Search, filter, sort

Free-text search over name, id, and branch, plus filters by type, runtime state,
and health state, and column sorting. All of it runs client-side over the
already-fetched rows, so typing never triggers a backend request
(`tasks/019.md` §9). The selection survives filtering, sorting, and opening a
detail panel, because it is a set of stable ids, not of visible positions.

### Selection

- single and multi-selection via the checkbox or the keyboard;
- **select all visible** (only the rows matching the current filters), and
  **clear**;
- selection by service type and by runtime state;
- selected, filtered, and all-workspace services are counted and labelled
  distinctly;
- **bulk actions operate only on the explicit selection**, in a deterministic
  order, never on the whole workspace (`tasks/019.md` §8, §18).

### Actions

Start, stop, and restart are available per row (through the detail panel) and
for the selection through the bulk toolbar. Every action submits through the
typed API, shows the returned task/workflow id, and then relies on the daemon's
authoritative state and the realtime stream. Pending state plus the daemon's own
operation coordination prevents a triple-click from starting three processes
(`tasks/019.md` §28): while a command is in flight for a service, its actions
are disabled, and the daemon additionally refuses a second start for a service
that is already starting or running.

### Detail panel

`get_service_detail` backs a panel that shows the service id, type, workspace,
path, repository, runtime requirements, branch, configured ports, build system,
managed-process status, dependencies, health checks with their observed
outcomes, last operation, last-checked time, last failure, and recent errors.
Values the adapter cannot yet know — notably the current commit, which needs a
Git operation deferred to a later task — are labelled "Not inspected yet" rather
than invented. No secret is rendered. Context-sensitive actions
(Start/Stop/Restart/Check Health/View Logs/View Tasks) live on the panel and use
the same desktop API.

## 10. Bulk operations

Selecting more than one service enables the bulk toolbar. Start, stop, and
restart then submit **one workflow** to the daemon instead of a frontend loop;
dependency ordering, concurrency limits, runtime validation, and duplicate
protection are all enforced by the daemon (`tasks/019.md` §14–§16).

Before a bulk operation runs, a **confirmation dialog** shows the operation, the
workspace, the number of affected services, and the explicit list of services,
and states that the daemon — not the dialog — enforces dependencies,
concurrency, and authorization.

A **progress dialog** then follows the workflow's real task events, showing each
service's status (`Waiting for the daemon` before the first event, then each
task's own state) and per-service results, including partial failures. Progress
is derived from real `Task` events grouped by the workflow id; nothing is
fabricated (`tasks/019.md` §25).

## 11. Error handling and confirmations

- **Confirmations.** Single `start` and `check health` run directly; `stop`,
  `restart`, and *any* operation with more than one target require confirmation.
  Cancel closes the dialog and calls nothing.
- **Structured errors.** A failed action opens an `ErrorDialog` that shows the
  action, the target services, and the daemon's `{ code, message }` — for
  example `RUNTIME_PORT_ALREADY_IN_USE: port 8081 is already in use` — without
  stack traces or secrets (`tasks/019.md` §26).
- **No optimistic success.** The outcome recorded is the daemon's; the row's
  state only changes when the daemon reports it.

## 12. Keyboard shortcuts

| Key | Context | Effect |
| --- | --- | --- |
| `Tab` / `Shift+Tab` | everywhere | move focus (visible focus ring) |
| `Space` | a focused service row | toggle that row's selection |
| `Enter` | a focused service row | open the service detail panel |
| `Space` / `Enter` | a focused button | activate it |
| `Esc` | any dialog | close it (cancel) |

Checkboxes and buttons are native, labelled controls, so screen readers announce
selection, busy, and state as text.

## 13. Development

```bash
cd apps/desktop
npm install
npm run tauri dev      # preflight: cargo build --bin service-orchestrator, then vite + tauri
```

`npm run tauri dev` runs `scripts/preflight.mjs` first, which builds the root
`service-orchestrator` binary — the daemon autostart prerequisite. Tauri then
starts the Vite dev server (`http://localhost:5173`, strict port) and the native
window.

Useful environment variables:

- `SERVICE_ORCHESTRATOR_CONFIG` — the configuration file to use (defaults to
  `~/.service-orchestrator/config.toml`);
- `SERVICE_ORCHESTRATOR_SOCKET` — overrides the socket path derived from the
  configuration directory;
- `SERVICE_ORCHESTRATOR_DAEMON_BIN` — overrides the daemon executable.

Rule of thumb for manual testing: point the GUI at a copy of a configuration in
a scratch directory so it cannot disturb a developer's real daemon.

## 14. Testing and validation

Backend (`apps/desktop/src-tauri`):

- unit tests cover the connection state machine, error mapping, view models,
  identifier validation, command mapping, mutation mapping, event payloads, and
  daemon-binary resolution;
- `tests/daemon_ipc.rs` starts a **real daemon** on a private socket with fake
  lifecycle executors over a shared in-memory runtime-state store (no real
  processes) and drives the adapter's query and mutation functions through the
  real `RemoteClient`. It asserts the exact wire shape the frontend consumes,
  the structured error codes, single and bulk lifecycle behaviour, partial
  failures, and that several clients share one authoritative daemon.

Frontend (`apps/desktop`):

- Vitest with jsdom and Testing Library; the Tauri API is mocked through a typed
  fake in `src/test/mocks.ts`;
- tests cover the shell and navigation, workspace switching, the connection
  banner and Retry, dashboard counts, the services table (search, filter, sort,
  selection), single and bulk lifecycle actions, confirmations, the error
  dialog, the detail panel, workflow progress from task events, realtime
  refresh, disconnection, and the loading/error/empty states.

Commands used for validation:

```bash
# repository root
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings

# adapter (its own workspace)
cd apps/desktop/src-tauri
cargo fmt --check && cargo check --all-targets && cargo test
cargo clippy --all-targets --all-features -- -D warnings

# frontend
cd apps/desktop
npm run build && npm run typecheck && npm run test
```

## 15. Limitations

- **Realtime refresh is task-driven.** The daemon's forwarded `ServerEvent`
  stream carries task, log, and shutdown events — not service runtime or health
  transitions — so the GUI refreshes service state when a task settles (or after
  any command) rather than on a dedicated service event. Runtime/health changes
  caused by another interface appear after the next task settles or refetch.
- **Commit is not shown.** Displaying the current commit needs a Git operation
  deferred to a later task; the panel labels it "Not inspected yet".
- **No dedicated client kind.** The GUI presents itself as a `Cli` client
  (`tasks/018.md` §6); a dedicated interactive kind remains deferred, and no
  destructive operation is exposed.
- **Out of scope by design.** No Git/build/library/migration screens, no full
  log viewer, no service editing, no Docker/Kubernetes, no DMG, signing,
  notarization, or CI/CD (`tasks/019.md` §42).
- The active workspace selection lives in memory only; it is not persisted
  across restarts.
- The development icon (`src-tauri/icons/icon.png`) is an obvious placeholder;
  real artwork is a packaging concern for a later task.

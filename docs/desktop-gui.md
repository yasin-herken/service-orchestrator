# Desktop GUI (Tauri 2)

The desktop GUI is the graphical interface to Service Orchestrator
(`tasks/018.md`). It is a Tauri 2 application — a React + TypeScript frontend
(`apps/desktop`) inside a native macOS window, with a small Rust adapter crate
(`apps/desktop/src-tauri`) behind it.

It is an **interface**, exactly like the Ratatui TUI and the MCP server:

```text
React webview  ──invoke──▶  Tauri commands  ──▶  OrchestratorClient  ──▶  the one daemon
React webview  ◀──emit────  connection / task events ◀── tracker / subscription
```

The GUI never parses configuration, schedules tasks, or runs a process. It
holds no second task engine, no second process manager, and no competing state:
every read goes through the shared client boundary to the single local daemon
(`docs/local-daemon.md`), so a task started by an agent in MCP or by a human in
the TUI is the same task the GUI displays.

## 1. Scope of this foundation

This task delivers the shell and the read path only:

| Delivered | Not yet delivered (later tasks) |
| --- | --- |
| Application shell: sidebar, header, workspace selector | Starting, stopping, or restarting services |
| Dashboard: workspace and daemon summary, task activity feed | Git, build, library, and migration screens |
| Services list: state, health, branch, and port per service | Log viewing, health-check runs |
| Connection status with disconnect banner and Retry | Service or workspace editing |
| Loading, error, empty, and disconnected states | DMG packaging, code signing, notarization, CI/CD |
| Light and dark themes; macOS window behaviour | Docker and Kubernetes targets |

Every mutating capability is deliberately absent: **no Tauri command in this
task starts, stops, or changes anything.** The adapter is read-only.

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
│   ├── hooks/useAsync.ts      # loading/error/data with a stale-response guard
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
        ├── main.rs, lib.rs    # event loop, window, navigation filter
        ├── commands.rs        # the six read-only Tauri commands
        ├── queries.rs         # command logic over OrchestratorClient
        ├── dto.rs             # wire view models
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

## 3. The six commands

Every command is a thin wrapper: validate input, delegate to the shared client,
map the answer into a view model.

| Command | Backing use case | Returns |
| --- | --- | --- |
| `get_daemon_status` | `RemoteClient::daemon_status` | pid, endpoint, protocol, clients, tasks, managed services |
| `list_workspaces` | `list_workspaces` | workspace summaries (id, name, root, service count) |
| `list_services` | `get_workspace_status` | one row per service in a workspace |
| `get_workspace_status` | `get_workspace_status` | workspace plus all service rows |
| `get_service_status` | `get_service_status` | one service row, checked against its workspace |
| `get_connection_state` | adapter tracker | the current `ConnectionSnapshot` |

Command arguments are camelCase on the JavaScript side (`workspaceId`,
`serviceId`) and snake_case in Rust, which is Tauri's default mapping.

`list_services` reuses the `get_workspace_status` round trip rather than
issuing a second IPC method: the daemon returns everything the table renders in
one response. `get_service_status` re-validates that the service belongs to the
requested workspace, so a stale selection cannot silently show a service from
another workspace.

### Errors

A failing command rejects with `{ code, message }`:

- `DAEMON_UNAVAILABLE` — the daemon is unreachable (also drives the banner);
- `PROTOCOL_INCOMPATIBLE` — the daemon speaks another protocol version;
- `INVALID_INPUT` — the identifier failed domain validation;
- `NOT_FOUND` — the service belongs to a different workspace;
- the application error's variant name (for example `WorkspaceNotFound`,
  `ServiceNotFound`);
- `INTERNAL` — an adapter-side failure.

Messages are short and user-safe: no stack traces, no secrets, no raw IPC
frames. Connection-shaped failures carry the daemon log path as a hint.

## 4. Events

The adapter pushes two event streams; the frontend subscribes once and keeps
its state from them.

| Event | Payload | Meaning |
| --- | --- | --- |
| `daemon://connection` | `ConnectionSnapshot` | a connection state transition |
| `daemon://task` | `TaskEvent` | a forwarded task-engine event |

`daemon://task` keeps the daemon's externally tagged shape
(`{"TaskCreated": { … }}`). If the local subscription falls behind, the adapter
emits a synthetic `{"Lagged": {"dropped": N}}` instead of silently losing
events, and the activity feed shows that events were dropped.

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
- **No idle resurrection.** Probing is guarded by a socket-reachability check:
  while the socket is unreachable the monitor records failures and issues no
  call, so a daemon the user stopped stays stopped. A probe that was already in
  flight when the daemon stopped can still trigger the shared client's single
  reconnect-retry (`connect_or_start`), which starts the daemon again — exactly
  the behaviour the TUI and MCP have while they are attached. From a fully
  disconnected state, only a command or the Retry button starts a daemon.
- **Retry.** The banner's Retry button asks the adapter for the daemon status;
  a successful answer flips the state to `Connected` and the pushed event
  clears the banner. `get_daemon_status` is the one command that is allowed to
  autostart the daemon on first use, which is what the TUI and MCP do too.
- **Failure streak.** The first four unreachable probes read as
  `Reconnecting`; the fifth settles on `Disconnected`, so a brief restart does
  not flash a hard failure.
- **Data while disconnected.** Loaded data stays on screen; only the banner
  changes. Fetches are gated on the connection so a disconnected GUI does not
  hammer the socket.

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
| Secrets | never returned by a command; commands expose only the shapes above |

The webview cannot ask the operating system for anything: its whole surface is
the six commands and two events. Destructive operations do not exist yet; when
they arrive (task 019) they must route through the same policy engine as the
TUI and MCP, with an explicit confirmation in the interface.

## 7. Frontend architecture

- **Typed API, no `any`.** `src/services/desktopApi.ts` is the only module that
  calls `invoke`; every shape in `src/types/models.ts` mirrors the Rust
  serialization exactly, including snake_case field names and enum variants.
- **Contexts, not a store library.** `ConnectionContext` owns the connection
  snapshot and the reconnect action; `WorkspaceContext` owns the workspace list
  and the active selection; `TaskEventsContext` owns a bounded activity feed.
  There is no Redux and no duplicated daemon state.
- **One fetch per transition.** Data providers enable their fetches only while
  the connection is `connected` and keep the last successful data afterwards,
  so connecting fetches once and a dropped connection does not clear the
  screen.
- **`useAsync`.** A small hook provides `idle/loading/error/data` with a
  stale-response guard, a reload nonce, and loader-in-ref semantics. The app
  deliberately does not run React's StrictMode in development because its
  double-invoked effects would duplicate IPC calls.
- **Presentation-only components.** Table, badges, cards, and state views
  receive data and callbacks; they contain no business rules.
- **Themes.** Light and dark are CSS custom properties switched by
  `prefers-color-scheme`; the TUI-style palette keeps status colours consistent
  between themes.

## 8. Development

```bash
cd apps/desktop
npm install
npm run tauri dev      # preflight: cargo build --bin service-orchestrator, then vite + tauri
```

`npm run tauri dev` runs `scripts/preflight.mjs` first, which builds the root
`service-orchestrator` binary — the daemon autostart prerequisite. Tauri then
starts the Vite dev server (`http://localhost:5173`, strict port) and the
native window.

Useful environment variables:

- `SERVICE_ORCHESTRATOR_CONFIG` — the configuration file to use (defaults to
  `~/.service-orchestrator/config.toml`);
- `SERVICE_ORCHESTRATOR_SOCKET` — overrides the socket path derived from the
  configuration directory;
- `SERVICE_ORCHESTRATOR_DAEMON_BIN` — overrides the daemon executable.

Rule of thumb for manual testing: point the GUI at a copy of a configuration in
a scratch directory so it cannot disturb a developer's real daemon.

## 9. Testing and validation

Backend (`apps/desktop/src-tauri`):

- unit tests cover the connection state machine, error mapping, view models,
  identifier validation, event payloads, and daemon-binary resolution;
- `tests/daemon_ipc.rs` starts a **real daemon** on a private socket with no
  executors and drives the adapter's query functions through the real
  `RemoteClient`, asserting the exact wire shape the frontend consumes, the
  structured error codes, and that several clients share one authoritative
  daemon.

Frontend (`apps/desktop`):

- Vitest with jsdom and Testing Library; the Tauri API is mocked through a
  typed fake in `src/test/mocks.ts`;
- tests cover the shell and navigation, workspace switching, the connection
  banner and Retry, dashboard counts, the services table (search, filter, sort,
  keyboard selection), and the loading/error/empty/disconnected states.

Commands used for validation:

```bash
# repository root
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings

# adapter
cd apps/desktop/src-tauri
cargo fmt --check && cargo check --all-targets && cargo test
cargo clippy --all-targets --all-features -- -D warnings

# frontend
cd apps/desktop
npm run build && npm run typecheck && npm run test
```

## 10. Limitations

- Read-only: nothing in this task starts, stops, or edits anything.
- No DMG, signing, or notarization; `bundle.active` is `false`.
- The GUI presents itself to the daemon as a `Cli` client, because it performs
  only read operations. Task 019 will add a dedicated interactive client kind
  and the destructive-confirmation flow.
- The active workspace selection lives in memory only; it is not persisted
  across restarts.
- The development icon (`src-tauri/icons/icon.png`) is an obvious placeholder;
  real artwork is a packaging concern for a later task.

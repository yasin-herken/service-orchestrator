# Desktop GUI (Tauri 2)

The desktop GUI is the graphical interface to Service Orchestrator
(`tasks/018.md`, `tasks/019.md`, `tasks/020.md`). It is a Tauri 2 application — a
React + TypeScript frontend (`apps/desktop`) inside a native macOS window, with a
small Rust adapter crate (`apps/desktop/src-tauri`) behind it.

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

This document covers the shell, the dashboard, **service lifecycle
management** (`tasks/019.md`), and **Git, build, and library management**
(`tasks/020.md`): listing and selecting services, repositories, build targets,
and libraries; start/stop/restart; Git fetch/pull/checkout/sync (single and bulk)
with a read-only preview; single and bulk builds; library build/install/sync and
preparation; dependency version alignment; task progress; structured logs;
cancellation; realtime refresh; and confirmation/error handling.

| Delivered | Not yet delivered (later tasks) |
| --- | --- |
| Application shell: sidebar, header, workspace selector | Migration management GUI |
| Dashboard: real workspace/daemon counters, task activity feed | Full health and logs dashboards |
| Services table: state, health, branch, port, last operation | Service or workspace creation/editing |
| Multi-selection with stable ids, select-all-visible, by type/state | Docker and Kubernetes targets |
| Start / stop / restart, single and bulk | DMG packaging, code signing, notarization, CI/CD |
| Service detail panel with configuration, observed state, diagnostics | Destructive Git operations (`git reset`, `git clean`, force stop) |
| Git screen: repositories, selection, fetch/pull/checkout/sync, bulk preview | Automatic Maven POM rewriting or version changes |
| Build screen: provider-agnostic targets and capabilities, single/bulk builds | New build providers |
| Library screen: artifacts, dependents, verification, version alignment, prepare | A dedicated interactive client kind (see §8) |
| Shared task monitoring: progress dialog, cancellation, bounded logs | |
| Realtime task-driven refresh; confirmation, error, and progress dialogs | |
| Connection status with disconnect banner, Retry, and safe reconnect | |

No business logic lives in the GUI. Starting a service runs the daemon's
`StartService` use case, a bulk sync runs the daemon's `SyncRepositories`
workflow, and a library build runs the daemon's `PrepareLibrary` use case; the
GUI only submits a request and then displays the authoritative result.

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
│   │   ├── useAsync.ts             # loading/error/data with a stale-response guard
│   │   ├── useServiceActions.ts    # lifecycle submissions, pending, operations
│   │   ├── useEntityOperations.ts  # shared submit/pending/error for Git/Build/Library
│   │   ├── useGitOperations.ts     # Git action → single task or bulk workflow
│   │   ├── useBuildOperations.ts   # build action → single task or bulk workflow
│   │   └── useLibraryOperations.ts # library build/install/sync/prepare
│   ├── utils/                 # pure logic: service/repository query, selection, taskProgress
│   ├── layouts/AppShell.tsx   # sidebar + header + content routing
│   ├── components/            # presentation only
│   │   ├── tasks/             # shared task monitoring (progress, summary, logs, errors)
│   │   ├── git/               # RepositoryTable, RepositoryDetailPanel, GitSyncDialog
│   │   ├── build/             # BuildTargetTable, BuildTargetDetailPanel, BuildCapabilitiesList
│   │   └── library/           # LibraryTable, LibraryDetailPanel, VersionAlignmentPanel, PrepareLibraryDialog
│   ├── pages/                 # Dashboard, Services, Git, Builds, Libraries, placeholders
│   └── styles/global.css      # light + dark via prefers-color-scheme
└── src-tauri/
    ├── Cargo.toml             # its own workspace; excluded from the root one
    ├── tauri.conf.json        # identifier, CSP, dev/build commands
    ├── capabilities/default.json
    ├── icons/icon.png         # development placeholder icon
    └── src/
        ├── main.rs, lib.rs    # event loop, window, navigation filter, command registration
        ├── commands.rs        # the Tauri commands (read + mutation)
        ├── queries.rs         # read logic over OrchestratorClient
        ├── mutations.rs       # lifecycle logic over OrchestratorClient
        ├── git.rs             # Git read/mutation use cases
        ├── build.rs           # build read/mutation use cases (provider-agnostic)
        ├── library.rs         # library read/mutation use cases
        ├── tasks.rs           # task/workflow/log reads and cancellation
        ├── validation.rs      # domain-typed identifier parsing at the boundary
        ├── dto.rs             # wire view models incl. Git/Build/Library/Task views
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
`serviceId`, `serviceIds`, `libraryId`, `taskId`, `workflowId`) and snake_case
in Rust, which is Tauri's default mapping.

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

### Git

| Command | Backing use case | Returns |
| --- | --- | --- |
| `list_repositories` | `list_services` + `get_workspace_status` | configuration-derived repository rows (no Git operation) |
| `get_git_status` | `get_git_status` | one repository's working-tree status |
| `inspect_repositories` | `get_git_status` per id | per-repository inspection, each success or failure independent |
| `list_branches` | `list_branches` | local and remote-tracking branches |
| `preview_git_sync` | `get_git_status` + `list_branches` per id | a read-only bulk preview with per-repository warnings |
| `fetch_repository` / `pull_repository` / `checkout_repository` / `sync_repository` | one Git use case each | an `OperationOutcome` (one task) |
| `fetch_repositories` / `sync_repositories` / `checkout_repositories` | `fetch_repositories` / `sync_repositories` / `checkout_repositories` | an `OperationOutcome` (one workflow) |

The repository *list* is derived from configuration in a single round trip
(`list_services` plus the in-memory `get_workspace_status`), so opening the Git
screen performs **no** Git operation and stays cheap with 100+ repositories
(§20). Real working-tree state and branches are inspected only on demand. A bulk
mutation maps onto an existing coordinated core operation — the GUI never loops
one request per repository. Git **pull is single-repository only** because the
core exposes no bulk pull; bulk synchronization is expressed as fetch, checkout,
and fast-forward pull, not as a frontend loop.

### Builds

| Command | Backing use case | Returns |
| --- | --- | --- |
| `list_build_targets` | `list_services` + `get_build_capabilities` (deduped by build system) | build targets with provider capabilities |
| `get_build_capabilities` | `get_build_capabilities` | one service's build-system operations |
| `build_service` | `build_service` | an `OperationOutcome` (one task) |
| `build_services` | `prepare_environment` with only the build phase enabled | an `OperationOutcome` (one workflow) |

`list_build_targets` resolves each distinct build system **once** through
`get_build_capabilities` and attaches `capability` (or a structured
`capability_error`) to every target, so a workspace with many Maven services
issues one capability query for Maven, not one per service. Bulk builds submit
one `prepare_environment` workflow restricted to the explicit selection with
`sync`, `include_dependencies`, `build_libraries`, `run_liquibase`, and `start`
all disabled and `build` enabled, so dependency ordering and concurrency stay in
the daemon.

### Libraries

| Command | Backing use case | Returns |
| --- | --- | --- |
| `list_libraries` | `list_libraries` + best-effort `get_library_status` + `find_library_dependents` | library rows with observed state and dependents |
| `get_library_dependents` | `find_library_dependents` | the services that depend on one library |
| `verify_library_artifact` | `verify_library_artifact` | artifact presence and path in the effective local repository |
| `inspect_version_alignment` | `inspect_library_version_alignment` | per-consumer version alignment |
| `build_library` / `install_library` / `sync_library` | one library use case each | an `OperationOutcome` (one task) |
| `prepare_library` | `prepare_library` | an `OperationOutcome` (one workflow) |
| `prepare_libraries` | `prepare_selected_libraries` | an `OperationOutcome` (one dependency-ordered workflow) |

`list_libraries` reads each library's repository status **best-effort**, so a
library whose checkout cannot be inspected still appears with a structured
`repository_error` rather than failing the whole list. Version alignment reports
each consumer as aligned, mismatched, unknown, or unresolved and never rewrites
a POM or changes a version (§14).

### Tasks, workflows, and logs

| Command | Backing use case | Returns |
| --- | --- | --- |
| `get_task` | `get_task` | one task with its progress and failure |
| `summarize_workflow` | `summarize_workflow` | a workflow's aggregated per-state counts |
| `get_task_logs` | `get_task_logs` over a bounded `LogRequest` | one bounded, filterable page of structured logs |
| `cancel_task` | `cancel_task` then `get_task` | the task's latest observed state |
| `cancel_workflow` | `cancel_workflow` | per-task cancellation outcome |

`get_task_logs` always carries a limit and an optional `after` cursor, clamps the
limit to a hard maximum, and supports a severity floor and a text filter, so the
viewer follows a task with stable pagination instead of unbounded memory
(§15, §20).

### Errors

A failing command rejects with `{ code, message }`:

- `DAEMON_UNAVAILABLE` — the daemon is unreachable (also drives the banner);
- `PROTOCOL_INCOMPATIBLE` — the daemon speaks another protocol version;
- `INVALID_INPUT` — an identifier (or an empty bulk selection, or a sync mode or
  branch) failed domain validation;
- `NOT_FOUND` — the entity belongs to a different workspace;
- the application error's variant name (for example `WorkspaceNotFound`,
  `ServiceNotFound`, `LibraryNotFound`), including a **permission denial** from
  the policy engine and structured Git/build/library codes such as
  `BUILD_UNSUPPORTED_OPERATION`;
- `INTERNAL` — an adapter-side failure.

Messages are short and user-safe: no stack traces, no secrets, no raw IPC
frames. Connection-shaped failures carry the daemon log path as a hint.

## 4. Application core integration

Every command maps onto an existing application use case over the shared
`client` boundary. This task (020) added **one** core query and no new
scheduler or workflow logic:

- **`get_build_capabilities`** — a read-only `Application` query backed by a new
  `BuildCapabilityIndex` port (`crates/application/src/ports/build.rs`). The
  build adapter's `BuildProviderRegistry` implements the port, so the GUI learns
  which operations a build system supports from the same registry the executor
  uses. Without an adapter the `NullBuildCapabilityIndex` reports "no provider is
  registered" instead of fabricating capabilities.
- **No duplicated logic.** Bulk Git synchronization, bulk builds, and library
  preparation are the core's existing `sync_repositories` / `fetch_repositories`
  / `checkout_repositories`, `prepare_environment`, and
  `prepare_selected_libraries` operations. The GUI submits one request; it never
  re-implements ordering, concurrency, or safety.
- **The adapter's `block_on` bridge** only crosses the sync/async boundary for
  the daemon-backed client's already-ready futures; it runs on the command's
  blocking pool and never blocks on real I/O.

## 5. Events and realtime updates

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
reload of service state. The Git, Builds, and Libraries screens use it the same
way: after an operation's task settles they reload the authoritative list, and
the Git screen additionally re-inspects the exact repositories it touched. The
progress dialog derives per-entity status from those same events; a workflow's
tasks are grouped by the `workflow` id the daemon attaches to each `Task`.

**A task-id or workflow-id is always real.** The GUI never fabricates progress
percentages and never marks an operation successful on its own
(`tasks/019.md` §24, §25; `tasks/020.md` §10, §17, §27).

## 6. Connection model

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

When the connection is not `Connected`, every screen:

- shows a `role="status"` notice that the daemon is disconnected;
- **disables every mutating action** — start/stop/restart, Git fetch/pull/
  checkout/sync, builds, library build/install/sync/prepare, and the detail
  actions;
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

## 7. Security

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
the TUI and MCP do — the GUI is classified as a `Cli` client and lifecycle,
Git-sync, build, and library-prepare operations are `Permission::SafeWrite`. A
confirmation dialog is a user-interface affordance only; the daemon authorizes every request itself and
the GUI never treats a confirmation as authorization (`tasks/019.md` §27, §35;
`tasks/020.md` §31). Destructive operations (`git reset`, `git clean`, force
stop) are not exposed by this interface at all.

## 8. Frontend architecture

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
- **One shared operation hook.** `useEntityOperations` owns submit/pending/
  error for the Git, Builds, and Libraries screens; `useGitOperations`,
  `useBuildOperations`, and `useLibraryOperations` only choose which typed API
  call a given action maps to. No screen loops over a selection itself.
- **Pure logic is separate from components.** `utils/serviceQuery.ts`,
  `utils/repositoryQuery.ts` (filter/sort/distinct over repository rows),
  `utils/serviceSelection.ts` (set operations on ids), and
  `utils/taskProgress.ts` (busy ids, last task/operation per service) are pure
  functions with their own tests; components only render and dispatch.
- **Presentation-only components.** Table, badges, cards, dialogs, capability
  lists, version-alignment panels, and state views receive data and callbacks;
  they contain no business rules.
- **Themes.** Light and dark are CSS custom properties switched by
  `prefers-color-scheme`; the TUI-style palette keeps status colours consistent
  between themes. State is never communicated by colour alone — every badge
  carries a text label.

## 9. Dashboard

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

## 10. Service management

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
(`tasks/019.md` §28).

### Detail panel

`get_service_detail` backs a panel that shows the service id, type, workspace,
path, repository, runtime requirements, branch, configured ports, build system,
managed-process status, dependencies, health checks with their observed
outcomes, last operation, last-checked time, last failure, and recent errors.
Values the adapter cannot yet know — notably the current commit, which the
repository detail panel now inspects on demand — are labelled "Not inspected
yet" rather than invented. No secret is rendered.

## 11. Git management (`tasks/020.md` §4–§11)

### The repository list

`list_repositories` derives one row per configured repository from
configuration, in a single round trip, with **no Git operation** — so the screen
opens cheaply even with 100+ repositories (§20). Each row carries a **stable,
workspace-aware id** (`workspace:service`) that matches the `repository_id` the
inspection returns, so selection and per-repository results always line up. The
table shows the repository name and id, its path, the last observed branch and
Git state, and the last operation observed for that service.

### Selection

Single and multi-selection, **select all visible** (only the rows matching the
current filters), and **clear**, all operating on the stable ids. Selection
survives filtering, sorting, and opening the detail panel, is pruned when the
list changes, and is cleared when the active workspace changes, so no selection
or dialog ever bleeds across workspaces (§22).

### Search, filter, sort

Free-text search over name, id, path, and branch, plus filters by **type**
(the hosting provider), **branch**, and **Git state**, and column sorting by
name, branch, state, or path with a deterministic stable-id tie-break. All of it
runs client-side over the already-fetched rows.

### Operations

| Action | Single | Bulk |
| --- | --- | --- |
| Fetch | `fetch_repository` | `fetch_repositories` (one workflow) |
| Checkout | `checkout_repository` | `checkout_repositories` (one workflow) |
| Sync (fetch + checkout + fast-forward pull) | `sync_repository` | `sync_repositories` (one workflow) |
| Pull (fast-forward) | `pull_repository` | — (single only) |
| Inspect status | `get_git_status` / `inspect_repositories` | `inspect_repositories` |
| List branches | `list_branches` | — |

A single repository maps to the corresponding single-repository task; a
multi-repository selection maps to the coordinated bulk workflow, so the GUI
never issues an uncoordinated loop (§6, §7).

### Bulk synchronization: preview then execute

`Sync selected…` opens the bulk synchronization dialog, which states the
workspace, the count, and the explicit repository list. It offers a **target
branch** and a **mode** — *fetch + checkout + fast-forward pull*, *fetch only*,
or *checkout only* — and a **Preview** button. Preview is read-only: it
aggregates each repository's real working-tree status and branch list and shows,
per repository, the current branch, whether the target branch exists locally or
remotely, the operations the daemon will run, a `Ready` / `Warning` / `Blocked`
status, and the warnings. It also summarises the counts: ready, dirty, missing
branch, diverged, and unavailable (§8).

**Execute** is disabled until a preview exists and at least one repository is
not blocked; a preview where every repository is blocked cannot be executed.
Executing closes the dialog and submits the corresponding single bulk operation:
`fetch` → `fetch_repositories`, `checkout` → `checkout_repositories`, and
`sync` → `sync_repositories`. The daemon re-checks every precondition at
execution time; the preview is advisory readability, not the boundary.

### Safety

The Git screen **never silently discards local changes**: it never stashes,
resets, cleans, or force-checks-out anything (§9). A repository with uncommitted
changes is reported as dirty with an explicit warning; a missing target branch
blocks a sync or checkout with a warning; a repository that cannot be inspected
is reported as unavailable with the inspection error rather than assumed clean.
The GUI's behaviour mirrors the daemon's, which refuses an unsafe checkout or
pull instead of overwriting work. Destructive Git operations are out of scope
for this interface.

### Results and the detail panel

Opening a repository inspects **only that repository** (status and branches on
demand, §20). The panel shows the configured facts (id, service id, workspace,
path, provider, default branch, clone URL), the on-demand working-tree
inspection (lifecycle, branch, commit, upstream, ahead/behind, staged, unstaged,
untracked, conflicts, and a clean/diverged/detached/unborn summary), the real
last operation, the branch list, and a checkout form limited to local branches.
A dirty working tree carries an explicit "no stash, reset, clean, or force"
note. Context actions (inspect, list branches, fetch, pull, sync, checkout) use
the same typed API as the table.

After a submitted operation's task settles, the screen re-inspects exactly the
repositories it touched — bounded to the explicit selection — through the
`terminalRevision` counter, so results reflect the daemon's real outcome rather
than an optimistic guess.

## 12. Build management (`tasks/020.md` §12–§19)

### Provider independence

The Builds screen is **build-system agnostic**: it never names Maven or npm. It
renders exactly the operations the Application Core reports for each target's
configured build system (§13). `list_build_targets` resolves each distinct build
system once and attaches its `BuildCapabilities`, or a structured
`capability_error` when a system has no registered provider or a service has no
build system. The capability list shows each operation as **Available**, **Not
exposed** (supported by the provider but not offered for services by the core),
or **Unsupported**, each with the core's own explanation. Actions are disabled
for anything not exposed, so the UI cannot offer an operation the core will
reject.

### Operations

| Action | Single | Bulk |
| --- | --- | --- |
| Build | `build_service` (one task) | `build_services` (one `prepare_environment` workflow, build phase only) |

The bulk build submits one coordinated workflow restricted to the explicit
selection, so dependency ordering and concurrency stay in the daemon (§16). The
bulk action is disabled if any selected service exposes no build operation, and
the toolbar explains which services those are; it is also disabled while any
selected service is busy.

### Progress and logs

While an operation runs, the shared progress dialog follows the workflow's real
task events. The Builds screen also renders a **Build logs** region for the
selected target's last task, and embeds the same `LogViewer` inside the progress
dialog. `LogViewer` reads the existing Observability API through the adapter,
keeps a **bounded** buffer (2,000 entries), follows an active task with a stable
sequence cursor (polling every second), and filters by severity and text — it
never buffers unlimited output in React (§18, §20). stdout and stderr are
distinguished by text and style, never by colour alone.

### Results

Per-service results, including partial failures, come from real task events and
the workflow summary — the GUI never claims a build succeeded on its own (§19).
A failed build opens a structured error dialog and its diagnostic logs remain
viewable through the shared viewer.

## 13. Library management (`tasks/020.md` §20–§26)

### The library list

`list_libraries` returns each configured library with its artifact coordinate,
configured version, best-effort repository state (branch, commit, Git state, or
a structured `repository_error`), and its dependents. The table shows the
library name, branch, artifact, configured version, Git state, dependents, and
the last operation observed. Free-text search covers name, id, coordinate, and
configured version.

### Artifact details

Opening a library loads its **dependents** and **verifies the artifact's
presence** in the effective local repository on demand. The panel shows the
artifact coordinate, packaging, classifier, file name, the verification result
(present or not, with the resolved path), and the dependent services.

### Dependency version alignment

Version alignment is inspected **explicitly** (it reads every consumer's
dependency, §21). The panel reports a target coordinate and, per consumer, the
version the consumer resolves to and a status:

| Status | Meaning |
| --- | --- |
| `Aligned` | the consumer resolves to the library's target version |
| `Mismatch` | the consumer resolves to a different version |
| `Unknown` | the consumer's effective version could not be determined |
| `Unresolved` | the consumer or its dependency could not be resolved |

It summarises how many consumers are aligned and whether the target is fully
aligned. Alignment is **reported, never enforced**: the GUI does not rewrite
Maven POM files or change dependency versions (§23, §24).

### Operations

| Action | Single | Bulk |
| --- | --- | --- |
| Build | `build_library` (one task) | `prepare_libraries` with only `build` (one workflow) |
| Install | `install_library` (one task) | `prepare_libraries` with only `install` (one workflow) |
| Sync | `sync_library` (one task) | `prepare_libraries` with only `sync` (one workflow) |
| Prepare | `prepare_library` | `prepare_libraries` |

A single library maps to its dedicated build/install/sync task or the
prepare-library workflow; a multi-library selection maps onto **one** prepare
workflow with only the requested phases enabled, so the daemon owns dependency
ordering rather than the GUI looping (§21, §25, §26).

### Preparation and local development

`Prepare…` opens an options dialog: **Build the library**, **Install the
artifact locally**, **Verify the installed artifact** (all on by default), and
**Synchronize the repository first** (off by default, with an optional target
branch). Synchronization is optional so a developer can build the current local
branch — including uncommitted changes — without any checkout. No branch is
forced (never `master`), and no version is changed and no POM is rewritten
(§24, §25, §26). At least one phase must be selected to run.

## 14. Shared task monitoring (`tasks/020.md` §27–§29)

Git, build, and library operations all return task or workflow ids, and all
three screens use **one** task-monitoring implementation, not three:

- **`OperationProgressDialog`** — a modal that renders the shared
  `OperationSummary`: the operation, the target label and count, the real
  per-entity status (`Waiting for the daemon` before the first event, then each
  task's own state), a live workflow summary, and per-entity terminal results
  including partial failures. It accepts an optional `logs` node so the Builds
  screen can embed the same `LogViewer`; `TaskProgressDialog` delegates to it.
- **`TaskStatusIndicator` / `TaskViews`** derive status from real `Task`
  events grouped by the workflow id; nothing is fabricated.
- **Cancellation** is available from the dialog through `cancel_task` and
  `cancel_workflow`. Cancellation is best-effort and never rolls back work
  already done, so the GUI reports whatever the engine observed — including
  partial completion — rather than assuming a rollback (§29).

## 15. Error handling and confirmations

- **Confirmations.** Single `start` and `check health` run directly; `stop`,
  `restart`, and *any* operation with more than one target require confirmation.
  Cancel closes the dialog and calls nothing. A confirmation is a usability
  affordance: the daemon re-evaluates policy and never trusts the frontend's
  `confirmed` flag (§31).
- **Structured errors.** A failed action opens an `ErrorDialog` that shows the
  action, the target entities, and the daemon's `{ code, message }` — for
  example `BUILD_UNSUPPORTED_OPERATION: build system 'npm' does not support the
  'publish_local_artifact' operation` or `RUNTIME_PORT_ALREADY_IN_USE: port 8081
  is already in use` — without stack traces or secrets (§30).
- **Per-item results.** Bulk Git and library operations report each repository's
  or library's success or failure independently, so one failure does not hide the
  others; the Git inspection view keeps a structured error per repository.
- **No optimistic success.** The outcome recorded is the daemon's; a row's state
  only changes when the daemon reports it.

## 16. Keyboard shortcuts

| Key | Context | Effect |
| --- | --- | --- |
| `Tab` / `Shift+Tab` | everywhere | move focus (visible focus ring) |
| `Space` | a focused entity row | toggle that row's selection |
| `Enter` | a focused entity row | open the detail panel |
| `Space` / `Enter` | a focused button | activate it |
| `Esc` | any dialog | close it (cancel) |

Checkboxes and buttons are native, labelled controls, so screen readers announce
selection, busy, and state as text. Tables use `scope`-ed headers and captions
(visually hidden where a heading is redundant), status is never conveyed by
colour alone, and every toolbar count is announced through an `aria-live`
region.

## 17. Performance

- **Cheap lists.** Repository and build-target lists are configuration-derived;
  no Git operation or per-service provider query runs on open beyond one
  capability query per distinct build system. Real Git state is inspected only
  on demand.
- **Bounded logs.** `LogViewer` retains at most 2,000 entries and pages through
  the Observability API with a cursor; `get_task_logs` clamps its page limit.
- **Bounded event maps.** `TaskEventsContext` keeps at most 200 tasks and 50
  workflows.
- **One reload per settle.** `terminalRevision` produces exactly one authoritative
  reload per settled task, and the Git screen re-inspects only the repositories
  an operation touched.

This keeps the GUI responsive at the documented scale of 40–100+ repositories
and services.

## 18. Development

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

## 19. Testing and validation

Backend (`apps/desktop/src-tauri`):

- unit tests cover the connection state machine, error mapping, view models,
  identifier validation, command mapping, mutation mapping, event payloads,
  daemon-binary resolution, Git sync-mode parsing, the bulk-build request shape,
  library prepare options, and log-level parsing;
- `tests/daemon_ipc.rs` starts a **real daemon** on a private socket with fake
  executors over a shared in-memory runtime-state store and drives the adapter's
  query and mutation functions through the real `RemoteClient`. It asserts the
  exact wire shape the frontend consumes, structured error codes, single and
  bulk lifecycle and Git behaviour, provider capabilities, bulk-build and
  library partial failures, version-alignment mismatch, cancellation, Git/build/
  library input validation, and that several clients share one authoritative
  daemon. It uses **fixture workspaces and fakes only** — no real developer
  repository and no real process.

Frontend (`apps/desktop`):

- Vitest with jsdom and Testing Library; the Tauri API is mocked through a typed
  fake in `src/test/mocks.ts`;
- tests cover the shell and navigation, workspace switching, the connection
  banner and Retry, dashboard counts, the services table and lifecycle actions,
  the Git screen (list, filters, selection, preview, execute, safety, results),
  the Builds screen (capabilities, single/bulk build, disabled unsupported,
  logs), the Libraries screen (list, detail, dependents, verification,
  alignment, prepare), workflow progress from task events, realtime refresh,
  disconnection, cancellation, and the loading/error/empty states.

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

## 20. Limitations

- **Realtime refresh is task-driven.** The daemon's forwarded `ServerEvent`
  stream carries task, log, and shutdown events — not service runtime, health, or
  Git transitions — so the GUI refreshes state when a task settles (or after any
  command) rather than on a dedicated event. Changes made by another interface
  appear after the next task settles or refetch.
- **No destructive Git operations.** `git reset`, `git clean`, `discard local
  changes`, and force stop are not exposed; the daemon requires an interactive
  same-UID client kind for them, which is deferred.
- **No bulk Git pull.** The core exposes bulk fetch, checkout, and sync but no
  bulk pull; pull is single-repository only. Bulk synchronization is expressed
  as fetch + checkout + fast-forward pull.
- **Build capabilities are informational.** The Builds screen shows what the
  core reports and disables what it does not expose, but it does not offer
  clean/test/dependency-install as separate service actions (those remain
  internal phases).
- **Version alignment is read-only.** Misalignment is reported; there is no
  one-click upgrade, and no POM is ever rewritten.
- **No dedicated client kind.** The GUI presents itself as a `Cli` client
  (`tasks/018.md` §6); a dedicated interactive kind remains deferred.
- **Out of scope by design.** No migration management GUI, no full health or
  logs dashboard, no service editing, no new build providers, no Docker or
  Kubernetes, no DMG, signing, notarization, or CI/CD (`tasks/020.md` §43).
- The active workspace selection lives in memory only; it is not persisted
  across restarts.
- The development icon (`src-tauri/icons/icon.png`) is an obvious placeholder;
  real artwork is a packaging concern for a later task.

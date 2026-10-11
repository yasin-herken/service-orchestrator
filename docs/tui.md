# The Ratatui TUI

Service Orchestrator ships a keyboard-first terminal interface for human
developers:

```bash
service-orchestrator tui
```

Running the binary with no arguments also launches the TUI. The interface is
fully usable offline: it needs no LLM, no MCP client, and no remote backend.

The TUI is an **adapter**. It reads application state, renders it, and turns
keyboard input into application use cases. It never runs `git`, `mvn`, `npm`,
`liquibase`, or an operating-system process itself — every mutating operation
goes through the application core and, when long-running, the task engine. This
is the same rule the MCP server follows, so both interfaces observe identical
behavior, policies, and error semantics (`AGENTS.md` §1, §21, §33;
`docs/adr/001-core-architecture.md`).

---

## 1. Architecture

The adapter lives in `service-orchestrator-tui`. It is split into small,
single-purpose modules so that no file mixes rendering, state, and I/O:

```text
crates/tui/src/
├── lib.rs            re-exports the public surface
├── action.rs         user intents (Action)
├── app.rs            the state machine: Action -> state change | application call
├── event.rs          AppEvent, Effect, AsyncOutcome
├── input.rs          Key / InputEvent and the crossterm translation
├── observer.rs       the read-only TaskObserver boundary over the task engine
├── state.rs          UiState (presentation) and Snapshot (application cache)
├── router.rs         composes the whole frame
├── theme.rs          the colour system
├── terminal.rs       terminal lifecycle and the async event loop
├── error.rs          TuiError
├── testing.rs        in-memory fakes and a TestBackend renderer
├── components/       table, status, dialog, footer widgets
└── screens/          one module per screen
```

The flow is one-directional:

```text
key / task event / async result / tick
        │
        ▼
    App::handle_event            (state machine)
        │            │
        │            └── Effect ──► terminal event loop ──► Application query
        ▼
   UiState change
        │
        ▼
   router::draw (&App) ──► Ratatui ──► terminal
```

- **`App`** is the only place where an `Action` becomes either a change of
  interface state or an application-core call. It is pure enough to drive from
  a test without a terminal.
- **`Effect`** values are *descriptions* of asynchronous queries (Git status,
  branch listing, artifact inspection, realtime log streaming). The state
  machine emits them; only `terminal.rs` runs them.
- **Screens** and **components** receive `&App` and render. They never mutate
  state and never call the application core.

### UI state vs application state

Two kinds of state are deliberately kept apart:

- `UiState` is *presentation* state: the active screen, the cursor, the
  selection set, dialogs, the search query, scroll offsets, and the cached
  results of on-demand inspections. The application core never sees it.
- `Snapshot` is a read-only *cache* of application state: workspace status,
  service statuses, libraries, migration configuration, health, tasks, and Git
  state. It is replaced wholesale on refresh; the application core remains
  authoritative.

Keeping these separate is what lets the interface stay a pure adapter: the UI
never re-derives business meaning, and the application never sees a cursor.

---

## 2. Layout and screens

```text
┌──────────────────────────────────────────────────────────────────────┐
│ SERVICE ORCHESTRATOR                                                 │
│ Workspace: Acme Platform                  Running: 8/40    Tasks: 3  │
├─────────────────┬────────────────────────────────────────────────────┤
│ Navigation      │ Services                                           │
│                 │                                                    │
│ > Dashboard     │ [x] auth-service   RUNNING  main   :8081   healthy │
│   Services      │ [ ] web-ui         STOPPED  main   :3000   unknown │
│   Git           │                                                    │
│   Builds        │                                                    │
│   Libraries     │                                                    │
│   Migrations    │                                                    │
│   Tasks         │                                                    │
│   Logs          │                                                    │
│   Health        │                                                    │
│   Settings      │                                                    │
├─────────────────┴────────────────────────────────────────────────────┤
│ [Space] select  [Enter] details  [s]tart  [x] stop  [?] help         │
└──────────────────────────────────────────────────────────────────────┘
```

Screens:

| Screen | Shows |
|---|---|
| **Dashboard** | Workspace, service/health counts, library and repository counts, active and failed task counts, recent operations. Every number is derived from real application state. |
| **Services** | Selectable list: name, type, branch, runtime state, health, port. Home of the per-service and bulk actions. |
| **Service** (detail) | Identity, workspace, repository path/URL/default branch, module path, Git state, runtime requirements, process state, health, ports, dependencies, last task, recent errors. |
| **Git** | Per-service repository status, current branch, dirty indicator, and fetch/pull/sync/checkout plus the destructive reset/discard/clean operations. |
| **Builds** | The build system the application reports for each service (Maven, npm, … — never hard-coded), whether a build command is configured, and the last build task. |
| **Libraries** | Artifact coordinates, configured version, build system, cached Git state, artifact presence, and consumer version alignment. |
| **Migrations** | Migration-enabled services, provider, database target, and status/pending/preview/validate/apply. Read-only operations are visually distinct from the database-mutating **apply**. |
| **Tasks** | Task id, type, service, status, progress, duration, and last message, with a status filter. |
| **Logs** | A bounded, filterable, followable view over service or task logs. |
| **Health** | Per-service process, TCP, and HTTP status and the aggregated overall health, with a manual check action. |
| **Settings** | Read-only: configuration path, schema version, active workspace, execution concurrency, default timeout, configured runtime requirements. |

The service detail view is a sub-view of Services: `Enter` opens it, `Esc`
returns.

### Terminal lifecycle

`terminal.rs` enters raw mode and the alternate screen once and restores them in
a `Drop` guard, which also runs while unwinding. A panic hook restores the
terminal before the default hook prints, so a panic never leaves the developer's
shell in raw mode. Ctrl+C is handled as a normal quit. When the terminal is
smaller than 60×14 the interface draws an informative minimum-size notice
instead of a broken layout.

---

## 3. Keyboard shortcuts

Navigation is keyboard-first and vim-friendly.

**Global**

| Key | Action |
|---|---|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `Tab` / `←` | Move focus to the navigation panel |
| `Enter` | Activate navigation entry / open details / confirm dialog |
| `Esc` | Close dialog / leave detail view / clear selection / back |
| `Space` | Toggle the selection of the current row |
| `/` | Search |
| `R` | Refresh the snapshot |
| `W` | Cycle the active workspace |
| `?` | Help overlay |
| `q` / Ctrl+C / Ctrl+D | Quit |

Uppercase keys are distinct from lowercase ones: lowercase acts on the row under
the cursor, uppercase acts on the whole selection.

**Services / Service detail**

| Key | Action |
|---|---|
| `Enter` | Open the service detail view |
| `s` / `x` / `r` | Start / stop / restart |
| `b` / `f` / `y` / `p` | Build / fetch / sync / pull |
| `h` | Health check |
| `l` | View logs |
| `S` / `X` / `R` | Start / stop / restart the selection |
| `B` / `F` / `Y` / `H` | Build / fetch / sync / health-check the selection |
| `a` / `c` / `t` | Select all visible / clear selection / cycle the filter |

**Git** — `c` checkout (choose a branch), `f`/`F` fetch, `y`/`Y` sync, `p` pull,
`r` refresh status, `d` hard reset, `D` discard local changes, `n` clean,
`l` logs, `a` select all visible.

**Builds** — `Enter`/`b` build, `B` build selected, `l` logs, `t` tasks,
`c` clear selection.

**Libraries** — `b` build, `i` install, `p` prepare, `y` sync, `v` verify
artifact, `a` inspect version alignment, `r` refresh status.

**Migrations** — `s` status, `p` pending, `P` preview, `v` validate, `a` apply,
`l` logs.

**Tasks** — `Enter`/`l` view task logs, `x` cancel, `t` cycle the status filter.

**Logs** — `f` toggle follow, `c` clear, `v` cycle the minimum level,
`↑`/`↓` scroll (which pauses follow).

**Health** — `Enter`/`h` check, `H` check the selection, `l` logs.

While the search bar is active it captures printable characters, `Backspace`,
`Enter` (apply), and `Esc` (cancel). While a dialog is open it captures `Enter`,
`Esc`, and `↑`/`↓`.

---

## 4. Multi-selection

Selection is a set of **typed identifiers** (`ServiceId`, `LibraryId`) held in a
`BTreeSet`, never a list of visible row indices. Consequences:

- filtering or searching never changes what is selected — a hidden row stays
  selected;
- the same row cannot be selected twice;
- bulk operations target exactly the selected services and nothing more.

`Space` toggles the current row, `a` selects every *visible* row (respecting the
active filter and search), and `c`/`Esc` clear the selection. When nothing is
selected, the per-row actions fall back to the row under the cursor.

Bulk operations reuse existing application workflows: `start`, `stop`,
`restart`, `sync`, and `fetch` submit one workflow whose steps are per-service;
`build` and `health check` run per service. A completed workflow reports its
succeeded / failed / cancelled / skipped counts, so a partially failed bulk
operation is never shown as a success.

---

## 5. Operation integration

Every operation is an application use case:

| Area | Application calls |
|---|---|
| Services | `start_service`, `stop_service`, `restart_service`, `build_service`, `sync_service`, `check_service_health`, and the bulk `start_services` / `stop_services` / `restart_services` |
| Git | queries `get_git_status`, `list_branches`; commands `fetch_repository`, `pull_repository`, `checkout_repository`, and the destructive `reset_repository_hard`, `clean_repository`, `discard_local_changes`, plus bulk `fetch_repositories`, `sync_repositories` |
| Builds | `build_service`, `build_library`, `install_library`, `prepare_library`, `sync_library` |
| Libraries | `list_libraries`, `get_library_status`, `verify_library_artifact`, `inspect_library_version_alignment` |
| Migrations | `get_migration_status`, `inspect_migration_pending`, `preview_migration`, `validate_migration`, `apply_migration` |
| Tasks | read through the `TaskObserver` boundary; cancel through `cancel_task` |
| Logs | `get_service_logs`, `get_task_logs`, `subscribe_service_logs` |
| Health | `check_service_health`, `list_service_health` |
| Workspace | `sync_workspace`, `build_workspace` |

The interface holds no operation logic of its own. It maps a key to an `Action`,
and `App` maps the `Action` to a use case, presenting the structured
`OperationResult` or `ApplicationError`.

### Git safety

Fetch, pull, sync, and checkout are safe writes. Hard reset, clean, and discard
local changes are destructive: the application denies them under standard
authorization and the interface opens a confirmation dialog. Only an explicit
confirmation re-runs the operation with destructive authorization. The same
policy check runs inside the application, so a confirmation in the UI is never
the authorization itself (`AGENTS.md` §29, §30).

### Migrations

Migration reads (status, pending, preview, validate) are non-mutating. **Apply**
mutates the database and is treated as a controlled operation: it flows through
the same policy path and confirmation dialog, and the screen labels read-only
actions distinctly from the mutating one. Maven POMs and migration files are
never rewritten from the UI.

---

## 6. Asynchronous event loop

The loop is non-blocking. Terminal input is read on a dedicated thread (crossterm's
reader is blocking) and forwarded onto a single Tokio `mpsc` channel together
with task-engine events, realtime log entries, the results of spawned queries,
and a 250 ms tick. The loop draws, then awaits exactly one event.

Nothing blocks the renderer on Git, Maven, npm, Liquibase, process startup, or a
health check: those run as tasks on the task engine or as spawned application
queries whose results arrive back on the same channel.

### Event-driven updates

- **Task events** — the interface subscribes to the engine's broadcast stream,
  so task creation, progress, success, failure, and cancellation update the
  tasks screen immediately.
- **Logs** — subscribing to a service opens a `LogSubscription`; entries are
  streamed onto the channel. The view holds at most 2000 entries, so a noisy
  service can never grow the interface without limit.
- **Periodic refresh** — cheap, in-memory application queries rebuild the
  snapshot every four ticks (about once a second). No repository is polled;
  Git and artifact inspections happen only when the developer asks for them.

Realtime task progress uses only what the engine reports (status, elapsed time,
phase, and progress when present). The interface never invents a percentage.

### Task progress and logs

The tasks screen shows the engine's own list — running tasks first, then recent
history — including id, kind, service, status, progress, duration, and the last
message. The logs screen pages historical entries and follows realtime ones,
with a severity filter, a text filter, follow/pause, and a bounded buffer. The
interface never keeps a task registry of its own.

---

## 7. Error handling

Application errors are rendered as structured, actionable dialogs, not a generic
"operation failed":

```text
Operation Failed
Operation: Build service auth-service
Code: MAVEN_BUILD_FAILED
Reason:
Dependency resolution failed.
[Esc] Close
```

Permission-denied responses open the confirmation dialog instead. Background
query failures (for example a Git status that cannot be read) surface as a
severity-coloured toast in the footer, never as a panic.

---

## 8. Testing

- **Unit tests** cover the pure state machine helpers: cursor clamping and
  scrolling, filter cycling, log-buffer bounds, and input translation
  (`crates/tui/src/{state,input}.rs`).
- **Integration tests** (`crates/tui/tests/tui.rs`) drive the real `App` over a
  real `Application` wired to in-memory fakes and render real frames through
  Ratatui's `TestBackend`. They cover startup and listing, every screen rendering,
  keyboard navigation and quit, filter-stable typed-id selection, search,
  action dispatch to the task engine, bulk operations, cancellation, task-event
  refresh, emitted effects, destructive confirmation, actionable error dialogs,
  the help overlay, and the minimum-size notice.
- **Fakes** (`service-orchestrator-tui::testing`) provide `FakeTasks` (both a
  `TaskManager` and a `TaskObserver`) and a `render` helper. No test starts a
  real Git, Maven, npm, or Liquibase process.

See `docs/adr/013-ratatui-tui.md` for the design rationale.

---

## 9. Limitations

- The interface is read-only where the application exposes no safe update path:
  Settings displays configuration but does not edit it, and library and migration
  screens never rewrite files.
- Advanced Git UX (interactive rebase, history rewriting, diff review) is out of
  scope; a future task may delegate to an optional external tool.
- The MVP coordinates Kubernetes and Docker only insofar as the application core
  does; the TUI adds no runtime of its own.
- Bulk `build` and `health check` run per service rather than as one workflow,
  because the application exposes no combined use case; their summary is
  reported as scheduled/failed counts.

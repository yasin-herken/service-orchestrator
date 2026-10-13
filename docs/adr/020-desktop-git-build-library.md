# ADR 020: Desktop Git, Build, and Library Management Through the Shared Daemon

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** Service Orchestrator maintainers

## Context

`tasks/020.md` asked the desktop GUI — already a service-management interface
(ADR 018/019) — to expose the existing Git, Build, and Library Manager
capabilities: browse repositories, build targets, and libraries; inspect real
Git state; synchronize repositories individually and in bulk with a preview;
build services through their configured build system; and prepare libraries with
artifact verification and dependency version alignment.

The TUI and MCP already reach all of these capabilities through the application
core. The task therefore had to add three new screens **without** re-introducing
the failure modes a second implementation invites:

- a second Git implementation with its own fetch/checkout/pull logic;
- a frontend loop that schedules one request per repository and bypasses the
  daemon's ordering and concurrency;
- a UI that silently discards local changes;
- build logic that hard-codes Maven (or npm) instead of asking the core;
- version "fixes" that rewrite a POM or change a dependency version;
- a second task monitor, or an unbounded log buffer in the webview.

Two constraints shaped the design. First, the adapter presents itself as a `Cli`
client, so the daemon's destructive Git operations (which require an interactive
same-UID client kind) are unavailable. Second, the build adapter's provider
registry — the single source of truth for which operations a build system
supports — was not reachable through the `OrchestratorClient` boundary.

## Decision

Extend the adapter with validated Git, build, library, and task/log commands and
three screens that reuse the shared core, keeping every decision below the
boundary.

- **One read-only core query for build capabilities.** A new
  `BuildCapabilityIndex` port (`crates/application/src/ports/build.rs`) exposes a
  projection of the build adapter's provider registry, with a
  `NullBuildCapabilityIndex` fallback that reports "no provider is registered".
  `Application::get_build_capabilities` authorizes a `Read`, resolves the
  service's configured build system, and returns its operations. This is the only
  capability the GUI needed the core to add; it puts no Maven or npm knowledge in
  the adapter.
- **Configuration-derived lists, on-demand inspection.** The repository list and
  the build-target list are derived from configuration in a single round trip
  (`list_services` plus the in-memory workspace status, or a capability query per
  *distinct* build system). No Git operation runs to open the Git screen, so it
  stays cheap at 100+ repositories. Real working-tree state and branches are
  inspected only when a repository is opened, selected for inspection, or after
  an operation settles.
- **One coordinated core operation per bulk action.** Bulk Git fetch, checkout,
  and sync map onto the core's existing `fetch_repositories`,
  `checkout_repositories`, and `sync_repositories` workflows; bulk builds map
  onto one `prepare_environment` workflow with only the build phase enabled; bulk
  library build/install/sync and preparation map onto one
  `prepare_selected_libraries` workflow. The GUI never loops over a selection, so
  ordering, concurrency limits, and duplicate protection stay in the daemon.
- **A read-only preview, and no silent discard of local changes.** The bulk
  synchronization dialog aggregates real status and branch data into a
  per-repository preview (dirty, missing branch, diverged, unavailable) and
  disables *Execute* when every repository is blocked. The adapter never stashes,
  resets, cleans, or force-checks-out; a dirty or unsafe repository is reported
  and the daemon re-checks and refuses at execution time. Pull is
  single-repository only because the core exposes no bulk pull.
- **Provider-agnostic builds.** `list_build_targets` attaches each target's
  `BuildCapabilities` (or a structured `capability_error`) and the UI renders
  operations as Available, Not exposed, or Unsupported with the core's own
  explanation, disabling anything not exposed.
- **Read-only version alignment.** Alignment is inspected on request and reports
  each consumer as aligned, mismatched, unknown, or unresolved. The GUI never
  rewrites a POM and never changes a version. Library preparation makes Git
  synchronization optional (off by default, no forced branch) so a developer can
  build the current local branch with uncommitted changes.
- **Shared task monitoring and bounded logs.** All three screens use one
  `OperationProgressDialog` and one `LogViewer`. Progress comes from real `Task`
  events grouped by workflow id; the log viewer keeps a bounded buffer (2,000
  entries) and follows a task with a stable sequence cursor. Cancellation uses
  the existing `cancel_task` / `cancel_workflow` APIs and never assumes a
  rollback.
- **The daemon remains the authority.** Every mutating command is submitted as a
  `Cli` client and authorized by the policy engine independently of the webview;
  a confirmation dialog is a usability affordance, never the authorization
  boundary. When the
  connection drops, every screen disables mutations and never replays an
  operation whose outcome was uncertain. Workspace switching scopes selection,
  detail, dialogs, and filters, so no state bleeds across workspaces.

## Alternatives considered

- **New bulk Git workflows in the daemon.** Rejected: the core already exposes
  coordinated bulk fetch, checkout, and sync. Adding new workflows would create a
  second path for ordering and safety with no new capability.
- **A frontend loop for bulk operations.** Rejected (`tasks/020.md` §7, §16): it
  would bypass dependency ordering, concurrency limits, and duplicate protection,
  and make the GUI a second scheduler.
- **A per-service capability query.** Rejected: a build provider's support set
  depends only on the build system, so capabilities are resolved once per
  distinct system rather than once per service.
- **A dedicated interactive `ClientKind` to unlock destructive Git operations.**
  Rejected for this task: it requires a protocol/interaction change and a
  confirmation flow that are separately justified. The destructive operations
  stay out of scope.
- **Auto-resolving version mismatches or rewriting POMs.** Rejected
  (`tasks/020.md` §24): alignment is reported, not enforced; rewriting a
  developer's build files silently is exactly the kind of implicit mutation the
  project forbids.
- **Executing the preview directly, or treating it as the safety boundary.**
  Rejected: the preview is read-only readability; the daemon enforces safety at
  execution time.
- **A second task-monitoring implementation per screen, or an unbounded log
  buffer.** Rejected (`tasks/020.md` §27, §35): one shared progress dialog and
  one bounded log viewer keep the webview's memory and behaviour predictable.

## Consequences

- The Git, build, and library screens are thin adapters: a repository synced from
  the GUI is the same task the TUI and MCP observe, and a build is the same
  engine operation. There is one daemon, one task engine, one process manager,
  one Git manager, one build registry, and one policy engine.
- The build adapter's provider registry is now reachable read-only through the
  application core, so any interface can render capabilities without naming a
  provider.
- The adapter's unit tests never run a process or a Git command;
  `tests/daemon_ipc.rs` drives a real daemon with fake executors over an
  in-memory runtime-state store and asserts the wire shape, structured error
  codes, single/bulk behaviour, provider capabilities, partial failures,
  alignment mismatch, cancellation, input validation, and the shared-daemon
  invariant, using fixture workspaces only.
- The frontend is testable with a typed mock of the Tauri API; screen behaviour
  (selection, preview, execute, capability disabling, alignment, prepare,
  disconnection, cancellation) is verified without a webview.
- Because the daemon's event surface is task-only, Git/build/library state
  changes made by another interface become visible after the next task settles
  or refetch; a dedicated event stream would let the screens drop the
  task-driven reload.
- A dedicated interactive client kind and destructive Git operations remain
  deferred, and no version is ever changed on the user's behalf.

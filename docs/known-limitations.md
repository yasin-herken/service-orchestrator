# Known Limitations

This page lists limitations that are **verified against the current
implementation**. They are deliberate or deferred, not bugs. Where a limitation
has its own guide, that guide has more detail.

## Configuration and editing

- **The TUI cannot edit configuration.** Settings is read-only; it reports the
  configuration the application loaded. The **desktop GUI** can edit it through
  the daemon ([`desktop-gui.md`](desktop-gui.md) §28), and you can always edit
  `config.toml` by hand.
- **MCP cannot edit configuration.** There are no configuration tools; MCP never
  rewrites a file.
- **Unknown configuration keys are rejected**, not ignored — a config written for
  a newer schema fails loudly.
- **A rewritten entity is rendered canonically** (nested tables may become inline
  tables) because the writer preserves formatting only outside the edited
  section.
- `~/.service-orchestrator/workspaces/` and `profiles/` are reserved and not read.

## Build and runtime

- **Gradle is not implemented.** A `build_system = "gradle"` value is rejected at
  validation; detection reports it but its provider is not installed.
- **No standalone dependency-install or test operation.** Dependency install is
  part of a build; there is no `test_service`.
- **No combined bulk-build use case.** `build_workspace` builds everything, or you
  call `build_service` per service.
- **Builds are not sandboxed.** Maven plugins and npm scripts run project code
  by design.

## Migrations

- **Flyway is not implemented.** It can be named in configuration but has no
  provider and fails at execution.
- **Cancelling a migration does not guarantee a rollback.**
- **A failed migration is never retried automatically and never claims a
  rollback.**
- **The in-process database lock does not coordinate separate daemons.**
  Liquibase's own lock is the cross-process mechanism and is never released
  automatically.
- **No isolated real-database migration test** ships in the repository.

## Git

- **The desktop GUI exposes no destructive Git operations** (`reset`, `clean`,
  discard changes, force stop).
- **No bulk pull.** Bulk synchronization is fetch + checkout + fast-forward.
- **Advanced Git UX** (interactive rebase, history rewriting, diff review) is out
  of scope.

## Libraries

- **Version alignment is read-only.** Misalignment is reported; no POM is
  rewritten and no version is changed automatically.
- **Installing a library does not change a consumer's dependency version.**

## Daemon and process state

- **Managed-process recovery after a crash/restart is not implemented.** A fresh
  daemon adopts nothing, reports zero managed services, and never signals a
  process by PID or port.
- **Runtime state, task history, workflow history, and logs are in memory.** They
  are lost on daemon restart.
- **Task history is retained in memory** without eviction; listing is bounded, but
  retention is future work.
- **Event delivery is best-effort.** A lagging subscriber drops events and must
  resynchronise; events are not persisted.
- **One daemon per configuration directory.** Two configurations run two daemons;
  there is no cross-daemon coordination.
- **No OS-level supervision.** The auto-started daemon is a detached child, not a
  `launchd` service, so it does not restart after logout or reboot.
- **The GUI cannot report a daemon-side dropped counter** for log/task event
  streams.

## Interfaces

- **The GUI has no Settings screen** (it renders a placeholder); configuration
  lives on the **Configuration** screen.
- **The GUI's active workspace selection is in memory only** and is not persisted.
- **Realtime refresh in the GUI is task-driven**; changes made by another
  interface appear after the next task settles or refetch.
- **Workflows are consumed, not authored.** The GUI selects profiles/groups but
  creates/edits them on the Configuration screen.
- **The TUI has no dedicated daemon-connection indicator**; an outage surfaces
  through error dialogs and toasts.
- **The TUI does not expose workspace sync/build or a bulk-restart key.**
- **Resource snapshots over MCP use the application's native field names**, while
  envelope/task/log views are camelCase.

## Packaging and platform

- **macOS builds are ad-hoc signed and unnotarized by default.** The build script
  seals the bundle ad-hoc so it is internally consistent, and supports Developer
  ID signing and Apple notarization when the standard Apple environment
  variables are provided. The reference build was produced without credentials,
  so Gatekeeper warns on first launch
  ([`macos-distribution.md`](macos-distribution.md) §10–§12).
- **The verified architecture is Apple Silicon.** Intel (`x86_64-apple-darwin`)
  and universal builds are supported by `scripts/build-macos.sh` when the
  matching Rust target is installed; they were not produced or verified in the
  reference environment ([`packaging-report.md`](packaging-report.md)).
- **The application icon is an original placeholder**, not final branding.
- **No automatic updates.** A new version is installed manually from a new DMG;
  user configuration and workspace data are preserved.
- **No CI/CD.** Distribution is entirely local.
- **Kubernetes and Docker targets are not implemented.**

## Security

- **Same-user trust only.** Any process already running as the user can connect;
  the daemon is not a defence against the user's own processes.
- **Redaction is best-effort.** An unrecognized secret shape in free text may
  survive ([`security.md`](security.md) §7).
- **Configuration may inline a secret.** New config files and backups are `0600`,
  but a pre-existing loose file is not re-tightened.
- **HTTP health-check URLs are not allow-listed.**
- **No remote access** — by design; there is no network transport.

## See also

- [`error-reference.md`](error-reference.md) — error codes for failures that are
  not limitations.
- [`security.md`](security.md) §11 and [`local-daemon.md`](local-daemon.md) §13 —
  the source discussions.

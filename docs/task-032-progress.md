# Task 032 — Progress Record

Working notes for **Template-Based Bulk Service Import** and the **Unified Bulk
Operations Dashboard**. This is a context-recovery record, not user
documentation; user-facing documentation lives in
[`bulk-service-import.md`](bulk-service-import.md),
[`service-templates.md`](service-templates.md),
[`bulk-operations.md`](bulk-operations.md), and
[`migration-presets.md`](migration-presets.md).

## Decisions

- **Templates, migration presets, and operation presets are ordinary
  configuration entities.** They live in `config.toml` (`[[templates]]`,
  `[[migration_presets]]`, `[[operation_presets]]`, `[discovery]`) and reuse
  the existing `ConfigChange` / commit / broadcast pipeline. There is no second
  configuration engine: the GUI edits them through the same
  `preview_configuration` / `commit_configuration` path as services.
- **One atomic write for a bulk import.** `ConfigRepository` gained
  `preview_batch` / `commit_batch(&[ConfigChange], expected_revision)`. A batch
  is applied in order to a single document, the whole proposal is validated,
  and only then is the file written with the existing atomic
  temp-file+backup+rename. The prototype-level description in `tasks/032.md`
  §15 is implemented as this single-write primitive rather than multiple
  sequential commits.
- **Built-in migration presets are merged, not synthesized into the file.**
  `status`, `update`, and `reset_local` exist as code-level defaults; a user
  preset with the same id overrides a built-in, and `MigrationPreset.builtin`
  reports where the definition came from. `Config::all_migration_presets()`
  returns the merged view; the `migration_presets` field stays user-only so a
  commit never rewrites built-ins into the file.
- **Only Liquibase + Maven presets.** A preset composes validated Maven goal
  tokens (`build_preset_goals`: no leading `-`, restricted charset, args pass
  through the existing argument validation). CLI execution mode is rejected;
  there is no Flyway, Gradle, Docker, or Kubernetes path.
- **Safety is layered.** `dropAll` marks a preset destructive automatically;
  `read_only` and `destructive` are mutually exclusive; a destructive preset
  must declare a non-empty `allowed_environments` (never `Unknown`), and that
  list never authorizes anything by itself. Destructive execution still goes
  through `Permission::Destructive` and the existing interactive
  same-UID authorization. A service whose `liquibase.preset` is destructive
  must declare a `database.environment` that its preset allows — enforced at
  configuration load and revalidated before every execution.
- **Templates are defaults, never a second source of truth.** A template can
  only set fields the service model already validates; it has no credentials,
  ports, or repository URL. Branch names and command *programs* cannot use
  variables; arguments, working directories, environment values, and health
  URLs can. Health checks in templates may only be `process` or `http` (an
  `http` URL may be built from `${port}`).
- **`${port}` is the sixth template variable.** `tasks/032.md` §4 lists five
  examples; the documented, validated `port` variable was added so a health URL
  can compose `http://localhost:${port}/actuator/health`. Resolution fails for
  a row with no assigned port.
- **Discovery is bounded and on demand.** A recursive walk up to
  `[discovery].max_depth` (default 6, cap 64), always excluding
  `.git`/`node_modules`/`target`/`build`/`dist`/`out` plus configured extras,
  never escaping the workspace root even when following symlinks, and capped at
  `max_repositories` (default 200). Technology detection reuses the existing
  repository inspector; detection results carry a confidence value.
- **Operation presets store selections, never authority.** A saved preset
  describes what a developer commonly runs (services/profiles/groups and
  phase options); it never stores credentials or authorization and never
  executes on load, and execution always re-runs full validation and policy.
- **No MCP tool additions for this task.** Cross-interface visibility comes
  from the existing shared daemon and configuration broadcast; the desktop GUI
  and the daemon read the same core state. The MCP crate only gained the
  `ApplicationError::Discovery` response arm so the new error surfaces
  correctly if one reaches MCP.
- **Discovery and inspection moved into a shared crate.** The bounded walker and
  the read-only repository inspector were root-binary-only modules. They now
  live in `crates/repository` (`service-orchestrator-repository`) so the desktop
  adapter and its IPC integration tests use the real implementation instead of a
  test double. `src/main.rs` consumes the crate; behavior is unchanged.
- **The desktop adapter returns application types directly for this feature.**
  `src-tauri/src/bulk.rs` does not introduce a parallel DTO layer: the types are
  already serializable wire contracts shared by the daemon. This keeps the
  command layer thin and prevents the GUI view from drifting from the planner.
- **Template preview uses the real planner.** “Preview resolved configuration”
  calls `plan_bulk_import` with one explicit repository; no substitution is
  implemented in TypeScript.
- **Restart composes stop, build, and start in one DAG.** Each build step depends
  on that service's stop step when they belong to the same restart, so the old
  process is shut down before its replacement is built. Start waits for both the
  stop and the build. Dependents stop first; builds and starts use dependency
  order.
- **Preset management uses the configuration pipeline.** The Bulk Operations
  screen saves/renames/deletes presets through `preview_configuration` →
  `commit_configuration`. “Set default” first clears the previous default in the
  same change set.

## Status

- [x] Domain: template variable model (`crates/domain/src/template.rs`),
      `TemplateId` / `MigrationPresetId` / `OperationPresetId`,
      `Service.build_operation`, `Library.build_operation`.
- [x] Configuration model: templates, migration presets (built-ins + merge),
      operation presets, discovery settings.
- [x] Configuration validation/normalization for all new entities.
- [x] Configuration management: 3 new `EntityKind`s, `ConfigChange` variants,
      diffs, affected references, snapshot fields, `preview_batch` /
      `commit_batch`.
- [x] Configuration tests: template/preset/discovery loading rules and batch
      atomicity (`templates_and_presets.rs`, `management.rs`).
- [x] Application: bulk import planning/commit, discovery queries and port,
      template preview, operation preset queries, destructive authorization
      rewrite, `PrepareEnvironmentRequest` extensions (`migration_preset`,
      `build_operation`, `restart`, `stop`).
- [x] Liquibase: preset goal/context composition, destructive revalidation,
      read-only/mutating checks.
- [x] Repository crate: bounded walker, exclusions, symlink containment,
      read-only inspector, technology detection (`crates/repository`, 18 tests).
- [x] Root: `src/main.rs` wired to the repository crate.
- [x] Client/daemon: six IPC operations, including the server-intercepted
      `commit_bulk_import` (the daemon computes the required permission).
- [x] Desktop adapter: `bulk.rs` commands + five adapter tests; `--lib` green.
- [x] React GUI: Templates screen, ten-step Bulk Import wizard, Bulk Operations
      dashboard, shared review/workflow hooks, three new navigation entries,
      configuration-diff labels, CSS.
- [x] Frontend tests: Templates (6), Bulk Import (5), Bulk Operations (6),
      shell labels; `npm run test` 30 files / 249 tests.
- [x] Documentation: `bulk-service-import.md`, `service-templates.md`,
      `bulk-operations.md`, `migration-presets.md`; updated
      `configuration.md`, `desktop-gui.md`.
- [x] Documentation and final validation.

## Validation

| Check | Result |
| --- | --- |
| `cargo fmt --check` | pass |
| `cargo check --workspace` | pass |
| `cargo test --workspace` | pass (all workspace unit, integration, and acceptance suites) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| `apps/desktop`: `npm run build` | pass |
| `apps/desktop`: `npm run typecheck` | pass |
| `apps/desktop`: `npm run test` | pass (30 files, 249 tests) |
| `apps/desktop/src-tauri`: `cargo test` | pass (99 adapter unit tests, 47 IPC integration tests) |

The acceptance suites drive real daemons and real Node fixtures. Two
timing-sensitive assertions were made bounded rather than single-shot so the
suite is stable under load: the HTTP health probe retries its check within the
existing deadline, and the desktop IPC test waits briefly for asynchronous
client attachment before asserting the shared-daemon client count. Both keep
their original assertion strength.

## Follow-up fixes found during live use

- Repository URL validation no longer treats a plain SSH username as a
  credential. `ssh://git@host:port/path.git` is accepted; passwords and HTTPS
  tokens are still rejected (`crates/domain/src/repository.rs`, tests, and
  `docs/configuration.md`).
- A failed build's one-line summary and bounded detail now prefer `[ERROR]`
  lines over harmless JVM `WARNING:` lines, so a Maven/Lombok failure is not
  reported as `WARNING: A restricted method in java.lang.System` alone
  (`crates/build/src/manager.rs`, tests, and `docs/troubleshooting.md`).
- A live build of `imos-mys-alacak` confirmed the cause of the reported failure:
  the daemon inherited Java 25 while the project targets Java 21. The build
  passes under Java 21 and fails under Java 25. The troubleshooting guide now
  explains that builds inherit the daemon's `JAVA_HOME`, while the service
  runtime pin is a start precondition.

## Final report

The 16-item completion report required by `tasks/032.md` §56 is delivered with
the task hand-off. The implementation, tests, and user documentation above are
the durable record.

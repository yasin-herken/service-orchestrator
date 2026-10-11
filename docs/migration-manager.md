# The Migration Manager

The Migration Manager is the infrastructure adapter that turns a validated
migration target into a structured migration plan and executes it. It is
**provider-based**: Liquibase is the initial implementation, but nothing in
orchestration is coupled to it. It lives in the `service-orchestrator-liquibase`
crate.

A migration always runs as a task. The application resolves a workspace-aware
`ResolvedMigrationTarget`, submits a task carrying it, and the engine invokes the
`MigrationTaskExecutor`; the executor delegates to the `MigrationManager`, which
selects a provider, plans, coordinates the database target, and runs the plan
through the process manager. The adapter never spawns an operating-system process
itself and never depends on Ratatui or MCP.

```text
Application use case
       │  (migration op as a TaskDefinition with target parameters)
       ▼
Task Engine ──> MigrationTaskExecutor ──┐
                                        ▼
                                  MigrationManager
                                 ┌──────┴───────┐
                                 ▼              ▼
                 MigrationProviderRegistry  MigrationRunner
                                 │              │
                                 ▼              ▼
                        LiquibaseProvider   Process Manager
                                 │              │
                                 └─ MigrationPlan ─┘
                                                 ▼
                                          Liquibase / Maven
```

> This task implements the Migration Manager and its Liquibase provider only.
> Flyway, automatic rollback, automatic lock release, production database
> management, arbitrary SQL execution, the TUI, and MCP are intentionally **not**
> implemented here.

---

## 1. What the manager is not

- It is **not** a business layer. Provider selection is driven by validated
  configuration; precondition checks (migration enabled, workspace path
  containment, environment authorization) live in the application. The manager
  contains no per-interface behavior and no service-specific branching.
- It is **not** a second scheduler. The task engine's `max_parallel_tasks` is the
  only global bound (`AGENTS.md` §41). The manager adds only *database-target*
  coordination, which removes a specific hazard rather than re-implementing
  scheduling.
- It is **not** a configuration loader. The manager reads the validated `Config`
  at execution time; it never parses TOML.
- It is **not** a process owner. Every process is started by the process manager
  through `MigrationRunner` (`AGENTS.md` §15).
- It is **not** provider-specific. Executables, Maven goals, CLI flags, output
  interpretation, and lock markers live in the provider.

---

## 2. The migration target

Services are never addressed by a bare name: two workspaces may legitimately
contain a service with the same id. Every operation takes a
`ResolvedMigrationTarget`:

| Field | Meaning |
| --- | --- |
| `workspace_id` | The owning workspace. |
| `service_id` | The service whose database is migrated. |
| `operation` | The provider-neutral operation to perform. |
| `path` | The service's absolute working directory. |
| `workspace_root` | The workspace root; the working directory must stay inside it. |
| `non_local_authorized` | Whether the caller explicitly authorized a non-local database. |

`ResolvedMigrationTarget::is_inside_workspace` is checked before anything runs; a
target outside its workspace fails with `MIGRATION_INVALID_TARGET`. The value
round-trips through the task engine via `to_parameters`/`from_parameters`, so a
migration task submitted by the TUI or MCP can be executed later without
re-reading configuration.

A migration target deliberately carries **no** database credentials, connection
strings, or configured environment values: task parameters are stored and may be
surfaced through the interfaces. The manager reads the connection metadata and
environment from validated `Config` when it runs (`AGENTS.md` §31,
`tasks/011.md` §11).

---

## 3. Migration operations and capabilities

Operations are provider-neutral names whose semantics are genuinely shared.
Providers declare which they support; an unsupported request is rejected with
`MIGRATION_UNSUPPORTED_OPERATION` before a process starts.

| Operation | Meaning | Liquibase (Maven) |
| --- | --- | --- |
| `Validate` | Validate the migration configuration without modifying the database. | `mvn -B liquibase:validate` |
| `Status` | Inspect migration status. | `mvn -B liquibase:status` |
| `Pending` | Inspect pending changesets. | `mvn -B -Dliquibase.verbose=true liquibase:status` |
| `Preview` | Generate SQL for a planned update without applying it. | `mvn -B liquibase:updateSQL` |
| `Apply` | Execute pending migrations. | `mvn -B liquibase:update` |

Only `Apply` mutates the database. The others are read-only inspections.

### Task kinds

| Task kind | Operation |
| --- | --- |
| `MigrationValidate` | `Validate` |
| `MigrationStatus` | `Status` |
| `MigrationPending` | `Pending` |
| `MigrationPreview` | `Preview` |
| `MigrationApply` | `Apply` |
| `Liquibase` (legacy) | `Apply` |

`TaskKind::Liquibase` is retained as an apply alias so workflows and callers
written before the manager existed keep working (`tasks/011.md` §8).

---

## 4. The migration plan

Providers generate a `MigrationPlan` rather than executing anything. A plan
contains:

| Field | Purpose |
| --- | --- |
| `program` | The executable (`mvn`, `./mvnw`, `liquibase`, or a configured program). |
| `args` | The argument vector, passed verbatim and never through a shell. |
| `working_dir` | The absolute directory the migration runs in. |
| `environment` | Environment variable *values* applied to the process. |
| `timeout` | An optional maximum runtime declared by the provider. |
| `provider`, `operation` | What the plan targets. |
| `database_identity` | A display-safe identity of the target database. |
| `target_key` | The stable key identifying the physical database target. |
| `safety` | The risk classification (`read_only`, `local_write`, `authorized_write`). |

`MigrationPlan`'s `Debug` implementation prints the environment variable *count*
rather than its values, so a plan can be logged without leaking a credential. The
`command()` helper renders only the program and arguments.

---

## 5. Provider selection

`MigrationProviderRegistry` is a small resolver — a map from
`MigrationProviderType` to `MigrationProvider`, no discovery and no lifecycle:

- `MigrationProviderRegistry::standard()` registers Liquibase;
- `resolve(provider)` returns the provider or `MIGRATION_PROVIDER_NOT_FOUND`;
- the manager then checks `supports(operation)` and rejects an unsupported
  operation with `MIGRATION_UNSUPPORTED_OPERATION`.

`MigrationProviderType` is open at the edges: `Custom(String)` carries a
project-specific provider name, so configuration can name a tool the domain does
not model yet. Naming a provider that is not registered is a structured error at
execution time, never a silent fallback.

Adding a provider (Flyway, for example) is: implement `MigrationProvider`,
register it in `standard()`, extend configuration validation, and add tests. The
manager, the engine, the application core, and the interfaces are unchanged.

---

## 6. The `MigrationProvider` abstraction

`MigrationProvider` is deliberately small. It exposes the provider type it
implements, its capability set, a `validate` step, and `plan`. It is a
synchronous, stateless trait: all of its inputs arrive as arguments, so a
provider is pure and trivially testable. Providers observe their environment only
through a one-method `BuildHost` seam (`file_exists`, `environment_has`), reused
from the Build Manager, which keeps them deterministic under test and free of
direct filesystem or `std::env` access.

Crucially, the trait has **no** method that runs anything. Execution is not a
provider responsibility; a provider's only output is a value.

---

## 7. The Liquibase provider

### Execution modes

- **Maven** (the default): Liquibase is invoked through the Maven Liquibase
  plugin. The provider resolves the Maven Wrapper (`./mvnw`) when it exists in the
  working directory and falls back to `mvn` from `PATH`, exactly as the Build
  Manager does. **No Maven plugin version is ever hard-coded**
  (`tasks/011.md` §6, §25).
- **CLI**: Liquibase is invoked through a CLI executable. Because the CLI is not
  guaranteed to be installed and its invocation flags are not universal, this
  mode is only usable when validated configuration supplies a command for the
  operation; otherwise the request is rejected with `MIGRATION_COMMAND_MISSING`.

### Configured commands win

When configuration declares a command for the requested operation, it is
authoritative and used verbatim — still structured (a program plus an argument
vector, never a shell line). Only the provider's *default* Maven plan needs
Maven knowledge. This is the seam that makes provider-independent orchestration
testable without invoking Maven or Liquibase.

For the **apply** operation only, the legacy `[services.commands] liquibase`
command is a fallback when no per-operation apply command is set, so
configuration written before the migration manager existed keeps working.

### Output interpretation

The provider does not parse Liquibase's human-readable output to decide success.
The process exit status is the primary outcome signal; captured output is
preserved for diagnostics (redacted and bounded). If a pending-changeset count
cannot be reliably determined, the manager returns no count rather than guessing
(`tasks/011.md` §24).

---

## 8. Configuration

Migration is still configured under `[services.liquibase]`, extended rather than
replaced so existing files keep loading. The legacy single `command` key maps to
the apply (`update`) operation; an explicit `commands.update` always wins.

```toml
[services.liquibase]
enabled = true                 # defaults to true when the block is present
provider = "liquibase"         # defaults to "liquibase"
execution_mode = "maven"       # "maven" (default) or "cli"
timeout_seconds = 300          # optional; falls back to execution.default_timeout_seconds

[services.liquibase.commands]
validate = "./mvnw liquibase:validate"
status   = "./mvnw liquibase:status"
pending  = "./mvnw liquibase:status"
preview  = "./mvnw liquibase:updateSQL"
update   = "./mvnw liquibase:update"

[services.liquibase.database]
environment   = "local"        # local | shared_development | staging | production | <other>
database_type = "postgresql"
name          = "acme_auth"
host          = "localhost"
schema        = "public"
contexts      = ["local"]
labels        = []

[services.liquibase.environment]
LIQUIBASE_COMMAND_CONTEXT = "local"
```

Every command may be a shell-style line or the structured
`{ program = "...", args = [...] }` form; lines are split into a program and
arguments, never re-interpreted by a shell (`AGENTS.md` §32).

**Credentials are never stored here.** Secrets belong in the environment,
referenced by name, and are redacted whenever the environment is displayed
(`AGENTS.md` §31, `tasks/011.md` §11).

See `docs/configuration.md` §5.7 for the full schema.

---

## 9. Database target identification

A migration must have an identifiable target database. The target is described
by display-safe metadata only — environment, type, name, host, schema, contexts,
labels — never a password or a full credential-bearing JDBC URL.

- `MigrationDatabaseConfig::display_identity()` renders
  `type/name@host/schema (environment)`, with missing parts shown as `?` so an
  unidentified target is visibly unidentified.
- `MigrationDatabaseConfig::target_key()` renders
  `type=…;host=…;name=…;schema=…` and is the key the database coordinator uses.
  The environment is **not** part of the key: it is a safety classification, not
  part of a database's identity, so two services pointing at the same physical
  database but labelling it differently still coordinate.

Both values are attached to the plan and to successful task metadata; neither can
contain a credential by construction.

---

## 10. Environment safety

The initial implementation is intended for local development. The default policy
is:

> **Only explicitly configured local development targets may execute migrations
> automatically.**

`MigrationTargetEnvironment` has six variants (`Local`, `SharedDevelopment`,
`Staging`, `Production`, `Other(String)`, `Unknown`). Only `Local` may migrate
without explicit authorization. Every other value — including `Unknown`, the safe
default when nothing is declared — requires authorization.

The environment is **never inferred from a hostname**: `localhost` is not treated
as local, because local port forwarding and tunnels may point at a remote
database (`tasks/011.md` §10). A declared but unmodelled name becomes
`Other(name)`, which is non-local.

### Where the policy is enforced

1. **Application boundary.** Before scheduling, `resolve_migration` maps read-only
   operations to `Permission::Read`, apply-to-local to `Permission::SafeWrite`,
   and apply-to-any-other-declared-environment to `Permission::Destructive` — the
   same explicit confirmation destructive Git operations use. The resolved target
   records whether that authorization was granted.
2. **Manager re-check.** `MigrationManager::check_safety` re-applies the policy at
   execution time, so it cannot be bypassed with a hand-crafted task. A mutation
   of an unidentified target fails with `MIGRATION_DATABASE_TARGET_UNKNOWN`; a
   mutation of a known non-local target the caller did not authorize fails with
   `MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED`.

Read-only operations require no authorization on any environment.

---

## 11. Database-level concurrency

Multiple services may share one database, which is common in a microservice
architecture:

```text
auth-service ─────┐
                  ├── shared-development-db
user-service ─────┘
```

Two migrations aimed at the same configured database must not run at once:
Liquibase's own change-log lock is a safety net, not a scheduler, and two tasks
racing over one database can still interleave before the lock is taken
(`tasks/011.md` §15).

`DatabaseCoordinator` is a keyed write lock. The manager acquires the target's
key for the duration of a run, so two writers never race over one database; a
different key never contends, so unrelated targets still migrate concurrently.
The coordinator is intentionally not a scheduler: the task engine remains the
only concurrency authority.

> **Limitation.** The coordinator coordinates migrations *within one running
> Service Orchestrator instance*. It does **not** coordinate separate instances, a
> developer's hand-run Liquibase, or CI. Liquibase's database lock is the only
> cross-process mechanism, which is why the coordinator is a defense-in-depth
> improvement rather than a replacement.

When the system cannot prove two targets are distinct (an unidentified target),
they share the same (mostly empty) key and are conservatively serialized.

---

## 12. Liquibase lock handling

Liquibase uses database locking to coordinate changeset execution. The manager
recognizes common lock-related failures by scanning the captured output for
case-insensitive markers (`could not acquire change log lock`, `lockexception`,
`waiting for changelog lock`) and reports them as `MIGRATION_LOCK_CONFLICT` with
actionable diagnostics.

The manager **never** automatically releases a Liquibase database lock and
**never** modifies Liquibase tracking tables (`tasks/011.md` §16). Any future
force-release capability must require explicit authorization and separate
implementation. Because Liquibase's messages differ across versions, the markers
are used only to *classify a failure for diagnostics*, never to drive behavior.

---

## 13. Cancellation and timeouts

Cancellation is cooperative and integrated with the task engine and the process
manager. The task's `Cancellation` token is passed to `spawn_with_cancellation`,
so a cancelled task stops the running migration gracefully
(`ExitReason::Cancelled` → `MIGRATION_CANCELLED` → `TaskExecutorError::Cancelled`).

A task's timeout takes precedence over the plan's own declared timeout
(`MigrationPlan::timeout`, set from `timeout_seconds`); a timed-out migration is
reported as `MIGRATION_TIMEOUT`.

> **Cancellation does not imply rollback.** Stopping the external process does
> not guarantee that database changes have been rolled back
> (`tasks/011.md` §18). After cancellation the managed process is stopped, the
> cancellation is recorded, and no migration is automatically rerun. Database
> state may require reinspection; the system never claims a rollback without
> verification.

---

## 14. Failure semantics

A failed migration marks the task failed, preserves the exit information (exit
code, bounded redacted output), and reports the affected service and database
target. It **never** claims the database was rolled back
(`tasks/011.md` §17), because a migration is not necessarily atomic across all
changesets or platforms.

- **No automatic retry.** Migration execution is submitted with
  `RetryPolicy::none`. A failed migration may have partially modified the
  database, so a retry requires a new, explicitly inspected operation
  (`tasks/011.md` §19). Read-only inspections may be retried by the caller, but
  the manager does not retry them on its own either.
- **Failure propagation.** Because a migration is a workflow step, a failed step
  prevents dependent startup steps from proceeding, exactly like any other task.

### Structured errors

Every failure is a `MigrationError` variant with a stable, machine-readable
`code()`.

| Code | Meaning |
| --- | --- |
| `MIGRATION_PROVIDER_NOT_FOUND` | No provider is registered for the requested provider. |
| `MIGRATION_DISABLED` | Migrations are disabled for the service. |
| `MIGRATION_NOT_CONFIGURED` | The service has no migration configuration. |
| `MIGRATION_UNSUPPORTED_OPERATION` | The provider does not support the operation. |
| `MIGRATION_COMMAND_MISSING` | No command is configured for the operation (CLI mode). |
| `MIGRATION_INVALID_CONFIGURATION` | The migration configuration is invalid. |
| `MIGRATION_INVALID_TARGET` | The target is invalid or escapes its workspace. |
| `MIGRATION_DATABASE_TARGET_UNKNOWN` | The target's environment could not be identified. |
| `MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED` | A non-local target was not authorized. |
| `MIGRATION_LOCK_CONFLICT` | A Liquibase lock blocked the migration. |
| `MIGRATION_STATUS_UNAVAILABLE` / `MIGRATION_PREVIEW_UNSUPPORTED` | The operation is unavailable. |
| `MIGRATION_FAILED` | The migration exited non-zero. |
| `MIGRATION_TIMEOUT` / `MIGRATION_CANCELLED` | The migration timed out or was cancelled. |
| `MIGRATION_PROCESS_FAILED` | The process manager refused or failed to run the plan. |
| `MIGRATION_MISSING_PARAMETER` / `MIGRATION_INVALID_PARAMETER` | A task parameter was missing or malformed. |
| `MIGRATION_INTERNAL_ERROR` | An internal manager failure. |

Failures are mapped to the engine as
`TaskExecutorError::Failed { code, message, detail }`; a cancelled migration
becomes `TaskExecutorError::Cancelled`.

### Redaction and bounds

Captured output is bounded to 4096 bytes and any line containing a known
credential marker (`jdbc:`, `password=`, `pwd=`, `--password`) is dropped before
it reaches a log, a task result, an `MigrationOutcome`, or an interface. The
`MigrationContext`, `MigrationPlan`, and `MigrationDatabaseConfig` all render
display-safe `Debug`.

---

## 15. Migration preview

A preview (`Preview`) runs the provider's dry-run SQL generation — for
Liquibase, `liquibase:updateSQL`. It performs no database mutation, so it
requires only `Permission::Read`.

A preview is **informational only**: the system does not claim that generated SQL
guarantees the subsequent migration will be identical if database state changes
between preview and execution (`tasks/011.md` §13). Preview output is bounded and
redacted like any other captured output.

---

## 16. Multi-service migration

`apply_selected_migrations` applies migrations for several selected services as
one workflow. Services without enabled migration configuration are **explicitly
skipped** rather than failing the batch. Each enabled service becomes an
independent workflow step carrying its resolved target; the migration adapter
serializes steps that share a database target through `DatabaseCoordinator` and
lets unrelated targets run concurrently through the engine
(`tasks/011.md` §21).

Service-dependency ordering is intentionally **not** applied to migrations:
database targets are coordinated by identity, not by service graph, so two
independent services sharing one database are serialized by that database while
unrelated services proceed in parallel. Migration steps compose with future
environment-preparation workflows as ordinary workflow steps; the complete
`PrepareEnvironment` orchestration is not implemented in this task.

---

## 17. Task-engine integration

- `MigrationTaskExecutor` confirms the task is a migration kind, decodes the
  `ResolvedMigrationTarget` from the task's parameters, applies the task timeout
  and cancellation token, delegates to `MigrationManager`, and maps the
  structured result back to a `TaskOutcome` or `TaskExecutorError`. It contains
  no migration logic, so the TUI and MCP reach identical behavior through the
  same engine (`AGENTS.md` §21).
- `register_migration_executors` registers one shared executor against every
  migration `TaskKind` (including the legacy `Liquibase` apply alias). The
  composition root hands the finished registry to the engine via
  `TaskEngineBuilder::registry`.
- A successful migration returns structured metadata: `service`, `provider`,
  `operation`, `database`, and `exit_code`.
- There is no `MigrationManager` port in the application: status inspection is
  itself a process execution, so every migration operation is a task and the only
  boundary is the `ResolvedMigrationTarget` value a task carries — exactly like
  builds.

---

## 18. Application core integration

The application exposes workspace-aware use cases:

```text
validate_migration
get_migration_status
inspect_migration_pending
preview_migration
apply_migration
apply_selected_migrations
```

Each resolves a `ResolvedMigrationTarget`, applies policy, and submits a task or
workflow. `run_liquibase` (the pre-existing legacy command) delegates to
`apply_migration`; the Liquibase step of `prepare_environment` resolves a target
directly. No MCP-specific or Ratatui-specific type is involved
(`tasks/011.md` §26).

---

## 19. Logging

Migration operations produce **structured output through the task engine**, the
same approach the Library Manager takes (`docs/library-manager.md` §15). There is
no interface-specific formatting:

- the executor emits a progress message identifying the operation and service
  (`"apply migration for 'auth-service'"`);
- a successful task carries structured metadata — `service`, `provider`,
  `operation`, `database` (the display-safe identity), and `exit_code`;
- a failed task carries a stable, machine-readable code and bounded, redacted
  detail — never the raw process text.

Captured `stdout`/`stderr` is redacted and bounded (§14) before it is attached to
an outcome, a task result, or a log. No credential, password, or
credential-bearing connection string is ever written. The `logging` crate is a
stub in the MVP, so no `tracing` subscriber is wired yet; when it lands, these
same structured fields are the event payload (`AGENTS.md` §35).

---

## 20. Testing

Every test is local and offline; no Maven, Liquibase, database, or network is
involved (`tasks/011.md` §30, §31). A configured command is authoritative, which
is exactly the seam provider-independent orchestration relies on.

- Provider unit tests cover capability declaration, Maven wrapper resolution,
  Maven-goal mapping, configured-command authority, the legacy apply command,
  CLI-without-command rejection, and proof that planning never spawns a process.
- Manager unit tests cover environment classification, target identity,
  authorization enforcement, disabled/unconfigured services, unregistered
  providers, redaction, and safety labels.
- `crates/liquibase/tests/support/mod.rs` provides a temporary workspace and a
  configuration builder.
- `crates/liquibase/tests/manager.rs` runs tiny local utilities (`echo`, `false`,
  `sh`, `sleep`) to exercise the manager's classification of success, non-zero
  exit, lock conflict, timeout, cancellation, credential redaction, and
  shared-database serialization.
- `crates/liquibase/tests/task_engine.rs` covers the whole adapter flow
  (`TaskDefinition → engine → executor → manager → provider → runner → process
  manager`) and asserts on real task outcomes, including the structured failure
  codes for a failed migration, an unauthorized target, and a malformed task.

No test connects to a company database or requires production credentials.

### Optional real-Liquibase integration setup

A genuine end-to-end test needs a JVM, Maven, and network access, which the
default suite deliberately avoids. `crates/liquibase/tests/liquibase_maven.rs`
is therefore an **optional, `#[ignore]`d** setup that runs the real Liquibase
Maven plugin against an in-memory H2 database:

```text
cargo test -p service-orchestrator-liquibase --test liquibase_maven -- --ignored
```

It requires `mvn` on `PATH`, a JVM, and network access to fetch the plugin and
H2 driver on first run. It uses in-memory H2, so no company database,
credentials, tunnel, or port forwarding is involved. These tests are not part of
the default suite and are **not** reported as passing unless actually run
(`tasks/011.md` §31).

---

## 21. Limits and future work

- **No rollback.** Destructive rollback is deliberately out of scope; a failed or
  cancelled migration never claims a rollback.
- **No automatic lock release.** Liquibase locks are recognized and reported, not
  released.
- **No automatic retry.** A failed apply requires a new, inspected operation.
- **Pending count is unknown.** The manager reports no pending-changeset count
  rather than parsing fragile human-readable output.
- **Coordination is in-process only.** `DatabaseCoordinator` does not coordinate
  separate Service Orchestrator instances; Liquibase's database lock remains the
  only cross-process mechanism.
- **CLI mode needs configuration.** Liquibase CLI execution requires a configured
  command per operation; no default CLI invocation is assumed.
- **Preview is informational.** Generated SQL is not guaranteed to match a later
  execution.
- **Structured logging has no subscriber yet.** Migration operations emit
  structured task output and metadata, but the `logging` crate is a stub, so no
  `tracing` subscriber is wired to format it.
- **Flyway is planned but not implemented.** The abstraction, the domain
  vocabulary, and the registry are ready for it: implement `MigrationProvider`,
  register it, extend validation, and test.

See `docs/adr/011-migration-manager.md` for the decisions behind this design.

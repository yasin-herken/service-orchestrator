# Migrations

Service Orchestrator runs database migrations through a **provider-based**
migration manager. Liquibase is the initial implementation; orchestration is not
coupled to it. The design, target safety, and coordination model are in
[`migration-manager.md`](migration-manager.md).

Flyway can be named in configuration but has **no provider**, so a Flyway job
fails at execution with `MIGRATION_PROVIDER_NOT_FOUND`.

## 1. Configuration

```toml
[services.liquibase]
enabled = true
provider = "liquibase"          # default
execution_mode = "maven"        # "maven" (default) or "cli"
timeout_seconds = 300

[services.liquibase.commands]   # optional per-operation commands
validate = "./mvnw liquibase:validate"
status   = "./mvnw liquibase:status"
pending  = "./mvnw liquibase:status"
preview  = "./mvnw liquibase:updateSQL"
update   = "./mvnw liquibase:update"

[services.liquibase.database]   # display-safe target identity (never a credential)
environment   = "local"         # local | shared_development | staging | production | <other>
database_type = "postgresql"
name          = "acme_auth"
host          = "localhost"
schema        = "public"
```

- `enabled` defaults to `true` when the block is present.
- The legacy single `command` key maps to the apply (`update`) operation; an
  explicit `commands.update` wins.
- In `cli` execution mode a per-operation command must be configured, otherwise
  the request fails with `MIGRATION_COMMAND_MISSING`.
- **Credentials are never stored here.** Secrets belong in the environment,
  referenced by name.

## 2. Operations

| Operation | Mutates the DB? | MCP | TUI | GUI |
| --- | --- | --- | --- | --- |
| Configuration snapshot | no | `migration_status` | Migrations screen | Migration row |
| Validate | no | `validate_migration` | `v` | Validate |
| Live status | no | `get_migration_status` | `s` | Inspect status |
| Pending changesets | no | `inspect_migration_pending` | `p` | Inspect pending |
| SQL preview | no | `preview_migration` | `P` | SQL preview |
| Apply | **yes** | `apply_migration` | `a` | Apply (confirmed) |
| Bulk apply | **yes** | `apply_selected_migrations` | — | bulk Apply |

Only **apply** mutates the database.

### Configuration snapshot vs live status

`migration_status` reports configuration and the last observed values, so its
`pending`/`locked` fields may be `null`. The live probe is a scheduled task:
`get_migration_status` or `inspect_migration_pending`, then read the resulting
task. The TUI/GUI show "Unknown" for `pending`/`locked` until a real inspection
answers — they never guess.

## 3. SQL preview

`preview_migration` renders the SQL a migration would run (`liquibase:updateSQL`)
as a task, without applying it. Preview is read-only. It is the recommended step
before applying to any target that is not your local database.

## 4. Target environments and authorization

The database target is identified by **display-safe metadata only** —
environment, type, name, host, schema — never a credential, and locality is never
inferred from a hostname.

- A target whose `environment = "local"` may be mutated under `SafeWrite`.
- **Every other environment** — `shared_development`, `staging`, `production`,
  an unrecognized name, or an absent one (`unknown`) — requires
  **`Destructive` authorization**.

Policy is applied at submission and re-checked by the manager, so it cannot be
bypassed with a hand-crafted task. In practice:

- **TUI** — a non-local apply opens a confirmation dialog; only an explicit
  confirmation runs it.
- **MCP** — a non-local apply is **denied** with `CONFIRMATION_REQUIRED`. A human
  must approve it interactively. Planning still succeeds and reports
  `requires_destructive_authorization`.
- **GUI** — apply is a confirmed action; the daemon makes the final decision.

A denied apply reports `MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED` (or
`CONFIRMATION_REQUIRED` at the interface).

## 5. Database locking and coordination

- The manager serializes migrations against the **same configured database** with
  an in-process keyed write lock, so two writers never race. Unrelated targets
  still run concurrently.
- This lock is **in-process only**; it does not coordinate separate Service
  Orchestrator instances. Liquibase's own database lock is the cross-process
  mechanism. Service Orchestrator **recognizes** a Liquibase lock and reports a
  conflict (`MIGRATION_LOCK_CONFLICT`) but **never releases it automatically**.

## 6. Failure and cancellation

- A failed migration **never claims a rollback**, is **never retried
  automatically**, and reports the affected service and target with a stable
  error code. Applying a migration may be partially applied.
- **Cancelling a migration does not guarantee a rollback.** Cancelling stops the
  external process; committed database changes are not reverted. The interface
  reports the state the engine observed rather than assuming a rollback.
- A migration step in an environment workflow blocks the dependent build and
  start of that service if it fails.

## 7. Common migration errors

| Code | Meaning | Action |
| --- | --- | --- |
| `MIGRATION_PROVIDER_NOT_FOUND` | The configured provider has no implementation (Flyway). | Use Liquibase. |
| `MIGRATION_DISABLED` | Migrations are disabled for the service. | Enable them, or do not request the operation. |
| `MIGRATION_NOT_CONFIGURED` | No Liquibase block for the service. | Add `[services.liquibase]`. |
| `MIGRATION_COMMAND_MISSING` | `cli` mode without a per-operation command. | Configure the command. |
| `MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED` | A non-local target without destructive authorization. | Approve interactively on a local client. |
| `MIGRATION_LOCK_CONFLICT` | Another process holds the database lock. | Wait or resolve it manually; it is never released for you. |
| `MIGRATION_FAILED` | A changeset failed. | Read the migration logs; fix forward. |
| `MIGRATION_TIMEOUT` | The migration exceeded its timeout. | Raise `timeout_seconds`; check the DB. |
| `MIGRATION_STATUS_UNAVAILABLE` | Live status could not be read. | Check DB connectivity and configuration. |

## 8. Limitations

- Flyway is not implemented.
- Cross-process coordination relies on Liquibase's own lock.
- Cancellation cannot roll back committed changes.
- No migration is retried automatically.

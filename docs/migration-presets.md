# Migration Presets

A migration preset is a named, reusable way to run Liquibase. It exists so that
“show me pending changes”, “apply them”, and “reset my local database” are not
three ad-hoc command lines typed by a developer or improvised by an AI agent.

A preset is **not** a shell command. It names a small set of validated Maven
goals and an optional context list; the Liquibase provider composes them into a
structured argument vector. There is no preset field that can inject an option,
a path, or a shell token.

## 1. Built-in presets

Three presets exist without any configuration:

| Id | Name | Goals | Safety |
| --- | --- | --- | --- |
| `status` | Status | `liquibase:status` | Read-only |
| `update` | Update | `liquibase:update` | Mutating, non-destructive |
| `reset_local` | Reset Local Database | `liquibase:dropAll`, `liquibase:update` | Destructive, allowed on `local` |

A user-defined preset with the same id replaces the built-in entirely. The
listing keeps a `builtin` flag for display, but there is only one effective
preset per id.

## 2. Declaring a preset

```toml
[[migration_presets]]
id = "validate"
name = "Validate"
description = "Validate changelogs without touching the database."
goals = ["liquibase:validate"]
read_only = true

[[migration_presets]]
id = "sql-preview"
name = "SQL Preview"
description = "Render the SQL that update would run, without applying it."
goals = ["liquibase:updateSQL"]
contexts = ["local"]
read_only = true

[[migration_presets]]
id = "reset-local"
name = "Reset Local Database"
goals = ["liquibase:dropAll", "liquibase:update"]
contexts = ["local"]
destructive = true
allowed_environments = ["local"]
```

| Field | Meaning |
| --- | --- |
| `id` | Stable identifier, unique among presets. |
| `name` | Display name; defaults to `id`. |
| `description` | Optional free text. |
| `provider` | Only `liquibase` is accepted. |
| `execution_mode` | Only `maven` is accepted: a preset composes Maven goals and cannot express a CLI invocation. |
| `goals` | One or more Maven goal tokens; required. |
| `contexts` | Liquibase contexts. When omitted, the service's own `liquibase.contexts` apply. |
| `read_only` | The preset only inspects the database. |
| `destructive` | The preset destroys database objects. |
| `allowed_environments` | Required, non-empty for a destructive preset. |
| `timeout_seconds` | Optional per-preset timeout; at least 1. |

`read_only` and `destructive` are mutually exclusive. A preset may be neither
(an ordinary mutating update).

### Goal validation

A goal must be non-empty, must not start with `-`, and may contain only ASCII
letters, digits, `.`, `_`, `-`, `:`, and `+`. Options such as
`-Dliquibase.contexts=…` are composed by the provider, never supplied by a
preset. A goal list that includes `dropAll` must be marked `destructive = true`.

### Contexts

Contexts are names such as `sql`, `data`, or `onlyDev`. They may not contain
whitespace, commas, or shell metacharacters; the provider joins them with commas
into one `-Dliquibase.contexts=…` argument. This is the same
`liquibase.contexts` mechanism Liquibase itself uses, so existing changelogs and
context filters behave as expected.

## 3. Assigning a preset to a service

A service can declare a default preset:

```toml
[services.liquibase]
enabled = true
execution_mode = "maven"
preset = "status"

[services.liquibase.database]
environment = "local"
```

Rules:

- The preset id must exist (built-in or user-defined).
- A destructive preset additionally requires the service to declare a concrete
  database environment (not `unknown`) that appears in the preset's
  `allowed_environments`.
- A service without a preset keeps its existing Liquibase command behavior; the
  preset is an opt-in named invocation.

Bulk Operations may choose a preset for the whole operation without editing each
service. The operation-level preset takes precedence for that run and is still
subject to every rule below.

## 4. Safety model

Destructive database operations are treated more strictly than ordinary writes.

```text
preset declared
      ↓
environment must be allowed by the preset
      ↓
operation must require Destructive permission
      ↓
explicit authorization for this concrete target
      ↓
revalidation immediately before planning/execution
```

1. **`allowed_environments` is a precondition, not authorization.** It says
   which environments the preset *may* target. It never grants permission by
   itself, and it is never self-granted by a preset or a saved operation.
2. **A destructive preset always requires `Destructive` permission**, even when
   the database environment is `local`. `dropAll` destroys objects regardless of
   how the environment is named.
3. **The database target must be identified.** A destructive preset against an
   `unknown` environment is rejected with `DatabaseTargetUnknown` rather than
   assumed safe.
4. **Authorization is per execution.** A previous confirmation, a saved
   operation preset, or a configuration flag is not reused. Every execution
   asks again where the policy requires it.
5. **Revalidation happens in the manager**, immediately before planning or
   execution. A preset that became destructive, a database that changed
   environment, or a target outside `allowed_environments` cannot slip through a
   stale request.
6. **Read-only presets cannot run a mutating operation.** A plan that tries to
   is rejected as invalid configuration, not silently downgraded.
7. **No automatic retry and no rollback claim.** If the task fails, the daemon
   reports the structured failure. It does not retry `dropAll` and does not
   pretend a database was restored.

Credentials are never part of a preset. They come from the service's environment
and are redacted from logs and task results like every other secret
(`AGENTS.md` §31).

## 5. Execution path

A named preset invocation is produced by the shared migration manager:

```text
preset id + service database + operation
      ↓
manager resolves goals, contexts, safety, timeout
      ↓
Liquibase provider builds:  mvn -B <goals> [-Dliquibase.contexts=…]
      ↓
task engine runs it in the service directory
```

The command is a structured `program` + `args` vector. There is no shell, no
string interpolation of user values into a command line, and no per-interface
duplicate of this logic: the TUI, MCP, and desktop GUI all reach the same
application use case.

Read-only inspections (`status`, `validate`, `updateSQL`) do not mutate the
database. Mutating operations are task-producing, so progress and logs are
observed through the task engine.

## 6. GUI

- The **Migrations** screen shows the configured provider, mode, database
  environment, and the effective preset for each migration-enabled service.
- The **Bulk Operations** screen offers the available presets (built-ins plus
  user-defined) when migrations are enabled. Selecting a destructive preset
  shows an explicit warning. The warning is not authorization; the daemon still
  enforces the policy at execution time.
- Migration presets are configuration entities. They are written through the
  same revision-checked configuration manager as everything else; the GUI never
  edits TOML directly and never stores a preset outside the configuration.

## 7. What presets deliberately cannot do

- Define a raw shell command or arbitrary Maven options.
- Store a database URL, username, password, or token.
- Pin a target environment that overrides the service's declared one.
- Grant destructive authorization, for one run or permanently.
- Run automatically when the application starts or when a preset is loaded.
- Retry, roll back, or “restore” a database.

For the broader migration workflow, see [`migrations.md`](migrations.md) and
[`migration-manager.md`](migration-manager.md). For using presets inside a
coordinated environment operation, see
[`bulk-operations.md`](bulk-operations.md).

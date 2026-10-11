# ADR 011: The Migration Manager

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

Migrating a database is the step that turns synchronized sources and built
artifacts into a schema a service can run against. ADR 001 and ADR 004 placed a
`liquibase` adapter in the infrastructure layer and fixed the inward dependency
rule; ADR 005 established the task engine and its executor registry; ADR 006
established the process manager as the only component allowed to run an
operating-system process (`AGENTS.md` §15, §32); ADR 008 established the build
provider pattern this adapter follows; ADR 010 established the runtime adapter
and its third infrastructure-to-infrastructure exception.

The amendment in `tasks/011.md` shaped this decision with hard requirements:

- the Migration Manager must be **provider-based**; Liquibase is the initial
  provider and Flyway is planned but must not be implemented;
- providers must produce **structured plans** and must **not spawn processes or
  schedule tasks** — the process manager executes and the task engine schedules;
- a migration must have an **identifiable target database**, and the system must
  distinguish local development from shared, staging, and production databases;
- **only explicitly configured local targets may migrate automatically**;
  non-local targets require explicit authorization, and locality must never be
  inferred from a hostname;
- migrations against the **same configured database must not run concurrently**,
  while independent targets may;
- credentials must never appear in plans, logs, task results, or errors;
- **no automatic retry**, **no automatic rollback**, and **no automatic lock
  release**;
- migration operations must reuse the existing task engine, process manager,
  configuration, and policy layers.

## Decision

Implement the Migration Manager as the `service-orchestrator-liquibase` crate:

```text
                        MigrationManager
                       ┌──────┴───────┐
                       ▼              ▼
         MigrationProviderRegistry  MigrationRunner ──> Process Manager
                       │
                       ▼
               LiquibaseProvider
                       │
                 MigrationPlan
```

### A focused provider abstraction

`MigrationProvider` is deliberately small: the provider type it implements, its
capability set, a `validate` step, and `plan`. It is a synchronous, stateless
trait; all inputs arrive as arguments, so a provider is pure and trivially
testable. Providers observe the filesystem and environment only through the
one-method `BuildHost` seam, reused from the Build Manager. Crucially, the trait
has **no** method that runs anything: a provider's only output is a value.

Reusing `BuildHost` (rather than defining a second, identical seam) is a
deliberate, narrow coupling: it keeps Maven wrapper detection and environment
observation in one place and avoids duplicating the build adapter's logic
(`tasks/011.md` §25).

### Liquibase is the initial provider, configuration-selected

Liquibase is the tool the current backend services already use, so it is the
first concrete provider. `MigrationProviderRegistry::standard()` registers only
Liquibase, and configuration selects the provider by name. The domain's
`MigrationProviderType` is open at the edges (`Custom`), so a named-but-
unregistered provider is a structured `MigrationProviderNotFound` error at
execution time rather than a silent fallback or a parse failure.

The provider supports two execution modes. **Maven** (the default) resolves the
Maven Wrapper (`./mvnw`) or a system-wide `mvn`, reusing the Build Manager's
resolution and never hard-coding a plugin version (`tasks/011.md` §6). **CLI**
requires a configured command per operation, because the Liquibase CLI is not
guaranteed to be installed and its flags are not universal.

### Plans, not processes

A provider produces a `MigrationPlan`: an executable, its argument vector, a
working directory, the environment to apply, an optional timeout, the
display-safe database identity and its coordination key, and a safety
classification. The plan is inert data. `MigrationRunner` turns it into a
structured `ProcessSpec` and runs it through the process manager. This keeps the
"no direct process spawning" rule intact and keeps every migration supervised,
stoppable, and bounded by the engine's concurrency.

Unlike a build plan, a migration plan carries environment *values*: the database
configuration a migration needs is supplied through the environment. The plan's
`Debug` prints only the variable count, and the domain `Environment` redacts
secret values whenever the specification is displayed, so a credential is never
logged (`AGENTS.md` §31).

### Database-target coordination is required

Two services may share one database; two migrations racing over it can interleave
*before* Liquibase's change-log lock is taken. `DatabaseCoordinator` is a keyed
write lock whose key is the target database's stable identity (type, host, name,
schema); the manager holds it for the duration of a run. Distinct keys never
contend, so unrelated targets still migrate concurrently, and the coordinator is
**not** a scheduler — the engine's `max_parallel_tasks` remains the only global
bound. The environment is deliberately excluded from the key: it is a safety
classification, not part of a database's identity. An unidentified target yields
the same mostly-empty key and is conservatively serialized.

### Non-local targets require explicit authorization

`MigrationTargetEnvironment` models the *declared* environment with six variants;
only `Local` may migrate without explicit authorization. The application maps
read-only operations to `Permission::Read`, apply-to-local to
`Permission::SafeWrite`, and apply-to-any-other-declared-environment to
`Permission::Destructive`. The resolved target records whether that
authorization was granted, and the manager **re-checks** it at execution time, so
the policy cannot be bypassed with a hand-crafted task.

Locality is never inferred from a hostname: `localhost` is not local, because
local port forwarding and tunnels may point at a remote database. `Unknown` is
the safe default and is treated as non-local.

### Automatic retries are disabled

A failed migration may have partially modified the database, and a migration is
not necessarily atomic across all changesets or platforms. Automatic retry is
therefore off: the task is submitted with `RetryPolicy::none`, and a retry
requires a new operation after inspecting current migration status
(`tasks/011.md` §19). A partial-application failure is reported with its exit
information and never claimed to be rolled back.

### Cancellation does not imply rollback

Cancellation is cooperative through the task's `Cancellation` token and the
process manager's `spawn_with_cancellation`, so a cancelled migration stops the
managed process gracefully. But stopping the external process does not guarantee
database changes were rolled back: after cancellation the manager records the
cancellation, never reruns the migration, and never claims a rollback without
verification (`tasks/011.md` §18).

### Task-engine integration

Every migration operation is a task. `MigrationTaskExecutor` confirms the task
is a migration kind, decodes a `ResolvedMigrationTarget` from its parameters,
applies the timeout and cancellation token, delegates to the manager, and maps the
structured result back to the engine. `register_migration_executors` registers one
executor against `MigrationValidate`, `MigrationStatus`, `MigrationPending`,
`MigrationPreview`, `MigrationApply`, and the legacy `Liquibase` apply alias.
There is no `MigrationManager` port in the application, because status inspection
is itself a process execution — there is no read-only migration query an
interface would call directly (`docs/application-core.md`).

### A documented dependency exception

The migration adapter depends on `execution` (to implement a `TaskExecutor`) and
on `process` (to run migration tools), in addition to `application`, `domain`,
and — for the `BuildHost` seam and Maven resolution — `build`. This is the same
infrastructure-to-infrastructure exception ADR 007 recorded for Git and ADR 008
for Build, applied deliberately and recorded again here. The edge is legitimate
because the adapter is the concrete executor for migration tasks and the concrete
client of the process manager; it points outward from the domain, and neither
`build`, `process`, nor `execution` depends on `liquibase`.

## Alternatives considered

1. **One Migration Manager with a `match` on the provider.**
   Rejected: it couples orchestration to every migration tool, so each new tool
   edits the manager. The provider abstraction confines tool knowledge to one
   implementation per tool.

2. **A generalized plugin framework (dynamic loading, discovery, lifecycle).**
   Rejected as over-engineering (`tasks/011.md` §5): a
   `BTreeMap<MigrationProviderType, Arc<dyn MigrationProvider>>` is all the
   indirection the problem needs.

3. **Let providers spawn processes and schedule tasks.**
   Rejected: it violates the single-process-owner rule and would leave migrations
   outside the engine's concurrency, cancellation, and supervision
   (`AGENTS.md` §15, §9).

4. **Infer locality from the host (`localhost`).**
   Rejected: local port forwarding and tunnels can point at a remote database, so
   a hostname is not an environment (`tasks/011.md` §10). Environment is declared,
   and an unidentified target requires authorization.

5. **Treat an undeclared environment as local.**
   Rejected: it would let a misconfigured target migrate silently. `Unknown` is
   the default and is non-local.

6. **A global migration lock.**
   Rejected: it would serialize unrelated databases. The coordinator is keyed by
   database identity so only genuine conflicts wait (`tasks/011.md` §15).

7. **Automatically release a stale Liquibase lock on failure.**
   Rejected: automatically modifying Liquibase tracking state is unsafe. Locks
   are recognized and reported, never released (`tasks/011.md` §16).

8. **Retry a failed migration automatically.**
   Rejected: a failed migration may be partially applied; retrying without
   inspection can corrupt state (`tasks/011.md` §19).

9. **Parse human-readable Liquibase output to decide success or count pending
   changes.**
   Rejected: messages differ across versions. The exit status is the outcome
   signal; output is preserved for diagnostics, and an unknown pending count is
   returned rather than guessed (`tasks/011.md` §24).

10. **Introduce a competing `[services.migration]` configuration block.**
    Rejected: it would duplicate and diverge from `[services.liquibase]`. The
    existing block was extended, and the legacy `command` key maps to
    `commands.update` for compatibility (`tasks/011.md` §8).

11. **Map migrations to service-dependency order.**
    Rejected: database targets are coordinated by identity, not by the service
    graph. Independent services sharing a database are serialized by that
    database; applying the service graph would add ordering without a database
    reason (`tasks/011.md` §22).

12. **Give the Migration Manager its own concurrency limit.**
    Rejected: the task engine's `max_parallel_tasks` is the only global bound
    (`AGENTS.md` §41). The adapter adds only database-target coordination.

## Consequences

### Positive

- Migration-tool knowledge is isolated in providers; the manager, engine,
  application, and interfaces are provider-agnostic.
- Adding Flyway is a new provider plus a registry entry and validation
  support — no change to orchestration, the engine, the application, or the
  interfaces.
- Providers are pure and deterministic, so capability handling, wrapper
  resolution, and plan generation are unit-tested without running a tool.
- Safety is enforced twice (application policy and manager re-check), so it
  cannot be bypassed by a hand-crafted task; locality is never guessed.
- Every migration runs as a supervised, cancellable, timeout-bounded process
  through the single process owner.
- Credentials are structurally absent from plans, identity, logs, and results.

### Negative / trade-offs

- The `liquibase -> execution`, `liquibase -> process`, and `liquibase -> build`
  edges are a third explicit infrastructure-to-infrastructure exception; it must
  be applied deliberately to future adapters, not copied casually.
- The provider/plan abstraction adds indirection for a single initial provider;
  the payoff is that Flyway and custom tools do not touch orchestration.
- Database coordination is in-process only and does not coordinate separate
  instances or external tools; Liquibase's database lock remains the only
  cross-process mechanism.
- No rollback, no automatic lock release, and no automatic retry mean a partial
  failure requires a human (or a higher-level workflow) to inspect status and
  decide the next step.
- CLI execution mode requires per-operation configuration; it is not usable out
  of the box.

## Follow-up

- Implement `FlywayProvider` (and custom providers) on the same provider/executor
  pattern when a service needs one.
- Compose migration steps into `PrepareEnvironment` so a service's migration
  precedes its startup through the task engine (`tasks/011.md` §22).
- Surface migration status, preview output, and bounded logs in the TUI and MCP
  strictly through the application use cases.
- Consider a future, explicitly authorized lock force-release capability as a
  separate, guarded operation.

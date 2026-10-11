# ADR 008: The Build Manager

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

Building is the step that turns synchronized sources into artifacts: compiling
services, assembling frontends, installing shared libraries, and running test
suites. ADR 001 and ADR 004 placed a `build` adapter in the infrastructure layer
and fixed the inward dependency rule; ADR 005 established the task engine and its
executor registry; ADR 006 established the process manager as the only component
allowed to run an operating-system process (`AGENTS.md` §15, §32); ADR 007
established the Git adapter and the task-executor pattern.

The amendment in `tasks/008.md` shaped this decision with hard requirements:

- the Build Manager must be **build-system agnostic**;
- Maven is the primary implementation, npm the second; **Gradle is planned but
  must not be implemented**;
- a focused `BuildProvider` abstraction declares capabilities, validates
  build-system requirements, resolves the executable or wrapper, constructs
  structured build plans, and supplies build-system-specific error context;
- **providers must not spawn processes or schedule tasks** — the process manager
  executes and the task engine schedules;
- build-system-neutral operations (`Clean`, `Build`, `Test`,
  `InstallDependencies`, `PublishLocalArtifact`) are supported where their
  semantics are genuinely shared, and an unsupported operation returns a
  structured error;
- a small registry selects a provider from validated configuration;
- build-system selection is configuration-driven, and the schema must let Gradle
  be added later without breaking existing configurations;
- do not build a generalized plugin framework and do not duplicate scheduling or
  process management.

## Decision

Implement the Build Manager as the `service-orchestrator-build` crate:

```text
                        BuildManager
                       ┌──────┴───────┐
                       ▼              ▼
             BuildProviderRegistry  BuildRunner ──> Process Manager
                       │
         ┌─────────────┴─────────────┐
         ▼                           ▼
  MavenBuildProvider          NpmBuildProvider
         └──────── BuildPlan ────────┘
```

### A focused provider abstraction

`BuildProvider` is deliberately small. It exposes the build system it
implements, its capability list, a `validate` step, and `build_plan`. It is a
synchronous, stateless trait: all of its inputs arrive as arguments, so a
provider is pure and trivially testable. Providers observe their environment
only through a one-method `BuildHost` seam (`file_exists`, `environment_has`),
which keeps them deterministic under test and free of direct filesystem or
`std::env` access.

Crucially, the trait has **no** method that runs anything. Execution is not a
provider responsibility; a provider's only output is a value.

### Providers differ where reality differs

Not every system supports every operation, and pretending otherwise would hide
the distinction the operation vocabulary exists to express. Maven supports
`Clean`, `Build`, `Test`, and `PublishLocalArtifact`; npm supports `Build`,
`Test`, and `InstallDependencies`. Maven deliberately does **not** declare
`InstallDependencies` because its `install` phase *publishes* to the local
repository (that is `PublishLocalArtifact`); npm does **not** declare `Clean` or
`PublishLocalArtifact`. The registry/manager rejects an unsupported request with
`BUILD_UNSUPPORTED_OPERATION` before any process starts.

### Plans, not processes

A provider produces a `BuildPlan`: an executable, its argument vector, a working
directory, environment *requirements* (names only — never values, `AGENTS.md`
§31), an optional timeout, and expected outputs. The plan is inert data. The
`BuildManager` hands it to `BuildRunner`, which turns it into a structured
`ProcessSpec` and runs it through the process manager. This is what keeps the
"no direct process spawning" rule intact and keeps every build process
supervised, stoppable, bounded by the engine's concurrency, and cleaned up.

### Configured commands are authoritative

When configuration declares a command for the requested operation, the provider
uses it verbatim (still program-plus-arguments, never a shell line) instead of a
build-system default. Only the *default* plan requires build-system knowledge.
This is also the seam that makes provider-independent orchestration testable
without invoking Maven or npm.

### Build-system selection is configuration-driven, and open to Gradle

Configuration takes an optional `build_system` key (`"maven"` or `"npm"`) and,
when it is absent, infers the system from declared runtimes and from the
programs of the build/install/test commands. The domain's `BuildSystem` enum is
closed: only systems with a provider are modelled. Gradle is intentionally
**not** a variant and **not** accepted by validation, so a configuration can
never appear to select a system that cannot build it. Adding Gradle later is:

1. implement `GradleBuildProvider`;
2. register it in `BuildProviderRegistry::standard`;
3. extend configuration validation (a new enum variant and a new accepted
   spelling);
4. add provider-specific tests.

No change to the manager, the task engine, the application core, the TUI, or the
MCP is required.

### Task-engine integration

Every build is a task. `BuildTaskExecutor` confirms the task is a build kind,
decodes the workspace-aware `ResolvedBuildTarget` from its parameters, applies
the task timeout and cancellation token, delegates to `BuildManager`, and maps
the structured `BuildOutcome`/`BuildError` back to the engine.
`register_build_executors` registers one executor against `BuildService`,
`BuildLibrary`, `MavenBuild`, `NpmBuild`, and `NpmInstall`.

Unlike Git, there is no `BuildManager` port in the application: a build has no
read-only query that an interface would call directly, so the build boundary is
only the `ResolvedBuildTarget` value a task carries (`docs/application-core.md`).
There is exactly one code path, reached through the engine.

### A documented dependency exception

The Build adapter depends on `execution` (to implement a `TaskExecutor`) and on
`process` (to run build tools), in addition to `application` and `domain`. This
is the same infrastructure-to-infrastructure exception ADR 007 recorded for
Git, applied deliberately and recorded again here. The edge is legitimate
because the adapter is the concrete executor for build tasks and the concrete
client of the process manager; it points outward from the domain and neither
`process` nor `execution` depends on `build`.

## Alternatives considered

1. **One Build Manager with a `match` on the build system.**
   Rejected: it couples orchestration to every build system, so each new system
   edits the manager. The provider abstraction confines build-system knowledge to
   one implementation per system.

2. **A generalized plugin framework (dynamic loading, discovery, lifecycle).**
   Rejected as over-engineering (`tasks/008.md`). A `BTreeMap<BuildSystem, Arc<dyn
   BuildProvider>>` is all the indirection the problem needs.

3. **Let providers spawn processes and schedule tasks.**
   Rejected: it violates the single-process-owner rule and would leave builds
   outside the engine's concurrency, cancellation, and supervision
   (`AGENTS.md` §15, §9).

4. **Emit shell strings instead of structured commands.**
   Rejected: values from configuration would be re-interpreted by a shell,
   reintroducing an injection surface (`AGENTS.md` §32). Plans are program plus
   typed arguments.

5. **Add a `Gradle` variant now.**
   Rejected: an unimplemented variant would appear operational and let a
   configuration select a system that cannot build it (`tasks/008.md`).

6. **Map Maven `install` to `InstallDependencies`.**
   Rejected: Maven's `install` publishes the module to the local repository,
   which is `PublishLocalArtifact`; conflating it with dependency resolution
   would erase a real semantic difference.

7. **Declare a `BuildManager` trait in the application, mirroring Git.**
   Rejected: all build operations are tasks; there is no read-only build query
   for an interface to call directly, so a port would be an unused abstraction.

8. **Carry environment values in the plan.**
   Rejected: a plan may be logged or surfaced through a task, and configured
   environment variables can be secrets (`AGENTS.md` §31). Plans carry
   requirement *names*; values come from the developer's process environment.

9. **Record every provider's expected outputs.**
   Rejected: npm's output location is project-specific and not declared by npm,
   so the npm provider records none rather than guessing. Providers differ where
   the truth differs.

10. **Give the Build Manager its own concurrency limit.**
    Rejected: the task engine's `max_parallel_tasks` is the only global bound
    (`AGENTS.md` §41). The adapter adds no pool.

## Consequences

### Positive

- Build-system knowledge is isolated in providers; the manager, engine,
  application, and interfaces are build-system agnostic.
- Adding a build system is a localized change, proven by the fact that npm
  required no change to the manager or the engine.
- Providers are pure and deterministic, so capability handling, executable
  resolution, and plan generation are unit-tested without running a build tool.
- Unsupported operations fail fast with a stable code instead of being
  approximated.
- Every build runs as a supervised, cancellable, timeout-bounded process through
  the single process owner.
- The configuration schema is open to Gradle without breaking existing files.

### Negative / trade-offs

- The `build -> execution` and `build -> process` edges are a second explicit
  infrastructure-to-infrastructure exception; it must be applied deliberately to
  future adapters, not copied casually.
- The operation vocabulary is intentionally small; a build capability that does
  not fit an existing operation needs a new operation (and provider support)
  rather than an ad-hoc command.
- Cancellation and timeouts are enforced at the process boundary, so a build tool
  may complete part of its work before it observes a stop; there is no rollback.
- Environment enforcement only covers *required* variables declared by a plan;
  anything else is left to the build tool's own diagnostics.

## Follow-up

- Implement the Liquibase, health, and service-start adapters on the same
  provider/executor pattern.
- Add `GradleBuildProvider` when a Gradle service appears: implement, register,
  extend validation, and test — no orchestration change.
- Surface a build's expected outputs and bounded logs in the TUI and MCP strictly
  through the application use cases.
- Reuse `BulkOperationSummary` for multi-entity build workflows.

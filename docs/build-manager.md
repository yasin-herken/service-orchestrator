# The Build Manager

The Build Manager is the infrastructure adapter that turns a validated build
target into a structured build plan and executes it. It is **build-system
agnostic**: Maven is the primary implementation and npm the second, but nothing
in orchestration is coupled to either. It lives in the
`service-orchestrator-build` crate.

A build always runs as a task. The application resolves a workspace-aware
`ResolvedBuildTarget`, submits a task carrying it, and the engine invokes the
`BuildTaskExecutor`; the executor delegates to the `BuildManager`, which selects
a provider, plans, and runs the plan through the process manager. The adapter
never spawns an operating-system process itself and never depends on Ratatui or
MCP.

```text
Application use case
       │  (build op as a TaskDefinition with target parameters)
       ▼
Task Engine ──> BuildTaskExecutor ──┐
                                    ▼
                              BuildManager
                       ┌──────────┴──────────┐
                       ▼                     ▼
             BuildProviderRegistry       BuildRunner
                       │                     │
         ┌─────────────┴─────────────┐       ▼
         ▼                           ▼  Process Manager
  MavenBuildProvider          NpmBuildProvider    │
         └──────── BuildPlan ────────┘             ▼
                                              build tool
```

> This task implements the Build Manager only. Liquibase, service startup, the
> TUI, and MCP are intentionally **not** implemented here.

---

## 1. What the manager is not

- It is **not** a business layer. Provider selection is driven by validated
  configuration, and precondition checks (build capability, build command,
  dependency order) live in the application. The manager contains no
  per-interface behavior and no service-specific branching.
- It is **not** a second concurrency framework. It adds no pool of its own; the
  task engine's `max_parallel_tasks` is the only global bound (`AGENTS.md` §41).
- It is **not** a configuration loader. Targets arrive as an already validated,
  workspace-aware `ResolvedBuildTarget`; the manager never parses TOML.
- It is **not** a process owner. Every process is started by the process manager
  (`AGENTS.md` §15).

---

## 2. The build target

Services are never addressed by a bare name: two workspaces may legitimately
contain a service with the same id. Every operation takes a `ResolvedBuildTarget`:

| Field | Meaning |
| --- | --- |
| `workspace_id` | The owning workspace. |
| `name` | The entity identifier (a service or library id). |
| `build_system` | The selected build system (`maven` or `npm`). |
| `operation` | The build-system-neutral operation to perform. |
| `path` | The entity's absolute working directory. |
| `workspace_root` | The workspace root; the working directory must stay inside it. |
| `command` | The configured command preferred for this operation, if any. |

`ResolvedBuildTarget::is_inside_workspace` is checked before anything runs; a
target outside its workspace fails with `BUILD_INVALID_TARGET`. The value
round-trips through the task engine via `to_parameters`/`from_parameters`, so a
build task submitted by the TUI or MCP can be executed later without re-reading
configuration.

Environment variable *values* are deliberately not part of the target: task
parameters may be surfaced through the interfaces, so embedding configuration
values there would risk leaking a credential (`AGENTS.md` §31).

---

## 3. Build operations and capabilities

Operations are build-system-neutral names whose semantics are genuinely shared.
Providers declare which they support; an unsupported request is rejected with
`BUILD_UNSUPPORTED_OPERATION` before a process starts.

| Operation | Maven | npm |
| --- | :---: | :---: |
| `Clean` | ✅ (`mvn -B clean`) | — |
| `Build` | ✅ (`mvn -B package`) | ✅ (`npm run build`) |
| `Test` | ✅ (`mvn -B test`) | ✅ (`npm test`) |
| `InstallDependencies` | — | ✅ (`npm install`) |
| `PublishLocalArtifact` | ✅ (`mvn -B install`) | — |

Maven does not declare `InstallDependencies` because its `install` phase
*publishes* to the local repository — that is `PublishLocalArtifact`. npm does
not declare `Clean` (no standard goal) or `PublishLocalArtifact` (a
Maven-repository concept). The capability sets are the providers' honest
declarations, not a lowest common denominator.

### Task kinds

| Task kind | Operation |
| --- | --- |
| `BuildService`, `BuildLibrary`, `MavenBuild`, `NpmBuild` | `Build` |
| `NpmInstall` | `InstallDependencies` |

---

## 4. The build plan

Providers generate a `BuildPlan` rather than executing anything. A plan contains:

| Field | Purpose |
| --- | --- |
| `program` | The executable (`mvn`, `./mvnw`, `npm`, or a configured program). |
| `args` | The argument vector, passed verbatim and never through a shell. |
| `working_dir` | The absolute directory the build runs in. |
| `environment` | Environment variable *requirements*, by name only. |
| `timeout` | An optional maximum runtime declared by the provider. |
| `expected_outputs` | Locations the build is expected to produce, where known. |
| `system`, `operation` | What the plan targets. |

### Executable or wrapper resolution

The Maven provider prefers the project's Maven Wrapper: when `mvnw` exists in
the module directory it runs `./mvnw`, otherwise it falls back to `mvn` from
`PATH`. This is the provider's only filesystem observation, and it is asked
through the `BuildHost` seam.

### Configured commands win

When configuration declares a command for the requested operation, the provider
uses it verbatim — program plus arguments, never a shell line (`AGENTS.md` §32) —
instead of a build-system default. Only the *default* plan needs build-system
knowledge, which is what lets provider-independent orchestration be tested with
a trivial local command.

### Environment requirements, not values

A plan may declare that a build expects an environment variable (for example
Maven prefers `JAVA_HOME`). Only the name and whether it is required are
recorded. The runner sets no variables: the build inherits the developer's
process environment. The manager rejects, before starting a process, a plan
whose *required* variables are absent (`BUILD_INVALID_TARGET`).

---

## 5. Provider selection

`BuildProviderRegistry` is a small resolver — a map from `BuildSystem` to
`BuildProvider`, no discovery and no lifecycle:

- `BuildProviderRegistry::standard()` registers Maven and npm;
- `resolve(system)` returns the provider or `BUILD_UNSUPPORTED_SYSTEM`;
- the manager then checks `supports(operation)` and rejects an unsupported
  operation with `BUILD_UNSUPPORTED_OPERATION`.

Adding a build system (Gradle, for example) is: implement `BuildProvider`,
register it in `standard()`, extend configuration validation, and add tests. The
manager, the engine, the application core, and the interfaces are unchanged.

---

## 6. Structured errors

Every failure is a `BuildError` variant with a stable, machine-readable `code()`
and enough context (entity, system, operation, exit code) to explain what
happened — without leaking credentials.

| Code | Meaning |
| --- | --- |
| `BUILD_UNSUPPORTED_SYSTEM` | No provider is registered for the build system. |
| `BUILD_UNSUPPORTED_OPERATION` | The provider does not support the operation. |
| `BUILD_INVALID_TARGET` | The target is invalid, escapes its workspace, or a required environment variable is missing. |
| `BUILD_COMMAND_FAILED` | The build exited non-zero. |
| `BUILD_TIMEOUT` / `BUILD_CANCELLED` | The build timed out or was cancelled. |
| `BUILD_PROCESS_FAILED` | The process manager refused or failed to run the plan. |
| `BUILD_MISSING_PARAMETER` / `BUILD_INVALID_PARAMETER` | A task parameter was missing or malformed. |
| `BUILD_INTERNAL_ERROR` | An internal manager failure. |

Failures are bounded (standard error is truncated to a fixed limit) and mapped
to the engine as `TaskExecutorError::Failed { code, message, detail }`; a
cancelled build becomes `TaskExecutorError::Cancelled`.

---

## 7. Task-engine integration

- `BuildTaskExecutor` confirms the task is a build kind, decodes the
  `ResolvedBuildTarget` from the task's parameters, applies the task timeout and
  cancellation token, delegates to `BuildManager`, and maps the structured
  result back to a `TaskOutcome` or `TaskExecutorError`. It contains no build
  logic, so the TUI and MCP reach identical behavior through the same engine
  (`AGENTS.md` §21).
- `register_build_executors` registers one shared executor against every build
  `TaskKind`. The composition root hands the finished registry to the engine via
  `TaskEngineBuilder::registry`.
- Cancellation is cooperative. The task's `Cancellation` is passed to the
  process manager through `spawn_with_cancellation`, so a cancelled task stops
  the running build gracefully (`ExitReason::Cancelled` → `BUILD_CANCELLED` →
  `TaskExecutorError::Cancelled`).
- There is no `BuildManager` port in the application: every build is a task, and
  the only boundary is the `ResolvedBuildTarget` value a task carries.

---

## 8. Configuration

Build-system selection is configuration-driven. A service or library may declare
`build_system = "maven"` or `build_system = "npm"`; when it is omitted the
system is inferred:

- from a `maven`/`npm` runtime requirement, then
- from the program of the build/install/test commands (`mvn`/`mvnw` → Maven;
  `npm`/`npx`/`yarn`/`pnpm` → npm), and
- for a library, from its `type` first (a `maven` library defaults to Maven).

An explicit value is authoritative and validated. Gradle is intentionally not
accepted yet: it is planned, but listing it would let a configuration select a
system that cannot build it. See `docs/configuration.md` §5.6.

---

## 9. Testing

Every test is local and offline; no Maven, npm, GitHub, or network is involved
(`tasks/008.md`).

- `crates/build/tests/support/mod.rs` provides a temporary workspace and a
  controlled `BuildHost`.
- `crates/build/tests/providers.rs` covers provider selection, unknown-provider
  rejection, unsupported-operation handling, plan generation (including wrapper
  resolution), configured-command authority, and proof that planning never
  spawns a process.
- `crates/build/tests/manager.rs` runs tiny local utilities (`echo`, `false`,
  `sleep`) to exercise the manager's classification of success, non-zero exit,
  timeout, cancellation, and unsupported operations.
- `crates/build/tests/task_engine.rs` covers the whole adapter flow and shows
  that the *same* path serves a Maven target and an npm target — the
  provider-independent orchestration requirement.

---

## 10. Limits and future work

- Gradle is planned but not implemented; the abstraction and the configuration
  schema are ready for it.
- The operation vocabulary is intentionally small. A capability that does not fit
  an existing operation needs a new operation and provider support.
- Cancellation and timeouts are enforced at the process boundary; there is no
  atomic rollback of a partially completed build.
- Only *required* environment variables are enforced by the manager; everything
  else is left to the build tool's diagnostics.
- Liquibase, health, and service-start adapters will reuse the same provider and
  executor patterns.

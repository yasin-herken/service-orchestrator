# Builds

Service Orchestrator builds services and libraries through a **build-system
agnostic** manager. Maven is the primary implementation and npm the second; the
manager selects a `BuildProvider` from the configured build system and runs a
structured build plan through the process manager. The design is in
[`build-manager.md`](build-manager.md).

Gradle is **planned but not implemented** — a `build_system = "gradle"` in
configuration is rejected at validation time.

## 1. Selecting a build system

```toml
build_system = "maven"        # maven | npm   (optional)
```

- When `build_system` is omitted it is **inferred**: a `maven`/`npm` runtime is
  consulted first, then the programs of the build-related commands
  (`mvn`/`mvnw` → Maven; `npm`/`npx`/`yarn`/`pnpm` → npm). For a library, its
  `type` is consulted first.
- An explicit `build_system` is authoritative and validated; an unknown value is
  an error rather than a silent fallback.
- If nothing is recognizable, the entity simply has no known build system.

In the **GUI** Builds screen, the rest of the build system is the interesting
part: the core tells the screen which operations each system supports
(its *capabilities*), and the screen disables what is not supported.

## 2. Commands

```toml
[services.commands]
install = "./mvnw dependency:resolve"
build   = "./mvnw clean install"

[libraries.commands]
build   = "./mvnw clean install"
```

Commands are structured: a string is tokenized **without a shell**, and a
`sh -c "…"` command is rejected at validation time. A structured command is:

```toml
build = { program = "./mvnw", args = ["clean", "install"], working_dir = "backend/auth-service" }
```

- **Maven Wrapper.** A `./mvnw` command uses the project's wrapper, so the
  workspace does not need a globally installed Maven.
- **npm.** `npm install`, `npm run build`, etc. Node.js is required on `PATH`.

## 3. Build operations

Providers declare a subset of the build-system-neutral operations: `Clean`,
`Build`, `Test`, `InstallDependencies`, `PublishLocalArtifact`. An unsupported
request fails fast with `BUILD_UNSUPPORTED_OPERATION` rather than being
approximated.

Every build is a **task**, submitted to the shared engine and bounded by
`execution.max_parallel_tasks`.

| Operation | TUI | GUI | MCP |
| --- | --- | --- | --- |
| Build one service | `b` | Builds → Build | `build_service { serviceId }` |
| Build selection | `B` | bulk Build | (call `build_service` per service) |
| Build a library | Libraries → `b` | Libraries → Build | `build_library { libraryId }` |
| Build the whole workspace | — | — | `build_workspace { workspaceId }` |

> There is no combined bulk-build use case in MCP, so `build_workspace` builds
> everything or you call `build_service` per service. The **TUI** runs bulk build
> per service and reports a scheduled/failed count. The **GUI** bulk build is
> submitted as one `prepare_environment` workflow restricted to the build phase.

## 4. Dependency installation

Dependency installation is part of a build for Maven (`install`) and npm
(`npm install`). There is deliberately **no standalone `install_dependencies`
operation**: a service installs through its build, a library through
`install_library` / `prepare_library` ([`libraries.md`](libraries.md)).

## 5. Runtime and version requirements

- A `java` or `node` runtime declaration must pin a version; the runtime adapter
  can check the installed version and reports `RUNTIME_VERSION_MISMATCH` when it
  differs.
- The project's stated Node.js version for managed frontend services is
  **20.x**.
- `validate_workspace` reports a missing `mvn`/`mvnw`/`npm` as
  `missing_build_tool`.

## 6. Build logs and failures

A build's `stdout`/`stderr` are captured by the process manager and ingested into
the unified log store with source `build`.

- **TUI** — Builds screen, `l` for logs; or the Tasks screen, `Enter` for task
  logs.
- **GUI** — the build target's log view, or the Tasks screen.
- **MCP** — `get_task_logs { taskId }` or
  `get_service_logs { serviceId, source: "build" }`.

Common build error codes:

| Code | Meaning | Action |
| --- | --- | --- |
| `BUILD_COMMAND_FAILED` | The build tool exited non-zero. | Read the build logs; fix the project. |
| `BUILD_UNSUPPORTED_OPERATION` | The provider does not support the operation. | Use a supported operation. |
| `BUILD_UNSUPPORTED_SYSTEM` | No provider for the configured system (Gradle). | Use Maven or npm. |
| `BUILD_INVALID_TARGET` / `BUILD_PATH_OUTSIDE_WORKSPACE` | The resolved target is invalid or escapes the root. | Fix the service `path`. |
| `BUILD_TIMEOUT` | The build exceeded its timeout. | Raise `execution.default_timeout_seconds` or the operation timeout. |
| `BUILD_CANCELLED` | The build was cancelled. | Rerun when ready. |
| `BUILD_SYSTEM_UNKNOWN` | The entity has no known build system. | Declare `build_system` or a recognizable command/runtime. |

A failed build never claims success, and its logs are kept for inspection.

## 7. Cancellation

Builds are tasks. A queued build can be cancelled immediately; a running build is
asked to cancel cooperatively, and the tool's process group is stopped. A
cancelled build reports `BUILD_CANCELLED`. Cancellation never guarantees that a
partially written artifact was cleaned up.

## 8. Limitations

- Gradle is not implemented.
- Builds are **not sandboxed**: a Maven plugin or npm script can execute
  arbitrary project code ([`security.md`](security.md) §6).
- There is no separate clean/test service action in the TUI/GUI; those are
  internal phases of the provider.

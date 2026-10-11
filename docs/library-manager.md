# The Library Manager

The Library Manager manages shared development libraries and their relationships
with the services that consume them. It is **application-layer orchestration**,
not a new infrastructure crate: it validates library configuration, resolves an
artifact against the effective build system, composes the existing Git and build
tasks into dependency-ordered workflows, and submits them to the task engine. It
never runs a build tool, a Git command, or a filesystem write itself.

```text
TUI / MCP / CLI
       │
       ▼
Application Core  ──> Library Manager (use cases + ArtifactInspector port)
       │                        │
       │            ┌───────────┼───────────────┐
       │            ▼           ▼               ▼
       │        Git tasks   Build tasks   ArtifactVerification task
       │            │           │               │
       │            ▼           ▼               ▼
       │        Git adapter  Build Manager  ArtifactInspector
       │                         │           (Maven, in `build`)
       │                         ▼
       │                   BuildProvider ──> Process Manager
       ▼
Task / Workflow Engine
```

The Maven-specific pieces — the effective local repository probe and the
conservative POM reader — live behind the `ArtifactInspector` port in the `build`
crate. Everything above that boundary is build-system-independent.

> This task implements the Library Manager and its application integration only.
> Liquibase, service startup, Ratatui screens, MCP tools, and a Gradle provider
> are intentionally **not** implemented here (`tasks/009.md` §31).

---

## 1. What the Library Manager is not

- It is **not** a build tool. Every build and install runs through the Build
  Manager (`docs/build-manager.md`); the Library Manager only chooses *what* to
  build and in *what order*.
- It is **not** a Git implementation. Synchronization and checkout reuse the Git
  task kinds; the Library Manager never resets, cleans, or discards changes.
- It is **not** a Maven dependency resolver. It never downloads or resolves a
  dependency graph and never copies a JAR into a repository. Artifact *presence*
  is checked with the standard Maven layout in the effective local repository.
- It is **not** a second scheduler or task registry. Ordering is expressed as
  workflow-step dependencies and every operation is a task in the shared engine
  (`tasks/009.md` §19, §10).
- It is **not** a configuration system. Library configuration lives in the
  existing versioned configuration and is validated there.

---

## 2. The library model

A library is a first-class entity (`docs/domain-model.md`), not a kind of
service. It carries no name-based behaviour.

| Field | Meaning |
| --- | --- |
| `id` | The stable library identifier. |
| `display_name` | The human-readable name. |
| `repository` | The source repository, when the library has one. |
| `path` | The module directory, relative to the workspace root. |
| `artifact` | The **primary** published coordinate, when known. |
| `artifacts` | Additional coordinates a reactor project produces. |
| `build_system` | `maven` or `npm`; inferred from `type`/runtimes/commands when omitted. |
| `dependencies` | Typed references that must target **other libraries**. |
| `runtimes`, `commands`, `environment` | Build toolchain and command descriptions. |

A library may depend only on other libraries; a dependency on a service is a
validation error. The domain's `Library::produced_artifacts()` returns every
artifact a library produces, primary first, with duplicates removed.

---

## 3. Artifact coordinates

A Maven artifact is identified by its **coordinate**, never by the repository
that produces it (`tasks/009.md` §4):

```text
groupId:artifactId:version          com.company:common-core:1.8.0-SNAPSHOT
```

Packaging and classifier are optional attributes that affect the artifact's
*file name*, not its identity:

```toml
[[libraries]]
id = "common-core"
type = "maven"
path = "libraries/common-core"

[libraries.artifact]
group_id = "com.company"
artifact_id = "common-core"
version = "1.8.0-SNAPSHOT"
packaging = "jar"
# classifier = "sources"
```

`ArtifactCoordinate` is pure domain vocabulary: it computes the standard Maven
repository layout from an identity and performs no repository access.

- `group_path()` — `com.company` → `com/company`
- `file_name()` — `common-core-1.8.0-SNAPSHOT.jar` (honouring packaging and
  classifier)
- `repository_path()` — `com/company/common-core/1.8.0-SNAPSHOT/common-core-1.8.0-SNAPSHOT.jar`

`ResolvedArtifact` pairs that coordinate with the owning workspace, the library
id, and the module's absolute directory, and round-trips through task
parameters, so an inspection task can run later without re-reading
configuration. Only coordinates and paths are carried — never credentials or
Maven settings values (`AGENTS.md` §31).

---

## 4. Operations

All operations are exposed through the application core (`tasks/009.md` §26),
using the existing request/response conventions and returning structured values,
never pre-formatted strings.

| Operation | Kind | Behaviour |
| --- | --- | --- |
| `list_libraries(workspace_id)` | Query | The configured libraries in a workspace. |
| `get_library(library_id)` | Query | One library's configuration. |
| `get_library_status(library_id)` | Query (async) | Configuration plus a best-effort Git snapshot. |
| `build_library(library_id)` | Command | Schedules a `BuildLibrary` task (`Build`). |
| `install_library(library_id)` | Command | Schedules a `LibraryInstall` task (`PublishLocalArtifact`). |
| `prepare_library(library_id, options)` | Command | A `PrepareLibrary` workflow. |
| `prepare_selected_libraries(request)` | Command | A dependency-ordered `PrepareLibraries` workflow. |
| `verify_library_artifact(library_id)` | Query (async) | Whether the primary artifact is present. |
| `find_library_dependents(library_id)` | Query | The services that depend on the library. |
| `inspect_library_version_alignment(library_id)` | Query (async) | A structured version-alignment plan. |

### Get Library Status

`get_library_status` combines validated configuration with a best-effort Git
snapshot (branch, commit, `GitState`) read through the `GitManager`. A repository
that cannot be inspected yields `GitState::Unknown` and a structured
`repository_error` rather than failing the whole query. Artifact presence is
deliberately **not** guessed here; the definitive answer is
`verify_library_artifact`.

### Verify Artifact

`verify_library_artifact` resolves the effective local repository the way Maven
does and checks the standard layout for the primary coordinate. A missing
artifact is **not** an error: it is reported with `present == false`. An
inspection that cannot determine the repository at all is a structured
`ArtifactInspection` error.

### Find Dependents

`find_library_dependents` returns every service whose declared dependencies
target the library, with the dependency's `required` flag and kind.

---

## 5. Git Manager integration

Library repository operations reuse the Git task kinds; the Library Manager
never operates on a repository directly (`tasks/009.md` §16).

```text
Validate Repository
        │
        ▼
   GitFetch (GitSync)
        │
        ▼
 GitCheckout (optional branch)
        │
        ▼
   Build Library
        │
        ▼
   Install / Verify
```

- Synchronization is **optional**. When `options.sync` is `false`, the current
  local branch — including uncommitted development changes — is built as-is.
- A branch is checked out only when `options.branch` is explicitly set. The
  configured default branch is never forced, and `master` is never assumed
  (`tasks/009.md` §20).
- Local changes are preserved: the workflow never performs `reset`, `clean`, or a
  discard.

---

## 6. Build Manager integration

Library builds and installs go through the Build Manager, exactly like service
builds (`tasks/009.md` §17):

```text
Library Manager
      │   (BuildOperation + ResolvedBuildTarget)
      ▼
Build Manager ──> BuildProvider ──> Process Manager
```

- `build_library` and the build phase of `prepare_library` use
  `BuildOperation::Build`.
- `install_library` and the install phase use
  `BuildOperation::PublishLocalArtifact`. Maven performs it with its `install`
  phase; the Library Manager never copies a JAR into a repository.
- When configuration declares a command for the operation, it is used verbatim
  (program plus arguments, never a shell line). Otherwise the provider's default
  plan runs.
- **Provider independence.** The Library Manager asks for a
  build-system-neutral capability, not for Maven. A library whose build system
  cannot publish a local artifact fails fast with
  `ApplicationError::BuildProviderUnsupported`; the operation is never
  approximated. Gradle is **not** implemented in this task.

### Task kinds

| Task kind | Operation | Executor |
| --- | --- | --- |
| `BuildLibrary` | `Build` | `BuildTaskExecutor` (build crate) |
| `LibraryInstall` | `PublishLocalArtifact` | `BuildTaskExecutor` |
| `ArtifactVerification` | — (inspection) | `ArtifactVerificationExecutor` |
| `GitSync`, `GitCheckout` | — (Git) | `GitTaskExecutor` |

`LibraryInstall` is registered against the same build executor as the other
build kinds, because it *is* a build operation. `ArtifactVerification` is a
distinct read-only task handled by `ArtifactVerificationExecutor`. Every
long-running operation is a task and returns a task or workflow id.

---

## 7. Local artifact installation and the effective repository

Maven does not always use `~/.m2/repository`: the location can be overridden by
settings, `MAVEN_OPTS`, or command-line properties (`tasks/009.md` §7). The
`ArtifactInspector` therefore asks Maven for the effective value rather than
assuming a path:

```text
mvn -B -q -DforceStdout help:evaluate -Dexpression=settings.localRepository
```

The resolved value is cached per module directory; a failure is never cached,
and an undeterminable location fails explicitly with
`ARTIFACT_LOCAL_REPOSITORY_UNKNOWN` instead of guessing. `LocalRepositoryProvider`
is the seam: `MavenLocalRepositoryProvider` evaluates Maven's configuration
(optionally against a pinned settings file), and `FixedLocalRepository` serves
tests and callers that already know the path. There is **no custom Maven
dependency resolver**.

---

## 8. Multi-module (reactor) projects

A repository may produce more than one artifact (`tasks/009.md` §8):

```text
common-platform/
├── pom.xml
├── common-api/
├── common-core/
└── common-security/
```

- `Library.artifact` is the **primary** coordinate; `Library.artifacts` lists the
  rest. Configuration accepts an `[libraries.artifact]` block plus zero or more
  `[[libraries.artifacts]]` blocks.
- `Library::produced_artifacts()` returns them primary-first, de-duplicated.
- `get_library_status` reports every produced coordinate.

**MVP limitation.** Verification, install, and the version-alignment target use
the **primary** coordinate. The install itself is a single reactor build at the
module directory, so Maven installs every module it produces; the additional
coordinates are modelled and surfaced but not individually verified in this
task.

---

## 9. Dependency relationships and build ordering

The existing domain dependency graph is reused; no competing graph is created
(`tasks/009.md` §9). A library depends on libraries, and a dependent library
must be built after the libraries it consumes.

```text
common-utils
     │
     ▼
common-core
     │
     ├───────────┐
     ▼           ▼
auth-service  user-service
```

- Selecting libraries resolves the **transitive closure** of those requested,
  then orders it **dependency-first** with a topological sort in the
  application layer (the domain exposes only crate-private cycle detection).
- A dependency that is not configured fails with
  `ApplicationError::DependencyResolution`; a cyclic selection fails with
  `ApplicationError::DependencyCycle`.
- Independent libraries are scheduled with no prerequisites and may build
  concurrently; a dependent library's steps wait for its prerequisites. The
  ordering is expressed as workflow-step dependencies, so the **task engine
  remains the only scheduler** and `max_parallel_tasks` remains the only global
  concurrency bound (`tasks/009.md` §10, §23).

---

## 10. The Prepare Library workflow

`prepare_library` and `prepare_selected_libraries` build a workflow from the
enabled phases (`tasks/009.md` §20):

```text
Validate configuration
        │
        ▼
Git sync        (optional)
        │
        ▼
Checkout branch (optional)
        │
        ▼
Build dependencies ── (dependency-order edges)
        │
        ▼
Build library
        │
        ▼
Install locally
        │
        ▼
Verify artifact
```

- Options (`LibraryPrepareOptions`) default to synchronize, build, install, and
  verify. Each phase can be turned off; `sync = false` builds the local working
  tree, and `branch` is only set when the developer asks for it.
- An empty library selection means "every library in the workspace".
- Verification ends the workflow. Consumer version alignment is a separate,
  read-only query (`inspect_library_version_alignment`), kept out of the
  mutating workflow so it never blocks a build.

---

## 11. Local development workflow

The scenario from `tasks/009.md` §21 works end-to-end without pushing:

```text
Modify common-core locally
        │
        ▼
prepare_library(common-core, sync = false)
        │   build → install → verify
        ▼
inspect_library_version_alignment(common-core)
        │   aligned / mismatch / unknown / unresolved per consumer
        ▼
build auth-service
```

The developer is never forced to push library changes to a remote repository,
and never forced onto a particular branch.

---

## 12. Dependency version management

Installing an artifact locally does **not** change the version a consumer
requests (`tasks/009.md` §11). Version alignment is therefore explicit and
read-only.

A Maven dependency version may come from a direct element, a property, a parent
POM, dependency management, an imported BOM, or a reactor build (§12). The
inspector resolves only what it can read with certainty and never rewrites a POM
or performs text replacement:

| Source | Result |
| --- | --- |
| Literal `<version>` | `Declared` |
| `${property}` defined in the same POM | `Declared` |
| Direct dependency without a version + matching `dependencyManagement` | `Declared` |
| Property/BOM/parent the reader cannot resolve | `Unknown` |
| Managed but not declared as a dependency | `Unknown` |
| Absent, but a `<parent>` or imported BOM is present | `Unknown` |
| Absent and no parent/BOM | `NotDeclared` |

`inspect_library_version_alignment` maps each consumer to one of four statuses
(`tasks/009.md` §14):

| Status | Meaning |
| --- | --- |
| `Aligned` | The consumer resolves the target version. |
| `Mismatch` | The consumer resolves a different, concrete version. |
| `Unknown` | A version exists but could not be resolved reliably. |
| `Unresolved` | The consumer does not declare the artifact. |

A version is **never guessed**. A consumer that is not a Maven module, or that
has no workspace path, is reported `Unknown` without an inspection attempt. If
the inspector itself is unavailable, the query fails with
`ApplicationError::VersionInspectionUnsupported`; any other inspection failure
degrades only that consumer to `Unknown`.

### Safe overrides

Automatic POM modification is **not implemented**. It is deferred deliberately:
`tasks/009.md` §13 requires that any such modification be opt-in, dry-run-able,
non-destructive to unrelated content, authorized, reversible, and
integration-tested, and none of that is needed to satisfy the visibility-first
goal. The extension point for future local-development overrides (Maven property
overrides, development profiles, generated temporary settings, explicit version
workflows, `tasks/009.md` §15) is the read-only alignment plan above.

---

## 13. Operation coordination

Two operations that build the **same module** must not run at once: a build and
an install both write the module's `target` directory and the local repository,
and interleaving them can corrupt shared artifact state (`tasks/009.md` §23).
`ArtifactCoordinator` is a keyed write lock inside the Build Manager:

- the key is the module's absolute directory, so unrelated libraries and
  services still build concurrently; there is **no global lock**;
- cloning shares the lock table, so every task executor coordinates on the same
  keys;
- keying on the directory (rather than the coordinate) also covers reactor
  projects, where several coordinates share one directory.

The coordinator is not a scheduler: the task engine remains the only concurrency
authority.

---

## 14. Structured errors

Library failures are `ApplicationError` variants with enough context (library,
workspace, coordinate, requested version, task id, exit code) to explain what
happened, without leaking credentials (`tasks/009.md` §24).

| Error | Meaning |
| --- | --- |
| `LibraryNotFound` | The library is not configured. |
| `LibraryConfigurationInvalid` | A phase needs a path or artifact the library lacks. |
| `BuildProviderUnsupported` | No provider can perform the requested capability. |
| `ArtifactInspection { code, .. }` | An inspection failed; `code` is the stable inspector code. |
| `VersionInspectionUnsupported` | No inspector is available to resolve consumer versions. |
| `DependencyResolution` | A declared library dependency is not configured. |
| `DependencyCycle` | The selected libraries form a cycle. |

Artifact/task failures carry stable codes: `ARTIFACT_NOT_FOUND` (a verified
artifact was missing during a workflow), `LIBRARY_BUILD_FAILED` (a library
build failed), `ARTIFACT_INSTALLATION_FAILED` (a local install failed),
`ARTIFACT_LOCAL_REPOSITORY_UNKNOWN`, `ARTIFACT_IO_ERROR`, and
`ARTIFACT_INSPECTION_UNAVAILABLE`.

---

## 15. Logging

Library operations produce structured output through the task engine: each task
carries progress messages and a structured `TaskOutcome` with metadata
(library id, coordinate, artifact path, size), and failures carry a stable code
and detail. Credentials and Maven settings values are never logged
(`tasks/009.md` §25, `AGENTS.md` §31). Detailed logs remain available through the
existing task/log surfaces.

---

## 16. Safety rules

- No direct shell execution: every action is a structured task.
- No manual JAR copying; installation is a Maven build operation.
- No hard-coded `~/.m2/repository`; the effective location is discovered.
- No naive or silent POM rewriting; version alignment is read-only.
- No silent destructive Git operations; local changes are preserved.
- No forced branch; the configured default branch is never assumed.
- No global lock; coordination is scoped to the module directory.
- Unsupported capabilities fail fast with a structured error.

---

## 17. Testing

- `crates/application/tests/library_manager.rs` covers resolution,
  configuration validation, ordering, provider-capability checks, version
  alignment results, unknown versions, missing artifacts, dependents, and
  workflow composition — all offline, through a fake inspector and Git manager.
- `crates/build/tests/library.rs` covers the artifact-verification task kind and
  the coordination lock end-to-end through the engine.
- `crates/build/src/artifact.rs` unit tests cover the conservative POM reader and
  the fixed-repository inspector; `coordinator.rs` unit tests cover keyed locking.
- `crates/build/tests/maven_library.rs` are gated (`#[ignore]`) Maven
  integration tests. They run with `--ignored` against an **isolated temporary
  local repository** (via a pinned settings file), so the developer's real
  `~/.m2/repository` is never modified (`tasks/009.md` §28). They cover
  single-module install + verify, reactor install + per-artifact verify, a
  missing artifact, and the effective-local-repository probe.

---

## 18. Known limitations

- **Primary artifact only.** Multi-module libraries are modelled and listed, but
  verification, install, and alignment target the primary coordinate (§8).
- **Maven only.** No Gradle provider; npm libraries have no local-artifact
  install capability and are rejected with `BuildProviderUnsupported`.
- **Conservative version inspection.** Parent POMs, imported BOMs, and
  multi-hop properties are reported `Unknown`, never resolved.
- **No automatic version overrides.** POM modification is deferred (§12).
- **No aggregated workflow summary.** `prepare_selected_libraries` returns the
  scheduled `WorkflowScheduled` result; per-library outcomes are observed through
  the task engine.
- **No health or startup.** Building/installing a library does not start
  anything; service startup and health are future adapters.

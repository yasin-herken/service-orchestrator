# Domain model

This document describes the vocabulary defined by the `service-orchestrator-domain`
crate. The domain is the innermost layer of the platform and depends on no other
workspace crate (`docs/architecture.md`, `docs/adr/001-core-architecture.md`).
It defines *what things are*; it never defines *how they are done*.

The model is implemented in `crates/domain/src`. Every module there corresponds
to one concept or a small cluster of related concepts.

---

## 1. Design principles

1. **The domain is inert.** No Git, no processes, no filesystem access, no
   network, no scheduling. Those belong to infrastructure, the execution engine,
   and the policy engine.
2. **Behaviour is capability-driven, never name-driven.** There is no branch on
   a service's name anywhere. A service can be built because it declares the
   `Build` capability, not because it is called `auth-service`.
3. **Configuration is external.** Services, libraries, profiles, and groups are
   assembled from configuration and then validated. Nothing is hard-coded, and
   no branch name is hard-coded.
4. **Invalid values are hard to construct.** Identifiers, ports, versions,
   branches, and paths validate themselves at construction, so a downstream
   module cannot be handed an empty id or a path that escapes the workspace.
5. **Strong identifiers where they pay for themselves.** `ServiceId`,
   `TaskId`, and friends prevent swapping one string for another. We do not
   wrap every primitive.
6. **Secrets cannot leak by formatting.** Secret environment variables render as
   `***`; debug-printing or displaying a domain value cannot reveal a credential.
7. **No speculative abstractions.** The domain declares no `GitProvider`,
   `ProcessExecutor`, or `BuildExecutor` traits. Those boundaries are defined by
   the application layer when infrastructure is actually wired in, not now.

---

## 2. Relationship diagram

```text
Workspace
   │
   ├── Services ──────────────────────────────────────────────┐
   │      │                                                    │
   │      ├── kind: ServiceType + capabilities                │
   │      ├── repository ──> Repository ──> Branch            │
   │      ├── path ────────> RelativePath                     │
   │      ├── runtimes ────> Runtime ──> RuntimeKind, Version │
   │      ├── commands ────> CommandSet ──> CommandSpec       │
   │      │                                   │               │
   │      │                                   └── Environment │
   │      ├── dependencies ─> Dependency ─> DependencyTarget  │
   │      │                                      │   │        │
   │      │                                      │   └──> LibraryId
   │      │                                      └──────> ServiceId
   │      ├── health_checks > HealthCheck > HealthCheckKind  │
   │      │                                   (Process|Port|Http)
   │      ├── ports ──────> Port                             │
   │      └── environment > Environment > EnvironmentVariable│
   │                                                          │
   ├── Libraries <───────────────────────────────────────────┘
   │      ├── repository ──> Repository
   │      ├── artifact ────> ArtifactCoordinate ──> Version
   │      ├── runtimes ────> Runtime
   │      ├── commands ────> CommandSet
   │      └── dependencies > Dependency (libraries only)
   │
   ├── Groups ──> ServiceGroup ──> ServiceId (references)
   │
   └── Profiles ─> Profile ──> ServiceId | GroupId (references)

Workflow ──> WorkflowStep ──> TaskKind
                └── depends_on ──> StepId (references)

Task ──> TaskKind, TaskStatus, TaskProgress, TaskFailure
   ├── service? ──> ServiceId
   ├── library? ──> LibraryId
   ├── workflow? ─> WorkflowId
   └── dependencies > TaskId

State / risk value objects (no ownership):
   ProcessState   GitState   Permission
```

Reading the diagram: solid containment lines are ownership (`Workspace` owns its
children); identifiers are references to other entities in the aggregate; the
bottom row are standalone value objects carried by tasks and reported to
interfaces.

---

## 3. The workspace aggregate

`Workspace` is the root aggregate. It owns the services, libraries, groups, and
profiles of one developer environment and is the only place where cross-cutting
invariants are checked.

`Workspace::validate` enforces:

- a non-empty workspace name;
- unique identifiers for services, libraries, groups, and profiles;
- every child entity is individually valid;
- every dependency, group membership, and profile reference resolves to an
  entity inside the workspace;
- the combined service + library dependency graph is acyclic.

Owning the whole set is what makes the last two checks possible. Configration
loading (`config`) builds the `Workspace`; the domain validates it.

---

## 4. Service

`Service` is the central entity: a buildable and/or runnable unit of software.

| Field | Meaning |
| --- | --- |
| `id` | Stable [`ServiceId`]. |
| `display_name` | Human-readable name for interfaces. |
| `kind` | `ServiceType`: `Backend`, `Frontend`, `Library`, `Other`. |
| `repository` | Optional `Repository` (URL, local path, default branch, provider). |
| `path` | Optional workspace-relative `RelativePath`. |
| `runtimes` | Required `Runtime`s (for example Java 21, Node 20.19.4). |
| `commands` | `CommandSet` of structured commands. |
| `dependencies` | `Dependency` references to services and libraries. |
| `health_checks` | `HealthCheck`s to run against the service. |
| `ports` | `Port`s the service uses. |
| `environment` | `Environment` variables (with secret redaction). |
| `capabilities` | `ServiceCapability` set that drives behaviour. |

**Invariants** (`Service::validate`): non-empty name; valid commands; no
duplicate dependency target; no self-dependency; no duplicate port; no
duplicate health check; every port health check targets a declared port; a
service must not declare `ServiceType::Library` (that is what `Library` is for).

**Capabilities, not names.** `ServiceType` supplies default capabilities
(`Backend` → build/run/migrate, `Frontend` → build/run, `Library` →
build/artifact), and configuration may add more. Generic orchestration asks
`service.supports(ServiceCapability::Run)`, never `service.name == "…"`.

**Why `ServiceType::Library` exists.** It is deliberately not a working type; a
configuration that classifies a service as a library is rejected with a clear
message, pointing the author at the dedicated `Library` entity.

---

## 5. Library

`Library` is a shared, buildable artifact consumed by services
(`AGENTS.md` §17). It mirrors the parts of `Service` that make sense for an
artifact: repository, path, runtimes, commands, and dependencies.

**Invariants** (`Library::validate`): non-empty name; valid commands; no
duplicate or self dependency; **libraries may only depend on libraries** (a
library that needs a running service is a modelling error).

`ArtifactCoordinate` (`com.company:common-core:1.8.0-SNAPSHOT`) records the
published artifact when known.

---

## 6. Repository, Runtime, and Command

These three describe requirements without prescribing an implementation.

- **`Repository`** — `url`, `local_path`, `default_branch: Branch`, `provider`.
  The domain rejects URLs that embed credentials; authentication is supplied by
  the environment, never stored in configuration. `default_branch` is
  configurable: the domain never assumes `master`.
- **`Runtime`** — a `RuntimeKind` (`Java`, `Node`, `Maven`, `Npm`, `Other`) and
  an optional `Version`. It says what is *needed*, not how it is discovered.
- **`CommandSet` / `CommandSpec`** — structured commands: a `program` and an
  argument vector, never a shell string. `CommandSet` is an ordered list of
  `CommandKind`/`CommandSpec` pairs, which serializes cleanly to JSON. A helper,
  `CommandSpec::invokes_shell`, flags the rare `sh -c` case for policy scrutiny.

`Environment` carries variables and marks secrets; secret values always render
as `***`.

---

## 7. Dependencies

`Dependency` links one entity to another through a `DependencyTarget`
(`Service(id)` or `Library(id)`) and a `DependencyKind` (`Build` or `Runtime`),
with a `required` flag.

The domain models dependencies and validates that they resolve and are acyclic.
**It does not schedule them.** Turning a dependency graph into an execution order
is the execution engine's responsibility.

---

## 8. Task and workflow

A `Task` is the domain representation of a unit of work:

- `id: TaskId`, `kind: TaskKind`, `status: TaskStatus`;
- optional `service`, `library`, and `workflow` associations;
- `created_at`, `started_at`, `finished_at` (`Timestamp`);
- `progress: TaskProgress`, `error: Option<TaskFailure>`;
- `dependencies: Vec<TaskId>`.

`TaskStatus` is a state machine, enforced by `Task::transition`:

```text
queued  ──> running ──> succeeded
   │  │        │    └─> failed
   │  │        └──────> cancelled
   │  ├──────> blocked ──> queued | running | failed | cancelled | skipped
   ├─────────> cancelled
   └─────────> skipped
```

Terminal states (`succeeded`, `failed`, `cancelled`, `skipped`) never change.
Timestamps are recorded automatically on `running` and terminal transitions.

A `Workflow` composes `TaskKind`s as `WorkflowStep`s, each with an optional
description and a set of prerequisite `StepId`s. `Workflow::validate` requires at
least one step, unique step ids, known prerequisites, and an acyclic graph. It
does **not** schedule; retry, timeout, conditions, and concurrency are the
execution engine's concern.

---

## 9. Profiles and groups

- `ServiceGroup` — a named collection of `ServiceId`s; non-empty and free of
  duplicates. Group actions reuse the same service workflows.
- `Profile` — a named developer environment referencing `ServiceId`s and
  `GroupId`s; it must reference at least one entity and must not duplicate
  references.

Both reference existing definitions rather than copying them; `Workspace::validate`
checks that every reference resolves.

---

## 10. Health checks, process state, and Git state

- `HealthCheck` — a `HealthCheckKind` (`Process`, `Port(Port)`, `Http`, or
  `Other`) plus optional `timeout`/`interval`. The model is open to future
  Docker, Kubernetes, and Spring Actuator checks without changing consumers.
- `ProcessState` — `NotRunning`, `Starting`, `Running`, `Stopping`, `Stopped`,
  `Failed`, `Unknown`. A running process does **not** imply a healthy service.
- `GitState` — `Clean`, `Modified`, `Ahead`, `Behind`, `Diverged`, `Unknown`,
  with helpers such as `has_local_changes` that the policy layer relies on
  before destructive operations.

None of these carry OS- or library-specific data (no PIDs, no signals, no
`git2` types).

---

## 11. Permission

`Permission` is the shared risk vocabulary used by both the TUI and MCP:

```text
Read         list_services, git_status, health_check      (no confirmation)
SafeWrite    git_pull, build_service, start_service       (no confirmation)
Destructive  git_reset, git_clean, delete_workspace       (confirmation required)
```

The variants are ordered by increasing risk, so `most_restrictive` returns the
stricter of two permissions. The policy engine (`policy` crate) applies these
rules; the domain only defines the contract.

---

## 12. Cross-cutting concerns

### Strong identifiers

`ids.rs` generates `ServiceId`, `LibraryId`, `WorkspaceId`, `TaskId`,
`WorkflowId`, `ProfileId`, `GroupId`, and `StepId`. Each validates itself:
non-empty, at most 128 characters, and limited to ASCII letters, digits, and
`-`, `_`, `.`, `:`. Distinct newtypes prevent swapping one id for another.

### Time

`Timestamp` is a single `serde`-transparent wrapper over `time::OffsetDateTime`,
ensuring the domain never scatters `SystemTime`, `i64`, or local times.
`Timestamp::now()` is the only clock read in the domain.

### Serialization

Domain values cross the configuration, TUI, and MCP boundaries, so the
configuration- and state-shaped types derive `serde::{Serialize, Deserialize}`.
`serde` is a compile-time-only concern and adds no runtime, network, or OS
dependency. `CommandSet` is represented as a list (not a map keyed by an enum)
so it serializes to clean JSON.

### Errors

`DomainError` is small and flat: invalid identifiers, validation failures,
not-found and duplicate entities, illegal task transitions, dependency cycles,
and invalid commands or paths. Application, infrastructure, and interface errors
live in their own layers; the domain does not define a global enum.

---

## 13. Intentionally not modelled yet

The following are deliberately absent from this task and belong to later work:

- **Git behaviour** — no fetch/checkout/pull/reset. Only `Repository` data and
  `GitState`.
- **Process management** — no spawning, termination, PIDs, or child processes.
  Only `ProcessState`.
- **Build tooling** — no Maven, npm, or Liquibase execution. Only structured
  commands.
- **Scheduling** — no scheduler, executor, retry, or concurrency. Only acyclic
  dependency graphs and a task state machine.
- **Policy engine** — only the `Permission` vocabulary; no rules.
- **Interfaces** — no TUI screens, MCP tools, resources, or prompts.
- **Runtime discovery** — no executable resolution, PATH handling, or version
  probing; `Runtime` only states requirements.
- **Kubernetes and Docker** — no container or cluster concepts.

---

## 14. Module map

```text
crates/domain/src/
├── lib.rs          crate docs and re-exports
├── error.rs        DomainError
├── ids.rs          strongly typed identifiers
├── timestamp.rs    Timestamp
├── paths.rs        RelativePath, AbsolutePath
├── port.rs         Port
├── environment.rs  EnvironmentVariable, Environment (secret redaction)
├── command.rs      CommandKind, CommandSpec, CommandSet
├── dependency.rs   Dependency, DependencyKind, DependencyTarget
├── repository.rs   Repository, RepositoryProvider, Branch
├── runtime.rs      Runtime, RuntimeKind, Version
├── health.rs       HealthCheck, HealthCheckKind, HttpHealthCheck
├── service.rs      Service, ServiceType, ServiceCapability
├── library.rs      Library, ArtifactCoordinate
├── group.rs        ServiceGroup
├── profile.rs      Profile
├── workspace.rs    Workspace and cross-cutting validation
├── task.rs         Task, TaskKind, TaskStatus, TaskProgress, TaskFailure
├── workflow.rs     Workflow, WorkflowKind, WorkflowStep
├── process.rs      ProcessState
├── git.rs          GitState
├── permission.rs   Permission
└── graph.rs        private cycle-detection helper
```

Related documents: `docs/architecture.md`, `docs/adr/001-core-architecture.md`,
`docs/adr/002-domain-model.md`.

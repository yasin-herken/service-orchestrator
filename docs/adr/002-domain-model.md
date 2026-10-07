# ADR 002: The core domain model

- **Status:** Accepted
- **Date:** 2026-10-07
- **Deciders:** Service Orchestrator maintainers

## Context

Service Orchestrator needs a shared vocabulary before any Git, build, process,
or interface work can proceed. Every later module — the task engine, the policy
engine, configuration loading, the Git and process adapters, the TUI, and the
MCP server — must agree on what a *service*, a *library*, a *task*, a
*dependency*, and a *permission* are.

Two failure modes are easy to fall into:

1. **A thin, stringly-typed model.** Pass `String`s everywhere and let each
   module interpret them. This makes invalid states representable, allows a
   `ServiceId` to be used where a `TaskId` is expected, and spreads validation
   across the codebase.
2. **A leaky model.** Let domain types absorb infrastructure details — `git2`
   types, PIDs, Tokio handles, shell strings — so the "domain" becomes a second
   infrastructure layer and cannot be tested or reasoned about in isolation.

The architecture (ADR 001) requires the domain to be the innermost layer with
no workspace dependencies. This ADR records how the domain model realizes that.

## Decision

Define a small, pure, capability-driven domain model in the
`service-orchestrator-domain` crate, with the following shape.

### Independence from infrastructure

The domain depends only on `serde`, `thiserror`, and `time`. It contains no Git
access, process spawning, Maven/npm/Liquibase execution, HTTP, scheduling, TUI,
or MCP. State that infrastructure produces (`GitState`, `ProcessState`) is
modelled as provider-independent enums.

### Capability-driven, not name-driven behaviour

A `Service` carries a `ServiceType` and an explicit `ServiceCapability` set
(`Build`, `Run`, `Migrate`, `ProvideArtifact`, `ServeHttp`). `ServiceType`
supplies defaults; configuration may extend them. Generic orchestration asks
what a service *can do*, never what it is *called*. This removes name-based
branching and makes new service categories pure configuration.

### Validated value objects and strong identifiers

Identifiers (`ServiceId`, `TaskId`, …), `Port`, `Version`, `Branch`,
`RelativePath`, and `AbsolutePath` validate themselves at construction. The
workspace aggregate additionally validates uniqueness, reference resolution, and
acyclicity.

### Task and workflow in the domain

`Task` and `Workflow` are domain concepts, not engine internals. The domain owns
the task state machine (`TaskStatus`, enforced by `Task::transition`) and the
*shape* of a workflow (unique steps, known prerequisites, acyclic). The engine
owns scheduling, concurrency, retry, and timeout.

### Permission as shared vocabulary

`Permission` (`Read`, `SafeWrite`, `Destructive`) is defined here so the TUI and
MCP cannot disagree about risk. The policy engine applies it later.

### Errors

A single small `DomainError` covers identifier validation, invariant violations,
missing/duplicate entities, illegal task transitions, dependency cycles, and
invalid commands or paths. It is not a global application error enum.

### Serialization

Configuration- and state-shaped domain types derive `Serialize`/`Deserialize`.
`serde` is compile-time-only and introduces no runtime dependency.

## Alternatives considered

1. **Stringly-typed identifiers and ad-hoc validation.**
   Rejected: invalid states remain representable and validation is duplicated in
   every consumer. Strong newtypes and validating constructors eliminate whole
   classes of bug at compile time.

2. **A parallel DTO hierarchy for serialization.**
   Rejected: for a local tool it is duplication without benefit. Domain values
   are plain data that must cross the config, TUI, and MCP boundaries; deriving
   `serde` directly keeps one definition. (If a boundary later needs a
   different wire shape, a DTO can be introduced then.)

3. **Model services as a trait (`trait Service`) rather than data.**
   Rejected: services are configuration data, not behaviour. A trait hierarchy
   would push executor concerns into the domain. Service-specific behaviour
   belongs behind the application's `ServiceExecutor` boundary (ADR 001).

4. **Put `Task`/`Workflow` in the `execution` crate.**
   Rejected: interfaces must report task and workflow state without depending on
   the engine, and the state machine is a business rule. Keeping the vocabulary
   in the domain lets `execution` depend on it rather than define it.

5. **Encode service ordering as a schedule in the domain.**
   Rejected: the domain validates that the dependency graph is acyclic; turning
   it into an execution order is a scheduling decision that belongs to the
   engine.

6. **A universal `Value`/map metadata bag on every entity.**
   Rejected: it trades type safety for speculative flexibility. `Other(String)`
   variants on the closed enums provide extensibility where it is actually
   needed.

## Reasoning

- **Testability.** With no infrastructure, the entire model is unit-testable
  without Git, Maven, Node, a terminal, or an MCP client.
- **Compile-time safety.** Distinct newtypes make category errors compile
  errors; crate boundaries keep infrastructure out.
- **Configuration-first.** Adding a service or a new service category is a
  configuration change, not a code change.
- **One source of truth.** Both interfaces consume the same `Permission` and
  task/workflow vocabulary, so they cannot drift.
- **Security by construction.** Structured commands, credential-free repository
  URLs, path traversal rejection, and secret redaction live in the model rather
  than in each caller.

## Consequences

### Positive

- A stable vocabulary that every later layer can build on.
- Invalid values are hard to construct; invariants are enforced once.
- Behaviour is configuration/capability driven, so the platform stays
  extensible.
- Interfaces report uniform, structured state.

### Negative / trade-offs

- More types and more `validate`/builder code than a stringly-typed model.
- `Workspace::validate` performs graph checks (uniqueness, references,
  acyclicity), which is more logic in the aggregate than a purely anemic model
  would have.
- Deriving `serde` on domain types couples them to a serialization library. This
  was judged acceptable because `serde` is compile-time-only and the values
  genuinely cross boundaries.
- `Other(String)` escape hatches can be overused; convention is to add a variant
  when a concept becomes first-class.

## Follow-up

- The `execution` crate builds the scheduler on top of `Task`/`Workflow`.
- The `policy` crate maps operations to `Permission` and enforces confirmation.
- The `config` crate parses TOML into a `Workspace` and calls `validate`.
- Add ADRs for the task engine, process management, Git integration,
  configuration, and MCP security as those decisions are made.

# ADR 016: End-to-End Workflow Orchestration & Environment Preparation

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

By the end of ADR 015 the project had one application core, one task engine, one
shared local daemon, and two interface adapters (the Ratatui TUI and the MCP
server). Every individual capability existed: the Git manager, the Build and
Library managers, the Service Runtime, the Migration Manager, and the Health
Manager each had an application use case and a provider-backed implementation.

What did not exist was the **end-to-end developer workflow** that ties them
together. Preparing a development environment — make repositories current,
prepare libraries, apply migrations, build, start, verify readiness — was a
sequence a developer (or agent) had to drive operation by operation. That had the
usual problems:

- the correct ordering was tribal knowledge (libraries before services,
  migrations before the build that needs the new schema, a service's dependencies
  before the service);
- different interfaces could compose the sequence differently, or race each
  other;
- a partial failure mid-sequence left the environment in an unclear state;
- there was no single, inspectable description of "what will this do?".

`tasks/016.md` made environment preparation the goal. It imposed hard
constraints:

- implement the workflow **once**, in the application core, and reuse it from TUI
  and MCP; do not add a second scheduler or a second core;
- a structured request with **documented, safe defaults** — no surprising
  database mutation or destructive Git;
- a **read-only planning** operation that describes the plan before execution;
- resolve the **dependency closure** and surface it (selected services vs
  automatically included dependencies vs libraries);
- preserve local changes and never hard-code `master`;
- enforce the **shared policy engine** for planning *and* execution, with no
  arbitrary `confirmed: true`;
- revalidate preconditions and behave predictably when repeated (idempotency);
- return **structured, aggregated** results, never unbounded logs;
- one shared workflow, observable by both interfaces through the daemon;
- explicit scope exclusions: no Docker, Kubernetes, remote orchestration,
  packaging, or release automation.

## Decision

Add an **environment workflow composition** to the application core, built on the
existing Task Engine.

### It belongs in the Application Core

Orchestration is an application concern, not an interface or infrastructure one.
The composition function lives beside the other use cases
(`crates/application/src/commands/workflow.rs`). The TUI and MCP call the same
use cases (`plan_prepare_environment`, `prepare_environment`,
`get_workflow_status`, `cancel_workflow`) through the shared `OrchestratorClient`.
There is exactly one place that decides which steps a preparation schedules, so
the interfaces cannot drift apart and the daemon owns the result.

### The existing Task Engine is reused

The workflow is a `WorkflowDefinition` submitted to the existing engine. The
engine already owns the parts that are hard and dangerous to duplicate:
dependency-ordered scheduling, bounded concurrency, cancellation, retry, timeout,
and events. The application layer only *composes* steps and their prerequisites;
it never runs a command. This keeps `docs/task-engine.md` the single description
of scheduling semantics.

Two engine behaviors were completed to make partial failures and cancellation
correct:

- **Transitive settling.** When a step fails or is skipped, its dependents settle
  transitively (skipped dependents propagate to *their* dependents). Previously a
  long dependency chain could leave a downstream step blocked forever.
- **Queued cancellation.** A cancellation request is honored before a queued step
  starts: it transitions `queued → cancelled` without occupying a slot, and its
  dependents settle as skipped.

### Dependency closure

Selection is explicit services, else profile, else group, else the whole
workspace. The closure then adds required service and library dependencies
(unless disabled). Services are ordered dependency-first with a Kahn topological
sort over a deterministic frontier, reusing the existing workspace dependency
graph. A cycle is reported as `ServiceDependencyCycle` rather than producing an
arbitrary order. The plan clearly separates *selected* services from
*automatically included* ones, and never silently starts an unrelated service.

### Partial failures

The engine's default `AllSucceeded` policy is exactly right for environment
preparation: a failed migration must block the dependent build and start; a
service's build failing must block its start. Failure is structural (a coded
`WorkflowTaskFailure`), and the workflow classifies into `succeeded`,
`partially_succeeded`, `failed`, or `cancelled` from the task counts.

### Planning is separated from execution

`plan_prepare_environment` takes the same request as `prepare_environment` and is
read-only. Crucially, **both are produced by the same composer**; the plan is not
a parallel re-implementation that can drift. The only difference is
`EnvironmentMode`: `Plan` requires only `Read` and reports
`requires_destructive_authorization`; `Execute` authorizes each migration target
as it is resolved. This gives a trustworthy preview without a second code path.

### Preconditions are revalidated

A plan is a snapshot and may be stale (Git moved, a port was taken, a service was
started, configuration changed). Execution therefore revalidates every
precondition — workspace, services, capabilities, commands, migration
enablement, database targets, and current process state — immediately before
scheduling. A plan is never treated as authorization: the migration target is
authorized again at execution time.

### TUI and MCP share workflows

Both are thin. The TUI builds the request, calls `plan_prepare_environment`,
renders the returned `EnvironmentPlan` for review, and on confirmation calls
`prepare_environment` through the trusted interactive authorization path. MCP
exposes the same four use cases as tools, validates strict schemas, and never
self-authorizes. Because both connect to the daemon, a workflow started in the
TUI is visible to `get_workflow_status` in MCP and vice versa.

## Alternatives considered

- **A new workflow engine / scheduler.** Rejected: it would duplicate the task
  engine's scheduling, concurrency, cancellation, and events — the exact thing
  `tasks/016.md` forbids — and create two sources of truth for task state.
- **Driving the sequence from the interface (TUI or MCP).** Rejected: logic would
  be duplicated across interfaces, could diverge, and would put business rules in
  adapters.
- **A separate plan type hand-written per interface.** Rejected: the plan would
  drift from execution. Computing both from one composer makes the plan
  authoritative.
- **Auto-stashing or forcing checkout to guarantee a fast-forward.** Rejected:
  discarding local changes silently is unacceptable (`AGENTS.md` §14). The
  workflow preserves local changes and fails the step instead.
- **A `confirmed: true` request flag for migrations.** Rejected explicitly by the
  task; authorization flows only through the shared policy engine.

## Consequences

Positive:

- One implementation of environment preparation, reused everywhere.
- An inspectable, accurate plan available before any mutation.
- Correct, safe handling of failures and cancellation through the existing
  engine.
- Safe defaults and enforced authorization; local changes are preserved.

Trade-offs and limitations:

- **Planning is synchronous and provider-agnostic.** It composes steps without
  running the providers, so library **version-alignment** inspection (which is
  asynchronous and artifact-based) is reported by the dedicated
  `inspect_library_version_alignment` query rather than embedded in the plan.
- **The workflow engine sees one workspace.** Cross-workspace orchestration is
  out of scope.
- **`ServiceDependencyCycle` is defensive.** Configuration validation already
  rejects cyclic workspaces, so the check guards against a future
  closed-world-assumption change rather than a reachable user input.
- The engine executes whatever is configured; the workflow does not implement new
  build or migration providers (explicitly out of scope).
